//! Running the event loop: awaiting, settling futures, switching tasks, and
//! deciding what happens when nothing can run.
//!
//! Everything here turns on one rule: a future is the only place a wait is
//! recorded. Awaiting registers a [`Waiter`] on it, settling walks that list
//! once, and every awaitable in the language reduces to that, whether it is a
//! coroutine, a task, a timer, a lock or an external call.

use std::{mem, task::Poll};

use monty_types::MontyException;
use smallvec::smallvec;

use super::{AwaitResult, CallFrame, CallResult, FrameExit, VM};
use crate::{
    args::ArgValues,
    asyncio::{
        CallId, Combinator, CombinatorKind, Coroutine, CoroutineState, Future, FutureKind, FutureState, ReturnWhen,
        TaskId, TaskRun, Waiter,
    },
    bytecode::vm::{
        generator::{ResumeMode, stop_async_iteration},
        scheduler::{SerializedTaskFrame, TaskBody, TaskState},
    },
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunError, RunResult, SimpleException},
    hash::identity_hash,
    heap::{ContainsHeap, DropWithContext, HeapData, HeapId, HeapRead, HeapReadOutput},
    intern::{FunctionId, StaticStrings},
    object_bridge::MontyObjectExt,
    run_progress::{ExtFunctionResult, ExtFunctionResultExt},
    types::{
        List, PyTrait, Set,
        asyncio::error_as_value_opt,
        instance::{class_defines, instance_class_id},
        tuple::allocate_tuple,
    },
    value::Value,
};

/// How a future settled, on its way from the settling site to every waiter.
#[derive(Debug)]
pub(crate) enum Outcome {
    /// A result, which every awaiting site receives a clone of.
    Value(Value),
    /// An exception, replayed at every awaiting site.
    Error(RunError),
    /// A cancellation. Kept apart from [`Self::Error`] only so `cancelled()`
    /// can answer; the error it carries is a `CancelledError`.
    Cancelled(RunError),
}

impl Outcome {
    /// The state a future settled with this outcome holds.
    fn into_state(self) -> FutureState {
        match self {
            Self::Value(value) => FutureState::Finished(value),
            Self::Error(error) => FutureState::Failed(error),
            Self::Cancelled(error) => FutureState::Cancelled(error),
        }
    }
}

impl<C: ContainsHeap> DropWithContext<C> for Outcome {
    fn drop_with(self, heap: &mut C) {
        match self {
            Self::Value(value) => value.drop_with(heap),
            Self::Error(_) | Self::Cancelled(_) => {}
        }
    }
}

impl<'h> VM<'h> {
    // ================================================================
    // Awaitables
    // ================================================================

    /// Whether `id` names something the `Await` opcode drives on its own, with
    /// no `__await__` call in between.
    ///
    /// The one list of them: `GetAwaitable` passes these through untouched,
    /// `GetYieldFromIter` does too, and `SendIter` ends its delegation on one.
    pub(super) fn is_native_awaitable(&self, id: HeapId) -> bool {
        match self.heap.get(id) {
            HeapData::Coroutine(_) | HeapData::Future(_) => true,
            HeapData::Generator(generator) => generator.is_async,
            _ => false,
        }
    }

    /// Executes the `GetAwaitable` opcode: turns TOS into the iterator the
    /// `await` loop drives.
    ///
    /// Natively awaitable objects are handed straight back. Anything else must
    /// define `__await__`, and calling it pushes a frame whose return value
    /// takes its place, the same shape CPython's `GET_AWAITABLE` has, where the
    /// slot call runs before the send loop starts.
    pub(super) fn exec_get_awaitable_iter(&mut self) -> RunResult<CallResult> {
        let obj = self.pop();
        let defines_await = match obj {
            Value::Ref(id) if self.is_native_awaitable(id) => return Ok(CallResult::Value(obj)),
            Value::Ref(id) => self.defines_await(id),
            _ => false,
        };
        if !defines_await {
            let name = obj.py_type_name(self);
            obj.drop_with(self);
            return Err(ExcType::object_not_awaitable(&name));
        }
        self.call_attr(obj, StaticStrings::DunderAwait.into(), ArgValues::Empty)
    }

    /// Executes the `GetYieldFromIter` opcode.
    ///
    /// Generators and awaitables are already what a delegation drives; only
    /// everything else goes through `iter()`. Outlined from the run loop, whose
    /// stack frame is paid for on every native re-entry.
    #[inline(never)]
    pub(super) fn exec_get_yield_from_iter(&mut self) -> RunResult<()> {
        let passthrough = matches!(*self.peek(), Value::Ref(id)
            if matches!(self.heap.get(id), HeapData::Generator(_)) || self.is_native_awaitable(id));
        if passthrough {
            return Ok(());
        }
        let value = self.pop();
        let iterator = value.py_iter(self);
        value.drop_with(self);
        self.push(iterator?);
        Ok(())
    }

    /// Takes the pending calls a dying task left for the run loop; see
    /// [`VM::deferred_resolve`]. Outlined for the same stack reason.
    #[cold]
    #[inline(never)]
    pub(super) fn take_deferred_resolve(&mut self) -> Vec<CallId> {
        self.deferred_resolve.take().unwrap_or_default()
    }

    /// Executes the `Await` opcode: waits on TOS.
    ///
    /// The wait either has its answer already (a settled future, a coroutine
    /// whose frame goes straight onto the stack) or parks this task on the
    /// future and lets the loop pick something else.
    pub(super) fn exec_get_awaitable(&mut self) -> Result<AwaitResult, RunError> {
        let this = self;
        let awaitable = this.pop();
        defer_drop!(awaitable, this);

        let Value::Ref(heap_id) = *awaitable else {
            return Err(ExcType::object_not_awaitable(&awaitable.py_type_name(this)));
        };

        // An async generator hands itself back from `__anext__`, so awaiting
        // one drives exactly one step of its body.
        if matches!(this.heap.get(heap_id), HeapData::Generator(generator) if generator.is_async) {
            return if this.generator_resume_op(heap_id, ResumeMode::Await, Value::None)? {
                Ok(AwaitResult::FramePushed)
            } else {
                Err(stop_async_iteration())
            };
        }

        match this.heap.get(heap_id) {
            HeapData::Coroutine(_) => {
                let HeapReadOutput::Coroutine(coro) = this.heap.read(heap_id) else {
                    unreachable!("matched Coroutine above")
                };
                this.await_coroutine(coro)
            }
            HeapData::Future(_) => {
                let task_id = this
                    .scheduler
                    .current_task_id()
                    .expect("exec_get_awaitable called without a current task");
                match this.poll_future(heap_id, Waiter::Task(task_id))? {
                    Poll::Ready(value) => Ok(AwaitResult::ValueReady(value)),
                    Poll::Pending => {
                        // A cancellation asked for while this task was running
                        // lands here, at the first point the task actually
                        // gives up control, as it does in CPython.
                        if let Some(error) = this.take_pending_cancellation(task_id) {
                            this.unregister_waiter(heap_id, task_id);
                            return Err(error);
                        }
                        this.scheduler.block_current_on(heap_id, this.heap);
                        this.switch_or_yield()
                    }
                }
            }
            _ => Err(ExcType::object_not_awaitable(&awaitable.py_type_name(this))),
        }
    }

    /// Awaits a coroutine by pushing a frame to execute it.
    ///
    /// A bare `await coro` runs the coroutine in the awaiting task rather than
    /// spawning one, which is why it needs no future of its own.
    fn await_coroutine(&mut self, mut coro: HeapRead<'h, Coroutine>) -> Result<AwaitResult, RunError> {
        if coro.get(self.heap).state != CoroutineState::New {
            return Err(ExcType::cannot_reuse_already_awaited_coroutine());
        }
        let func_id = coro.get(self.heap).func_id;
        let namespace_values: Vec<Value> = coro
            .get(self.heap)
            .namespace
            .iter()
            .map(|v| v.clone_with_heap(self))
            .collect();
        coro.get_mut(self.heap).state = CoroutineState::Running;
        drop(coro);
        self.start_coroutine_frame(func_id, namespace_values)?;
        Ok(AwaitResult::FramePushed)
    }

    /// Registers `waiter` on the future at `fut_id`, or answers at once when it
    /// has already settled.
    ///
    /// A settled future replays its outcome to every later wait, as CPython's
    /// caches its result; nothing here is single-shot.
    pub(crate) fn poll_future(&mut self, fut_id: HeapId, waiter: Waiter) -> RunResult<Poll<Value>> {
        let HeapReadOutput::Future(mut fut) = self.heap.read(fut_id) else {
            panic!("poll_future called with a non-future heap id")
        };
        match &fut.get(self.heap).state {
            FutureState::Finished(value) => {
                let value = value.clone_with_heap(self.heap);
                drop(fut);
                waiter.drop_with(self);
                Ok(Poll::Ready(value))
            }
            FutureState::Failed(error) | FutureState::Cancelled(error) => {
                let error = error.clone();
                drop(fut);
                waiter.drop_with(self);
                Err(error)
            }
            FutureState::Pending => {
                fut.get_mut(self.heap).waiters.push(waiter);
                Ok(Poll::Pending)
            }
        }
    }

    /// Drops the `Waiter::Task(task_id)` entry the caller had just registered.
    fn unregister_waiter(&mut self, fut_id: HeapId, task_id: TaskId) {
        let HeapReadOutput::Future(mut fut) = self.heap.read(fut_id) else {
            return;
        };
        fut.get_mut(self.heap)
            .waiters
            .retain(|waiter| !matches!(waiter, Waiter::Task(id) if *id == task_id));
    }

    // ================================================================
    // Future allocation and settling
    // ================================================================

    /// Allocates a pending future and returns its heap id, owned by the caller.
    pub(crate) fn alloc_future(&mut self, kind: FutureKind, call_id: Option<CallId>) -> HeapId {
        self.heap
            .allocate(HeapData::Future(Box::new(Future::new(kind, call_id))))
    }

    /// A future that has already settled, which is what an `asyncio` primitive
    /// hands back when it has nothing to wait for.
    pub(crate) fn settled_future(&mut self, outcome: Outcome) -> Value {
        let id = self.alloc_future(FutureKind::Future, None);
        let HeapReadOutput::Future(mut fut) = self.heap.read(id) else {
            unreachable!("just allocated a future")
        };
        fut.get_mut(self.heap).state = outcome.into_state();
        drop(fut);
        Value::Ref(id)
    }

    /// Settles the future at `fut_id` and tells everyone waiting on it.
    ///
    /// A second settling is dropped rather than raised on: the callers that
    /// must reject one (`set_result`, `cancel`) check first, while the loop's
    /// own paths can legitimately race a host answer against a cancellation.
    pub(crate) fn settle_future(&mut self, fut_id: HeapId, outcome: Outcome) {
        let HeapReadOutput::Future(mut fut) = self.heap.read(fut_id) else {
            panic!("settle_future called with a non-future heap id")
        };
        if fut.get(self.heap).state.is_settled() {
            drop(fut);
            outcome.drop_with(self);
            return;
        }
        let state = fut.get_mut(self.heap);
        state.state = outcome.into_state();
        let waiters = mem::take(&mut state.waiters);
        let callbacks = mem::take(&mut state.callbacks);
        drop(fut);

        for waiter in waiters {
            let outcome = self.settled_outcome(fut_id);
            match waiter {
                Waiter::Task(task_id) => self.deliver_to_task(task_id, outcome),
                Waiter::Slot { owner, index } => {
                    self.combinator_child_settled(owner, index, fut_id, outcome);
                    self.heap.dec_ref(owner);
                }
            }
        }

        for callback in callbacks {
            self.heap.inc_ref(fut_id);
            self.scheduler.spawn(TaskBody::Callback {
                func: callback,
                arg: Value::Ref(fut_id),
            });
        }
    }

    /// A clone of the outcome a settled future is holding.
    fn settled_outcome(&mut self, fut_id: HeapId) -> Outcome {
        let HeapReadOutput::Future(fut) = self.heap.read(fut_id) else {
            panic!("settled_outcome called with a non-future heap id")
        };
        let outcome = match &fut.get(self.heap).state {
            FutureState::Pending => panic!("settled_outcome called on a pending future"),
            FutureState::Finished(value) => Outcome::Value(value.clone_with_heap(self.heap)),
            FutureState::Failed(error) => Outcome::Error(error.clone()),
            FutureState::Cancelled(error) => Outcome::Cancelled(error.clone()),
        };
        drop(fut);
        outcome
    }

    /// Hands an outcome to a parked task: the value onto its operand stack, or
    /// the error into the state that raises it when the task is switched in.
    fn deliver_to_task(&mut self, task_id: TaskId, outcome: Outcome) {
        if !self.scheduler.has_task(task_id) {
            outcome.drop_with(self);
            return;
        }
        match outcome {
            Outcome::Value(value) => {
                let is_current = self.scheduler.current_task_id() == Some(task_id) && !self.frames.is_empty();
                if is_current {
                    self.stack.push(value);
                } else {
                    self.scheduler.get_task_mut(task_id).stack.push(value);
                }
                self.scheduler.make_ready(task_id, self.heap);
            }
            Outcome::Error(error) | Outcome::Cancelled(error) => {
                self.scheduler.remove_from_ready_queue(task_id);
                self.scheduler.set_state(task_id, TaskState::Failed(error), self.heap);
                self.scheduler.push_ready(task_id);
            }
        }
    }

    // ================================================================
    // Tasks
    // ================================================================

    /// Wraps `coroutine` in an `asyncio.Task` and queues it to run.
    ///
    /// `coroutine` stays the caller's; the task takes its own reference.
    pub(crate) fn spawn_task(&mut self, coroutine: HeapId) -> RunResult<Value> {
        let HeapReadOutput::Coroutine(coro) = self.heap.read(coroutine) else {
            panic!("spawn_task called with a non-coroutine heap id")
        };
        let startable = coro.get(self.heap).state == CoroutineState::New;
        drop(coro);
        if !startable {
            return Err(ExcType::cannot_reuse_already_awaited_coroutine());
        }

        let future = self.alloc_future(FutureKind::Task, None);
        // Three owners to arrange: the scheduler task holds the coroutine and
        // the future, the future's own `run` holds the coroutine again (that is
        // what `get_coro()` hands back after the task is gone), and the value
        // returned here holds the future.
        self.heap.inc_ref(coroutine);
        self.heap.inc_ref(coroutine);
        self.heap.inc_ref(future);
        let task_id = self.scheduler.spawn(TaskBody::Coroutine { coroutine, future });
        let HeapReadOutput::Future(mut fut) = self.heap.read(future) else {
            unreachable!("just allocated a future")
        };
        fut.get_mut(self.heap).run = Some(TaskRun {
            coroutine: Some(coroutine),
            task_id,
        });
        drop(fut);
        Ok(Value::Ref(future))
    }

    /// Cancels the future at `fut_id`, returning whether the request was
    /// accepted, which is what `Future.cancel()` reports.
    ///
    /// A plain future settles as cancelled at once. A task is cancelled at its
    /// suspension point: if it is parked, the future it waits on is cancelled
    /// and the error travels back through it, exactly as CPython cancels
    /// `_fut_waiter`; if it is running, the request is remembered and fires at
    /// its next `await`.
    pub(crate) fn cancel_future(&mut self, fut_id: HeapId, message: Value) -> bool {
        let HeapReadOutput::Future(mut fut) = self.heap.read(fut_id) else {
            panic!("cancel_future called with a non-future heap id")
        };
        if fut.get(self.heap).state.is_settled() {
            drop(fut);
            message.drop_with(self);
            return false;
        }
        let state = fut.get_mut(self.heap);
        state.cancelling += 1;
        let task_id = state.run.as_ref().map(|run| run.task_id);
        drop(fut);

        let Some(task_id) = task_id else {
            let error = cancelled_error(&message, self);
            message.drop_with(self);
            self.settle_future(fut_id, Outcome::Cancelled(error));
            return true;
        };

        let parked_on = match self.scheduler.try_task(task_id).map(|task| &task.state) {
            Some(TaskState::Blocked(on)) => Some(*on),
            Some(_) | None => None,
        };
        if let Some(on) = parked_on {
            // Cancelling what the task waits on is what wakes it: the
            // `CancelledError` arrives by the path a failure would have taken.
            message.drop_with(self);
            self.cancel_future(on, Value::None);
            return true;
        }

        let running = self.scheduler.current_task_id() == Some(task_id);
        if running && self.scheduler.has_task(task_id) {
            let HeapReadOutput::Future(mut fut) = self.heap.read(fut_id) else {
                unreachable!("checked above")
            };
            fut.get_mut(self.heap).must_cancel = Some(message);
            drop(fut);
            return true;
        }

        let error = cancelled_error(&message, self);
        message.drop_with(self);
        if self.scheduler.has_task(task_id) {
            // Queued but never started: it ends without running a line, as
            // throwing into a fresh coroutine does in CPython.
            self.deliver_to_task(task_id, Outcome::Cancelled(error));
        } else {
            self.settle_future(fut_id, Outcome::Cancelled(error));
        }
        true
    }

    /// Takes the cancellation `task_id` was asked to honour at its next
    /// suspension, if any.
    fn take_pending_cancellation(&mut self, task_id: TaskId) -> Option<RunError> {
        let fut_id = self.scheduler.try_task(task_id)?.future()?;
        let HeapReadOutput::Future(mut fut) = self.heap.read(fut_id) else {
            return None;
        };
        let message = fut.get_mut(self.heap).must_cancel.take();
        drop(fut);
        let message = message?;
        let error = cancelled_error(&message, self);
        message.drop_with(self);
        Some(error)
    }

    /// Handles a spawned task's coroutine returning.
    pub(super) fn handle_task_completion(&mut self, result: Value) -> Result<AwaitResult, RunError> {
        let task_id = self
            .scheduler
            .current_task_id()
            .expect("handle_task_completion called without current task");
        self.finish_current_task(task_id, Outcome::Value(result));
        self.run_next()
    }

    /// Handles an exception escaping every frame of a spawned task.
    ///
    /// The error belongs to the task's future from here on, so nothing
    /// propagates to whoever happened to be running: awaiting sites see it, and
    /// a task nobody awaits keeps it.
    pub(super) fn handle_task_failure(&mut self, error: RunError) -> Result<(), RunError> {
        let task_id = self
            .scheduler
            .current_task_id()
            .expect("handle_task_failure called without current task");
        debug_assert!(!task_id.is_main(), "handle_task_failure called for main task");
        let outcome = if is_cancellation(&error) {
            Outcome::Cancelled(error)
        } else {
            Outcome::Error(error)
        };
        self.finish_current_task(task_id, outcome);
        match self.run_next()? {
            AwaitResult::FramePushed => Ok(()),
            AwaitResult::Yield(pending) => {
                self.deferred_resolve = Some(pending);
                Ok(())
            }
            AwaitResult::ValueReady(value) => {
                value.drop_with(self);
                unreachable!("run_next never resolves a value")
            }
        }
    }

    /// Tears the current task down and reports `outcome` to whatever it ran for.
    fn finish_current_task(&mut self, task_id: TaskId, outcome: Outcome) {
        let body_future = self.scheduler.get_task(task_id).future();
        if let Some(coroutine) = self.task_coroutine(task_id) {
            let HeapReadOutput::Coroutine(mut coro) = self.heap.read(coroutine) else {
                panic!("task coroutine id is not a coroutine")
            };
            coro.get_mut(self.heap).state = CoroutineState::Completed;
            drop(coro);
        }
        self.cleanup_current_task();
        if let Some(future) = body_future {
            // Hold the future across the settle: dropping the task releases the
            // reference the task held.
            self.heap.inc_ref(future);
            self.scheduler.drop_task(task_id, self.heap);
            self.settle_future(future, outcome);
            self.heap.dec_ref(future);
        } else {
            // A done-callback: what it returned is nobody's.
            outcome.drop_with(self);
            self.scheduler.drop_task(task_id, self.heap);
        }
    }

    /// The coroutine a task drives, if it drives one.
    fn task_coroutine(&self, task_id: TaskId) -> Option<HeapId> {
        match &self.scheduler.try_task(task_id)?.body {
            TaskBody::Coroutine { coroutine, .. } => Some(*coroutine),
            TaskBody::Main { .. } | TaskBody::Callback { .. } => None,
        }
    }

    /// Returns true if the current task is a spawned task (not main).
    #[inline]
    pub(super) fn is_spawned_task(&self) -> bool {
        self.scheduler.current_task_id().is_some_and(|id| !id.is_main())
    }

    // ================================================================
    // The loop
    // ================================================================

    /// Parks the current task and finds something else to run.
    ///
    /// When nothing else can run, the current task's frames stay in the VM: a
    /// suspension to the host has to snapshot them where they are.
    fn switch_or_yield(&mut self) -> Result<AwaitResult, RunError> {
        let parked = self.scheduler.current_task_id();
        loop {
            if self.scheduler.ready_is_empty() {
                if self.fire_due_timers() {
                    continue;
                }
                if self.scheduler.has_pending_externals() {
                    return Ok(AwaitResult::Yield(self.get_pending_call_ids()));
                }
                if self.scheduler.advance_to_next_deadline() {
                    continue;
                }
                return Err(nothing_left_to_wait_for());
            }
            self.save_current_context();
            if self.load_ready_task()? {
                return Ok(AwaitResult::FramePushed);
            }
            // Everything queued finished without a frame of its own. Put the
            // parked task back exactly as it was before deciding again.
            if let Some(parked) = parked {
                self.scheduler.set_current_task(Some(parked));
                self.restore_task_context(parked);
            }
        }
    }

    /// Finds something to run when the VM holds no frames, because the task
    /// that held them has just finished.
    fn run_next(&mut self) -> Result<AwaitResult, RunError> {
        loop {
            if self.load_ready_task()? {
                return Ok(AwaitResult::FramePushed);
            }
            if self.fire_due_timers() {
                continue;
            }
            if self.scheduler.has_pending_externals() {
                return Ok(AwaitResult::Yield(self.get_pending_call_ids()));
            }
            if self.scheduler.advance_to_next_deadline() {
                continue;
            }
            return Err(nothing_left_to_wait_for());
        }
    }

    /// Loads ready tasks until one of them has frames to run.
    ///
    /// Returns `false` when the queue drained without loading anything, which a
    /// done-callback that returned at once, or a task cancelled before it
    /// started, both produce.
    fn load_ready_task(&mut self) -> RunResult<bool> {
        while let Some(next) = self.scheduler.next_ready_task() {
            self.scheduler.set_current_task(Some(next));
            if self.load_or_init_task(next)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Settles every timer whose deadline the clock has reached.
    ///
    /// Returns whether any fired, since firing can make tasks ready.
    fn fire_due_timers(&mut self) -> bool {
        let due = self.scheduler.take_due_timers();
        let fired = !due.is_empty();
        for timer in due {
            self.settle_future(timer.future, Outcome::Value(timer.value));
            self.heap.dec_ref(timer.future);
            if let Some(target) = timer.cancel_target {
                self.cancel_future(target, Value::None);
                self.heap.dec_ref(target);
            }
        }
        fired
    }

    /// Arms a timer to settle a fresh future `delay_nanos` from now.
    ///
    /// The future is what `asyncio.sleep` hands back and what
    /// `asyncio.timeout` reads to tell whether its deadline passed; the
    /// optional `cancel_target` is the task the deadline interrupts.
    pub(crate) fn arm_timer(&mut self, delay_nanos: u64, value: Value, cancel_target: Option<HeapId>) -> HeapId {
        let future = self.alloc_future(FutureKind::Future, None);
        // One reference for the timer, one for the caller.
        self.heap.inc_ref(future);
        if let Some(target) = cancel_target {
            self.heap.inc_ref(target);
        }
        self.scheduler.arm_timer(delay_nanos, future, value, cancel_target);
        future
    }

    /// Disarms every timer aimed at `future`, releasing what they held.
    pub(crate) fn disarm_timers(&mut self, future: HeapId) {
        for timer in self.scheduler.disarm_timers(future) {
            timer.drop_with(self.heap);
        }
    }

    /// Queues `callback` to be called with the already-settled future at
    /// `fut_id`, which is what `add_done_callback` owes a late caller.
    pub(crate) fn schedule_callback(&mut self, fut_id: HeapId, callback: Value) {
        self.heap.inc_ref(fut_id);
        self.scheduler.spawn(TaskBody::Callback {
            func: callback,
            arg: Value::Ref(fut_id),
        });
    }

    /// Whether the class of the instance at `id` defines `__await__`.
    fn defines_await(&self, id: HeapId) -> bool {
        instance_class_id(id, self).is_some_and(|class_id| class_defines(class_id, "__await__", self))
    }

    /// The exception object standing for `error`, or `None` when it has no
    /// Python form (a resource limit, which must keep propagating).
    fn error_as_value(&mut self, error: &RunError) -> Option<Value> {
        error_as_value_opt(error, self)
    }

    /// A `set` of futures, whose hashes are their identities and so cannot
    /// collide or raise.
    fn future_set(&mut self, values: Vec<Value>) -> Value {
        let entries = values
            .into_iter()
            .map(|value| {
                let hash = match value {
                    Value::Ref(id) => identity_hash(id).raw(),
                    _ => unreachable!("a wait set holds only futures"),
                };
                (value, hash)
            })
            .collect();
        Value::Ref(self.heap.allocate(HeapData::Set(Set::from_entries(entries))))
    }

    /// Saves the current task's context before switching tasks.
    fn save_current_context(&mut self) {
        if let Some(current_task_id) = self.scheduler.current_task_id() {
            self.save_task_context(current_task_id);
        }
    }

    /// Moves the VM's execution context into `task_id`.
    fn save_task_context(&mut self, task_id: TaskId) {
        let frames: Vec<SerializedTaskFrame> = self
            .frames
            .drain(..)
            .map(|f| SerializedTaskFrame {
                function_id: f.function_id,
                ip: f.ip,
                stack_base: f.stack_base,
                locals_count: f.locals_count,
                exception_stack_base: f.exception_stack_base,
                call_offset: f.call_offset,
                is_initializer: f.is_initializer,
            })
            .collect();

        // Each task gets its own recursion budget, so hand this one's
        // contribution back while it is parked.
        let task_depth = frames.len().saturating_sub(1);
        self.recursion_depth -= task_depth;

        let task = self.scheduler.get_task_mut(task_id);
        task.frames = frames;
        task.stack = mem::take(&mut self.stack);
        task.exception_stack = mem::take(&mut self.exception_stack);
        task.gen_activations = mem::take(&mut self.gen_activations);
        task.instruction_ip = self.instruction_ip;
    }

    /// Moves `task_id`'s saved execution context back into the VM.
    fn restore_task_context(&mut self, task_id: TaskId) {
        let task = self.scheduler.get_task_mut(task_id);
        let frames = mem::take(&mut task.frames);
        self.stack = mem::take(&mut task.stack);
        self.exception_stack = mem::take(&mut task.exception_stack);
        self.gen_activations = mem::take(&mut task.gen_activations);
        self.instruction_ip = task.instruction_ip;
        self.recursion_depth += frames.len().saturating_sub(1);

        self.frames = frames
            .into_iter()
            .map(|sf| {
                let code = match sf.function_id {
                    Some(func_id) => &self.interns.get_function(func_id).code,
                    None => self.module_code.expect("module_code not set for main task frame"),
                };
                CallFrame {
                    code,
                    ip: sf.ip,
                    stack_base: sf.stack_base,
                    locals_count: sf.locals_count,
                    exception_stack_base: sf.exception_stack_base,
                    function_id: sf.function_id,
                    call_offset: sf.call_offset,
                    should_return: false,
                    is_initializer: sf.is_initializer,
                }
            })
            .collect();
    }

    /// Brings `task_id` into the VM, starting it if it has not run yet.
    ///
    /// Returns `false` when the task ended without needing a frame. An `Err`
    /// carries the exception the task must raise, and is only returned once its
    /// frames are in place so the unwinder finds them.
    fn load_or_init_task(&mut self, task_id: TaskId) -> RunResult<bool> {
        let started = !self.scheduler.get_task(task_id).frames.is_empty();
        let failure = match &self.scheduler.get_task(task_id).state {
            TaskState::Failed(_) => {
                let TaskState::Failed(error) =
                    mem::replace(&mut self.scheduler.get_task_mut(task_id).state, TaskState::Ready)
                else {
                    unreachable!("matched above")
                };
                Some(error)
            }
            TaskState::Ready | TaskState::Blocked(_) | TaskState::Completed(_) => None,
        };

        if started {
            self.restore_task_context(task_id);
            return match failure {
                Some(error) => Err(error),
                None => Ok(true),
            };
        }

        // Nothing has run yet, so an error is the task's whole outcome.
        if let Some(error) = failure {
            let outcome = if is_cancellation(&error) {
                Outcome::Cancelled(error)
            } else {
                Outcome::Error(error)
            };
            self.finish_current_task(task_id, outcome);
            return Ok(false);
        }

        let body = match &self.scheduler.get_task(task_id).body {
            TaskBody::Main { .. } => panic!("the main task must always hold its own frames"),
            TaskBody::Coroutine { coroutine, .. } => Ok(*coroutine),
            TaskBody::Callback { func, arg } => Err((func.clone_with_heap(self.heap), arg.clone_with_heap(self.heap))),
        };
        match body {
            Ok(coroutine) => {
                let HeapReadOutput::Coroutine(coro) = self.heap.read(coroutine) else {
                    panic!("task coroutine id is not a coroutine")
                };
                let startable = coro.get(self.heap).state == CoroutineState::New;
                drop(coro);
                if !startable {
                    let error = ExcType::cannot_reuse_already_awaited_coroutine();
                    self.finish_current_task(task_id, Outcome::Error(error));
                    return Ok(false);
                }
                self.init_task_from_coroutine(coroutine)?;
                Ok(true)
            }
            Err((func, arg)) => match self.call_function(&func, ArgValues::One(arg))? {
                CallResult::FramePushed => Ok(true),
                other => {
                    // A callback that needed no frame is already done; nothing
                    // reads what it returned.
                    other.drop_with(self);
                    self.finish_current_task(task_id, Outcome::Value(Value::None));
                    Ok(false)
                }
            },
        }
    }

    /// Starts a spawned task's coroutine as the root frame of its own stack.
    fn init_task_from_coroutine(&mut self, coroutine_id: HeapId) -> Result<(), RunError> {
        let HeapReadOutput::Coroutine(mut coro) = self.heap.read(coroutine_id) else {
            panic!("task coroutine id is not a coroutine")
        };
        let func_id = coro.get(self.heap).func_id;
        let namespace_values: Vec<Value> = coro
            .get(self.heap)
            .namespace
            .iter()
            .map(|v| v.clone_with_heap(self))
            .collect();
        coro.get_mut(self.heap).state = CoroutineState::Running;
        drop(coro);

        let func = self.interns.get_function(func_id);
        let locals_count = u16::try_from(namespace_values.len()).expect("coroutine namespace size exceeds u16");
        let stack_base = self.stack.len();
        self.stack.extend(namespace_values);
        let exc_stack_base = self.exception_stack.len();
        self.push_frame(CallFrame::new_function(
            &func.code,
            stack_base,
            locals_count,
            exc_stack_base,
            func_id,
            // No call site: the coroutine is this task's root frame.
            None,
        ))?;
        Ok(())
    }

    /// Starts a coroutine as a frame above the awaiting one.
    fn start_coroutine_frame(&mut self, func_id: FunctionId, namespace_values: Vec<Value>) -> Result<(), RunError> {
        let call_offset = self.current_offset();
        let func = self.interns.get_function(func_id);
        let locals_count = u16::try_from(namespace_values.len()).expect("coroutine namespace size exceeds u16");
        let stack_base = self.stack.len();
        self.stack.extend(namespace_values);
        let exc_stack_base = self.exception_stack.len();
        self.push_frame(CallFrame::new_function(
            &func.code,
            stack_base,
            locals_count,
            exc_stack_base,
            func_id,
            call_offset,
        ))?;
        Ok(())
    }

    // ================================================================
    // Combinators
    // ================================================================

    /// Allocates a combinator over `children` and registers it on each of them.
    ///
    /// `children` must already be futures, and the caller transfers one
    /// reference per entry. The returned value is the combinator's output
    /// future, which is what callers await.
    pub(crate) fn start_combinator(&mut self, kind: CombinatorKind, children: Vec<HeapId>) -> HeapId {
        let combinator = self.build_combinator(kind, children);
        let output = self.combinator_output(combinator);
        self.heap.dec_ref(combinator);
        output
    }

    /// `asyncio.as_completed(fs)`: the combinator itself, which is the async
    /// iterator handing each child back as it settles.
    pub(crate) fn start_as_completed(&mut self, children: Vec<HeapId>) -> HeapId {
        self.build_combinator(CombinatorKind::AsCompleted, children)
    }

    /// The shared half of [`Self::start_combinator`]: allocates the combinator,
    /// registers it on every child, and reports the ones already settled.
    fn build_combinator(&mut self, kind: CombinatorKind, children: Vec<HeapId>) -> HeapId {
        let output = self.alloc_future(FutureKind::Future, None);
        let watched = children.clone();
        let combinator = self
            .heap
            .allocate(HeapData::Combinator(Box::new(Combinator::new(kind, children, output))));

        // Register on every child before reporting any of them, so a child that
        // has already settled cannot finish the combinator while later slots
        // are still unregistered.
        let mut already: Vec<usize> = Vec::new();
        for (index, child) in watched.iter().enumerate() {
            let HeapReadOutput::Future(mut fut) = self.heap.read(*child) else {
                panic!("combinator child is not a future")
            };
            if fut.get(self.heap).state.is_settled() {
                drop(fut);
                already.push(index);
            } else {
                self.heap.inc_ref(combinator);
                fut.get_mut(self.heap).waiters.push(Waiter::Slot {
                    owner: combinator,
                    index: u32::try_from(index).expect("combinator child count exceeds u32"),
                });
                drop(fut);
            }
        }
        for index in already {
            let child = watched[index];
            let outcome = self.settled_outcome(child);
            let index = u32::try_from(index).expect("combinator child count exceeds u32");
            self.combinator_child_settled(combinator, index, child, outcome);
        }
        // An empty combinator has nothing to wait for.
        self.combinator_check_done(combinator);
        combinator
    }

    /// The `asyncio.Task` object of the running task, made on demand for
    /// module-level code so `asyncio.timeout` and `current_task()` have
    /// something to name there, as they do under CPython's `asyncio.run`.
    pub(crate) fn current_task_future(&mut self) -> Value {
        match self.current_task_future_id() {
            Some(id) => {
                self.heap.inc_ref(id);
                Value::Ref(id)
            }
            None => Value::None,
        }
    }

    /// The heap id of the running task's `asyncio.Task`, borrowed, creating the
    /// main task's on first ask.
    pub(crate) fn current_task_future_id(&mut self) -> Option<HeapId> {
        let task_id = self.scheduler.current_task_id()?;
        if let Some(existing) = self.scheduler.try_task(task_id)?.future() {
            return Some(existing);
        }
        if !matches!(self.scheduler.try_task(task_id)?.body, TaskBody::Main { .. }) {
            return None;
        }
        let future = self.alloc_future(FutureKind::Task, None);
        let HeapReadOutput::Future(mut fut) = self.heap.read(future) else {
            unreachable!("just allocated a future")
        };
        fut.get_mut(self.heap).run = Some(TaskRun {
            coroutine: None,
            task_id,
        });
        drop(fut);
        self.scheduler.get_task_mut(task_id).set_main_future(future);
        Some(future)
    }

    /// The scheduler's clock, in nanoseconds.
    pub(crate) fn scheduler_now(&self) -> u64 {
        self.scheduler.now()
    }

    /// One `as_completed` step: the next child to have settled, a future that
    /// settles with the next one, or the end of the iteration.
    pub(crate) fn as_completed_next(&mut self, owner: HeapId) -> RunResult<Value> {
        let HeapReadOutput::Combinator(mut comb) = self.heap.read(owner) else {
            panic!("as_completed_next called off a combinator")
        };
        let state = comb.get_mut(self.heap);
        let ready = if state.finished.is_empty() {
            None
        } else {
            Some(state.finished.remove(0))
        };
        let exhausted = ready.is_none() && state.pending == 0;
        drop(comb);
        if let Some(child) = ready {
            return Ok(self.settled_future(Outcome::Value(Value::Ref(child))));
        }
        if exhausted {
            return Err(stop_async_iteration());
        }
        let future = self.alloc_future(FutureKind::Future, None);
        self.heap.inc_ref(future);
        let HeapReadOutput::Combinator(mut comb) = self.heap.read(owner) else {
            unreachable!("checked above")
        };
        let previous = comb.get_mut(self.heap).next_wait.replace(future);
        drop(comb);
        if let Some(previous) = previous {
            self.heap.dec_ref(previous);
        }
        Ok(Value::Ref(future))
    }

    /// Records one child's outcome on its combinator and settles the output if
    /// the combinator's rule is now satisfied.
    fn combinator_child_settled(&mut self, owner: HeapId, index: u32, child: HeapId, outcome: Outcome) {
        let HeapReadOutput::Combinator(mut comb) = self.heap.read(owner) else {
            panic!("combinator waiter owner is not a combinator")
        };
        let kind = comb.get(self.heap).kind;
        let slot = index as usize;
        if comb.get(self.heap).results[slot].is_some() {
            // Already reported: a child listed twice fills its slot once.
            drop(comb);
            outcome.drop_with(self);
            return;
        }
        let state = comb.get_mut(self.heap);
        state.pending = state.pending.saturating_sub(1);
        state.finished.push(child);
        drop(comb);
        self.heap.inc_ref(child);

        match kind {
            CombinatorKind::Gather { return_exceptions } => {
                self.gather_child_settled(owner, slot, return_exceptions, outcome);
            }
            CombinatorKind::Wait { .. } | CombinatorKind::AsCompleted | CombinatorKind::TaskGroup => {
                if matches!(kind, CombinatorKind::TaskGroup)
                    && let Outcome::Error(error) = &outcome
                {
                    let HeapReadOutput::Combinator(mut comb) = self.heap.read(owner) else {
                        unreachable!("checked above")
                    };
                    let first = comb.get(self.heap).error.is_none();
                    if first {
                        comb.get_mut(self.heap).error = Some(error.clone());
                    }
                    drop(comb);
                    if first {
                        self.taskgroup_cancel_children(owner);
                    }
                }
                // These report the children themselves, not their values.
                self.heap.inc_ref(child);
                let HeapReadOutput::Combinator(mut comb) = self.heap.read(owner) else {
                    unreachable!("checked above")
                };
                comb.get_mut(self.heap).results[slot] = Some(Value::Ref(child));
                drop(comb);
                outcome.drop_with(self);
                self.as_completed_hand_out(owner);
            }
        }
        self.combinator_check_done(owner);
    }

    /// `gather`'s rule: fill the slot, or, without `return_exceptions`, settle
    /// the whole thing with the first exception and leave the siblings running,
    /// as CPython does.
    fn gather_child_settled(&mut self, owner: HeapId, slot: usize, return_exceptions: bool, outcome: Outcome) {
        let value = match outcome {
            Outcome::Value(value) => value,
            Outcome::Error(error) | Outcome::Cancelled(error) => {
                let carried = if return_exceptions {
                    self.error_as_value(&error)
                } else {
                    None
                };
                let Some(value) = carried else {
                    let output = self.combinator_output(owner);
                    self.settle_future(output, Outcome::Error(error));
                    self.heap.dec_ref(output);
                    return;
                };
                value
            }
        };
        let HeapReadOutput::Combinator(mut comb) = self.heap.read(owner) else {
            unreachable!("gather_child_settled called off a combinator")
        };
        comb.get_mut(self.heap).results[slot] = Some(value);
        drop(comb);
    }

    /// Settles the combinator's output once its rule says the wait is over.
    fn combinator_check_done(&mut self, owner: HeapId) {
        // A settled output means the answer is already out, whichever rule
        // produced it, so the watch is over even when children are still
        // running. `gather` reaches here that way: its first failure settles
        // the output while its siblings carry on by design.
        if self.combinator_output_settled(owner) {
            self.combinator_release_slots(owner);
            return;
        }
        let (kind, done) = {
            let HeapReadOutput::Combinator(comb) = self.heap.read(owner) else {
                panic!("combinator_check_done called off a combinator")
            };
            let state = comb.get(self.heap);
            let kind = state.kind;
            let pending = state.pending;
            let done = match kind {
                CombinatorKind::Gather { .. } | CombinatorKind::TaskGroup | CombinatorKind::AsCompleted => pending == 0,
                CombinatorKind::Wait { return_when } => match return_when {
                    ReturnWhen::AllCompleted => pending == 0,
                    ReturnWhen::FirstCompleted => pending < state.children.len(),
                    ReturnWhen::FirstException => {
                        pending == 0
                            || state.finished.iter().any(|id| {
                                matches!(self.heap.get(*id), HeapData::Future(f) if matches!(f.state, FutureState::Failed(_)))
                            })
                    }
                },
            };
            drop(comb);
            (kind, done)
        };
        if !done {
            return;
        }
        let output = self.combinator_output(owner);
        let outcome = match kind {
            CombinatorKind::Gather { .. } => {
                let results = self.combinator_take_results(owner);
                let list = self.heap.allocate(HeapData::List(List::new(results)));
                Outcome::Value(Value::Ref(list))
            }
            CombinatorKind::AsCompleted => Outcome::Value(Value::None),
            CombinatorKind::Wait { .. } => Outcome::Value(self.wait_result(owner)),
            CombinatorKind::TaskGroup => {
                let HeapReadOutput::Combinator(mut comb) = self.heap.read(owner) else {
                    unreachable!("checked above")
                };
                let error = comb.get_mut(self.heap).error.take();
                drop(comb);
                match error {
                    Some(error) => Outcome::Error(error),
                    None => Outcome::Value(Value::None),
                }
            }
        };
        self.settle_future(output, outcome);
        self.heap.dec_ref(output);
        // Last: this can drop the combinator's final reference.
        self.combinator_release_slots(owner);
    }

    /// Whether the combinator's answer is already out.
    fn combinator_output_settled(&mut self, owner: HeapId) -> bool {
        let output = self.combinator_output(owner);
        let HeapReadOutput::Future(fut) = self.heap.read(output) else {
            unreachable!("a combinator's output is a future")
        };
        let settled = fut.get(self.heap).state.is_settled();
        drop(fut);
        self.heap.dec_ref(output);
        settled
    }

    /// Ends the watch: every child still holding a slot for this combinator
    /// gives it up.
    ///
    /// A combinator is owned by nothing but those slots, while it owns the
    /// children they sit on, so the two kept each other alive for as long as
    /// any child stayed unsettled. A `gather` that failed early and a `wait`
    /// that returned on the first completion both leave children running
    /// forever by design, and their combinator then outlived every use of it.
    /// Releasing here is not an optimisation: it is where the wait ends.
    ///
    /// Nothing may touch `owner` afterwards, since the last release frees it.
    fn combinator_release_slots(&mut self, owner: HeapId) {
        let children = {
            let HeapReadOutput::Combinator(comb) = self.heap.read(owner) else {
                panic!("combinator_release_slots called off a combinator")
            };
            let children = comb.get(self.heap).children.clone();
            drop(comb);
            children
        };
        // Counted first and released after: freeing the combinator releases the
        // very children this loop is reading.
        let mut released = 0;
        for child in children {
            let HeapReadOutput::Future(mut fut) = self.heap.read(child) else {
                panic!("combinator child is not a future")
            };
            let before = fut.get(self.heap).waiters.len();
            fut.get_mut(self.heap)
                .waiters
                .retain(|waiter| !matches!(waiter, Waiter::Slot { owner: slot, .. } if *slot == owner));
            released += before - fut.get(self.heap).waiters.len();
            drop(fut);
        }
        for _ in 0..released {
            self.heap.dec_ref(owner);
        }
    }

    /// The combinator's output future, with a reference for the caller.
    fn combinator_output(&mut self, owner: HeapId) -> HeapId {
        let HeapReadOutput::Combinator(comb) = self.heap.read(owner) else {
            panic!("combinator_output called off a combinator")
        };
        let output = comb.get(self.heap).output;
        drop(comb);
        self.heap.inc_ref(output);
        output
    }

    /// Takes the filled slots, in order, leaving the combinator empty.
    fn combinator_take_results(&mut self, owner: HeapId) -> Vec<Value> {
        let HeapReadOutput::Combinator(mut comb) = self.heap.read(owner) else {
            panic!("combinator_take_results called off a combinator")
        };
        let results = mem::take(&mut comb.get_mut(self.heap).results);
        drop(comb);
        results
            .into_iter()
            .map(|slot| slot.expect("every gather slot is filled before the gather completes"))
            .collect()
    }

    /// `(done, pending)`, the pair `asyncio.wait` settles with.
    fn wait_result(&mut self, owner: HeapId) -> Value {
        let (finished, children) = {
            let HeapReadOutput::Combinator(comb) = self.heap.read(owner) else {
                panic!("wait_result called off a combinator")
            };
            let state = comb.get(self.heap);
            let pair = (state.finished.clone(), state.children.clone());
            drop(comb);
            pair
        };
        let done: Vec<Value> = finished
            .iter()
            .map(|id| {
                self.heap.inc_ref(*id);
                Value::Ref(*id)
            })
            .collect();
        let pending: Vec<Value> = children
            .iter()
            .filter(|id| !finished.contains(id))
            .map(|id| {
                self.heap.inc_ref(*id);
                Value::Ref(*id)
            })
            .collect();
        let done = self.future_set(done);
        let pending = self.future_set(pending);
        allocate_tuple(smallvec![done, pending], self.heap)
    }

    /// Hands the next settled child to a waiting `as_completed` step.
    fn as_completed_hand_out(&mut self, owner: HeapId) {
        let HeapReadOutput::Combinator(mut comb) = self.heap.read(owner) else {
            panic!("as_completed_hand_out called off a combinator")
        };
        if !matches!(comb.get(self.heap).kind, CombinatorKind::AsCompleted) {
            drop(comb);
            return;
        }
        let state = comb.get_mut(self.heap);
        let handed = match (state.next_wait, state.finished.is_empty()) {
            (Some(wait_id), false) => {
                let child = state.finished.remove(0);
                state.next_wait = None;
                Some((wait_id, child))
            }
            _ => None,
        };
        drop(comb);
        if let Some((wait_id, child)) = handed {
            self.settle_future(wait_id, Outcome::Value(Value::Ref(child)));
            self.heap.dec_ref(wait_id);
        }
    }

    /// Cancels every child of a task group that has not settled.
    fn taskgroup_cancel_children(&mut self, owner: HeapId) {
        let children = {
            let HeapReadOutput::Combinator(comb) = self.heap.read(owner) else {
                panic!("taskgroup_cancel_children called off a combinator")
            };
            let children = comb.get(self.heap).children.clone();
            drop(comb);
            children
        };
        for child in children {
            self.cancel_future(child, Value::None);
        }
    }

    // ================================================================
    // The host boundary
    // ================================================================

    /// Allocates the future standing for `call_id` and pushes it.
    pub fn add_pending_call(&mut self, call_id: CallId) {
        let future_id = self.alloc_future(FutureKind::Future, Some(call_id));
        self.scheduler.add_pending_external(call_id, future_id, self.heap);
        self.push(Value::Ref(future_id));
    }

    /// Gets the pending call IDs from the scheduler.
    pub fn get_pending_call_ids(&self) -> Vec<CallId> {
        self.scheduler.pending_call_ids()
    }

    /// Settles the future for an external call the host answered.
    ///
    /// A future the sandbox already cancelled keeps its cancellation and the
    /// value is dropped: the host cannot know the sandbox stopped caring.
    pub fn resolve_future(&mut self, call_id: u32, value: Value) {
        let Some(future_id) = self.scheduler.take_pending_external(CallId::new(call_id)) else {
            value.drop_with(self);
            return;
        };
        self.settle_future(future_id, Outcome::Value(value));
        self.heap.dec_ref(future_id);
    }

    /// Fails the future for an external call the host could not answer.
    pub fn fail_future(&mut self, call_id: u32, error: RunError) {
        let Some(future_id) = self.scheduler.take_pending_external(CallId::new(call_id)) else {
            return;
        };
        self.settle_future(future_id, Outcome::Error(error));
        self.heap.dec_ref(future_id);
    }

    /// Resolves external futures and resumes execution.
    pub fn resume_with_resolved_futures(&mut self, results: Vec<(u32, ExtFunctionResult)>) -> RunResult<FrameExit> {
        for (call_id, ext_result) in results {
            match ext_result {
                ExtFunctionResult::Return(obj) => {
                    let value = obj.to_value(self).map_err(|e| {
                        RunError::from(MontyException::runtime_error(format!(
                            "Invalid return value for call {call_id}: {e}"
                        )))
                    })?;
                    self.resolve_future(call_id, value);
                }
                ExtFunctionResult::Error(exc) => self.fail_future(call_id, RunError::from(exc)),
                ExtFunctionResult::Future(_) => {}
                ExtFunctionResult::NotFound(function_name) => {
                    self.fail_future(call_id, ExtFunctionResult::not_found_exc(&function_name));
                }
            }
        }

        if let Some(current_task_id) = self.scheduler.current_task_id() {
            match self.scheduler.get_task(current_task_id).state {
                TaskState::Failed(_) => {
                    let TaskState::Failed(err) = mem::replace(
                        &mut self.scheduler.get_task_mut(current_task_id).state,
                        TaskState::Ready,
                    ) else {
                        unreachable!("matched above");
                    };
                    self.scheduler.remove_from_ready_queue(current_task_id);
                    return self.resume_with_exception(err);
                }
                TaskState::Blocked(_) => {}
                TaskState::Ready => {
                    self.scheduler.remove_from_ready_queue(current_task_id);
                    return self.run_external();
                }
                TaskState::Completed(_) => {
                    panic!("current task is Completed after resolving futures")
                }
            }
        }

        // Still parked; something else may be able to run.
        match self.switch_or_yield() {
            Ok(AwaitResult::FramePushed) => self.run_external(),
            Ok(AwaitResult::Yield(pending)) => Ok(FrameExit::ResolveFutures(pending)),
            Ok(AwaitResult::ValueReady(value)) => {
                value.drop_with(self);
                unreachable!("switch_or_yield never resolves a value")
            }
            Err(error) => self.resume_with_exception(error),
        }
    }
}

/// `RuntimeError` for a program that has parked every task on something nothing
/// can ever settle. CPython's loop would sit there forever; a sandbox says so.
fn nothing_left_to_wait_for() -> RunError {
    SimpleException::new_msg(ExcType::RuntimeError, "every task is waiting and nothing can wake them").into()
}

/// Whether `error` is a cancellation, and so settles a task as cancelled rather
/// than as failed.
pub(crate) fn is_cancellation(error: &RunError) -> bool {
    matches!(error, RunError::Exc(exc) if exc.exc.exc_type() == ExcType::CancelledError)
}

/// The `CancelledError` a cancellation delivers, carrying `message` when one
/// was given.
pub(crate) fn cancelled_error(message: &Value, vm: &mut VM<'_>) -> RunError {
    match message {
        Value::None => SimpleException::new_none(ExcType::CancelledError).into(),
        other => match other.py_str(vm) {
            Ok(text) => {
                let rendered = text.to_str(vm).map(str::to_owned);
                text.drop_with(vm);
                match rendered {
                    Ok(rendered) => SimpleException::new_msg(ExcType::CancelledError, rendered).into(),
                    Err(error) => error,
                }
            }
            Err(error) => error,
        },
    }
}
