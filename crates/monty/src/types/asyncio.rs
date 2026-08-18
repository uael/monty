//! What `asyncio`'s objects look like from Python: `Future` and `Task`, and the
//! coordination primitives built on them.
//!
//! Every primitive here works the same way. It never waits; it hands out a
//! future and settles it later. `Lock.acquire()` returns a future that settles
//! when the lock is free, `Event.wait()` one that settles when the event is
//! set, `Queue.get()` one that settles when an item arrives. That is why none
//! of them needs a scheduler of its own, and why `async with lock:` works
//! through the ordinary `__aenter__` / `await` path.
//!
//! The one divergence this shape forces is stated in `limitations/asyncio.md`:
//! CPython's `acquire()` returns a coroutine that does nothing until awaited,
//! while these take effect at the call.

use std::{collections::VecDeque, fmt::Write, mem};

use crate::{
    args::ArgValues,
    asyncio::{Combinator, Future, FutureKind, FutureState},
    bytecode::{CallResult, Outcome, VM},
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, ExceptionObject, RunError, RunResult, SimpleException},
    hash::{HashValue, identity_hash},
    heap::{DropWithContext, HeapData, HeapId, HeapItem, HeapRead},
    modules::asyncio::call_primitive_method,
    types::{LazyHeapSet, PyTrait, Type, str::allocate_string},
    value::{EitherStr, Value},
};

/// One of `asyncio`'s coordination objects.
///
/// They share a heap variant because they share a shape: a little state, plus a
/// list of futures to settle when that state changes. Nothing outside this
/// module dispatches on which one it is.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum AsyncPrimitive {
    Lock(Lock),
    Event(Event),
    Semaphore(Semaphore),
    Barrier(Barrier),
    Queue(Queue),
    TaskGroup(TaskGroup),
    Timeout(Timeout),
}

/// `asyncio.Lock`: at most one holder, the rest queued in arrival order.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct Lock {
    /// Whether someone holds it.
    pub locked: bool,
    /// Futures handed to waiting acquirers, oldest first. Owned.
    pub waiters: VecDeque<HeapId>,
}

/// `asyncio.Event`: a flag every waiter sees once it is raised.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct Event {
    /// Whether the flag is up.
    pub is_set: bool,
    /// Futures handed to waiters while it was down. Owned.
    pub waiters: VecDeque<HeapId>,
}

/// `asyncio.Semaphore`, and `BoundedSemaphore` when `bound` is set.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Semaphore {
    /// Permits still available.
    pub value: i64,
    /// The starting count a `BoundedSemaphore` refuses to exceed.
    pub bound: Option<i64>,
    /// Futures handed to waiting acquirers, oldest first. Owned.
    pub waiters: VecDeque<HeapId>,
}

/// `asyncio.Barrier`: every party waits until `parties` of them have arrived.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Barrier {
    /// How many have to arrive before the barrier opens.
    pub parties: i64,
    /// Futures handed to the parties waiting now, in arrival order. Owned.
    pub waiters: VecDeque<HeapId>,
}

/// `asyncio.Queue`, with the blocking and the no-wait halves of `put` and `get`.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Queue {
    /// Items waiting to be taken. Owned.
    pub items: VecDeque<Value>,
    /// Capacity, or zero for unbounded, as in CPython.
    pub maxsize: i64,
    /// Futures handed to waiting `get()`s, oldest first. Owned.
    pub getters: VecDeque<HeapId>,
    /// Futures handed to waiting `put()`s, with the item each is holding.
    /// Both owned.
    pub putters: VecDeque<(HeapId, Value)>,
    /// Items put but not yet `task_done()`, which is what `join()` waits for.
    pub unfinished: i64,
    /// Futures handed to waiting `join()`s. Owned.
    pub join_waiters: VecDeque<HeapId>,
}

/// `asyncio.TaskGroup`: an async context manager whose body spawns tasks and
/// whose exit waits for all of them.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct TaskGroup {
    /// Whether `__aenter__` has run, which is what `create_task` requires.
    pub entered: bool,
    /// The tasks spawned inside the body. Owned.
    pub children: Vec<HeapId>,
}

/// `asyncio.timeout(delay)`: an async context manager that cancels the task
/// running its body once the clock reaches the deadline.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Timeout {
    /// The scheduler time at which the body is cancelled, or `None` for a
    /// `timeout(None)` that never fires.
    pub deadline: Option<u64>,
    /// The future the timer settles, which is also what disarms it. Owned.
    pub timer: Option<HeapId>,
    /// Whether the deadline passed, which turns the cancellation into a
    /// `TimeoutError` on the way out.
    pub expired: bool,
}

impl AsyncPrimitive {
    /// The Python type this object reports.
    pub(crate) fn py_type(&self) -> Type {
        match self {
            Self::Lock(_) => Type::Lock,
            Self::Event(_) => Type::Event,
            Self::Semaphore(semaphore) => {
                if semaphore.bound.is_some() {
                    Type::BoundedSemaphore
                } else {
                    Type::Semaphore
                }
            }
            Self::Barrier(_) => Type::Barrier,
            Self::Queue(_) => Type::Queue,
            Self::TaskGroup(_) => Type::TaskGroup,
            Self::Timeout(_) => Type::Timeout,
        }
    }

    /// Pushes every heap id this primitive owns.
    ///
    /// The one place they are listed: both the GC's child walk and the
    /// destructor's reference release go through it.
    pub(crate) fn owned_ids(&self, push: &mut impl FnMut(HeapId)) {
        let queued = |waiters: &VecDeque<HeapId>, push: &mut dyn FnMut(HeapId)| {
            for waiter in waiters {
                push(*waiter);
            }
        };
        match self {
            Self::Lock(lock) => queued(&lock.waiters, push),
            Self::Event(event) => queued(&event.waiters, push),
            Self::Semaphore(semaphore) => queued(&semaphore.waiters, push),
            Self::Barrier(barrier) => queued(&barrier.waiters, push),
            Self::Queue(queue) => {
                for item in &queue.items {
                    if let Value::Ref(id) = item {
                        push(*id);
                    }
                }
                queued(&queue.getters, push);
                for (future, item) in &queue.putters {
                    push(*future);
                    if let Value::Ref(id) = item {
                        push(*id);
                    }
                }
                queued(&queue.join_waiters, push);
            }
            Self::TaskGroup(group) => {
                for child in &group.children {
                    push(*child);
                }
            }
            Self::Timeout(timeout) => {
                if let Some(timer) = timeout.timer {
                    push(timer);
                }
            }
        }
    }
}

impl HeapItem for AsyncPrimitive {
    fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        // Mirrors `AsyncPrimitive::owned_ids`, which the GC's child walk uses.
        // The two cannot share a body: a destructor must hand each owned
        // `Value` to `Value::py_dec_ref_ids`, which neutralizes it as well as
        // reporting it, and that needs `&mut`.
        match self {
            Self::Lock(lock) => stack.extend(lock.waiters.iter().copied()),
            Self::Event(event) => stack.extend(event.waiters.iter().copied()),
            Self::Semaphore(semaphore) => stack.extend(semaphore.waiters.iter().copied()),
            Self::Barrier(barrier) => stack.extend(barrier.waiters.iter().copied()),
            Self::Queue(queue) => {
                for item in &mut queue.items {
                    item.py_dec_ref_ids(stack);
                }
                stack.extend(queue.getters.iter().copied());
                for (future, item) in &mut queue.putters {
                    stack.push(*future);
                    item.py_dec_ref_ids(stack);
                }
                stack.extend(queue.join_waiters.iter().copied());
            }
            Self::TaskGroup(group) => stack.extend(group.children.iter().copied()),
            Self::Timeout(timeout) => stack.extend(timeout.timer),
        }
    }
}

// ====================================================================
// Future and Task
// ====================================================================

impl Future {
    /// `asyncio.Future` or `asyncio.Task`, which is all the two flavours differ
    /// by outside their extra methods.
    pub(crate) fn py_type(&self) -> Type {
        match self.kind {
            FutureKind::Future => Type::Future,
            FutureKind::Task => Type::Task,
        }
    }
}

impl<'h> PyTrait<'h> for HeapRead<'h, Future> {
    fn py_type(&self, vm: &VM<'h>) -> Type {
        self.get(vm.heap).py_type()
    }

    fn py_len(&self, _vm: &VM<'h>) -> Option<usize> {
        None
    }

    fn py_eq_impl(&self, _other: &Value, _vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        // CPython defines no `__eq__`: two futures are equal only if identical.
        Ok(None)
    }

    fn py_hash(&self, self_id: HeapId, _vm: &mut VM<'h>) -> RunResult<Option<HashValue>> {
        Ok(Some(identity_hash(self_id)))
    }

    fn py_bool(&self, _vm: &mut VM<'h>) -> RunResult<bool> {
        Ok(true)
    }

    /// CPython's shape, minus the pieces it draws from an address or a frame:
    /// `<Future pending>`, `<Future finished result=1>`, `<Task cancelled>`.
    fn py_repr_fmt(&self, f: &mut impl Write, vm: &mut VM<'h>, heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        let name = match self.get(vm.heap).kind {
            FutureKind::Future => "Future",
            FutureKind::Task => "Task",
        };
        let settled = match &self.get(vm.heap).state {
            FutureState::Pending => return Ok(write!(f, "<{name} pending>")?),
            FutureState::Cancelled(_) => return Ok(write!(f, "<{name} cancelled>")?),
            FutureState::Finished(value) => Ok(value.clone_with_heap(vm.heap)),
            FutureState::Failed(error) => Err(error.clone()),
        };
        write!(f, "<{name} finished ")?;
        match settled {
            Ok(value) => {
                f.write_str("result=")?;
                let written = value.py_repr_fmt(f, vm, heap_ids);
                value.drop_with(vm);
                written?;
            }
            Err(error) => {
                f.write_str("exception=")?;
                let value = error_as_value(&error, vm);
                let written = value.py_repr_fmt(f, vm, heap_ids);
                value.drop_with(vm);
                written?;
            }
        }
        Ok(write!(f, ">")?)
    }

    fn py_getattr(&self, attr: &EitherStr, vm: &mut VM<'h>) -> RunResult<Option<CallResult>> {
        match attr.as_str(vm.interns) {
            // A future is its own `__await__` iterator; see `heap_data.rs` for
            // why the coroutine case lives there instead.
            "_asyncio_future_blocking" => Ok(Some(CallResult::Value(Value::Bool(
                !self.get(vm.heap).state.is_settled(),
            )))),
            _ => Ok(None),
        }
    }

    fn py_call_attr(
        &mut self,
        self_id: HeapId,
        vm: &mut VM<'h>,
        attr: &EitherStr,
        args: ArgValues,
    ) -> RunResult<CallResult> {
        let is_task = matches!(self.get(vm.heap).kind, FutureKind::Task);
        match attr.as_str(vm.interns) {
            "done" => {
                args.check_zero_args("done", vm.heap)?;
                Ok(CallResult::Value(Value::Bool(self.get(vm.heap).state.is_settled())))
            }
            "cancelled" => {
                args.check_zero_args("cancelled", vm.heap)?;
                Ok(CallResult::Value(Value::Bool(matches!(
                    self.get(vm.heap).state,
                    FutureState::Cancelled(_)
                ))))
            }
            "result" => {
                args.check_zero_args("result", vm.heap)?;
                match &self.get(vm.heap).state {
                    FutureState::Finished(value) => Ok(CallResult::Value(value.clone_with_heap(vm.heap))),
                    FutureState::Failed(error) | FutureState::Cancelled(error) => Err(error.clone()),
                    FutureState::Pending => Err(invalid_state("Result is not set.")),
                }
            }
            "exception" => {
                args.check_zero_args("exception", vm.heap)?;
                match &self.get(vm.heap).state {
                    FutureState::Finished(_) => Ok(CallResult::Value(Value::None)),
                    FutureState::Cancelled(error) => Err(error.clone()),
                    FutureState::Failed(error) => {
                        let error = error.clone();
                        Ok(CallResult::Value(error_as_value(&error, vm)))
                    }
                    FutureState::Pending => Err(invalid_state("Exception is not set.")),
                }
            }
            "cancel" => {
                let message = args.get_zero_one_arg("cancel", vm.heap)?.unwrap_or(Value::None);
                Ok(CallResult::Value(Value::Bool(vm.cancel_future(self_id, message))))
            }
            "set_result" => {
                let Some(value) = args.get_zero_one_arg("set_result", vm.heap)? else {
                    return Err(ExcType::type_error(
                        "Future.set_result() takes exactly one argument (0 given)",
                    ));
                };
                if self.get(vm.heap).state.is_settled() {
                    value.drop_with(vm);
                    return Err(invalid_state_settled());
                }
                vm.settle_future(self_id, Outcome::Value(value));
                Ok(CallResult::Value(Value::None))
            }
            "set_exception" => {
                let Some(value) = args.get_zero_one_arg("set_exception", vm.heap)? else {
                    return Err(ExcType::type_error(
                        "Future.set_exception() takes exactly one argument (0 given)",
                    ));
                };
                defer_drop!(value, vm);
                if self.get(vm.heap).state.is_settled() {
                    return Err(invalid_state_settled());
                }
                let error = vm.make_exception(value, true);
                vm.settle_future(self_id, Outcome::Error(error));
                Ok(CallResult::Value(Value::None))
            }
            "add_done_callback" => {
                let Some(callback) = args.get_zero_one_arg("add_done_callback", vm.heap)? else {
                    return Err(ExcType::type_error(
                        "Future.add_done_callback() takes exactly one argument (0 given)",
                    ));
                };
                if self.get(vm.heap).state.is_settled() {
                    // Already settled: the loop owes the call straight away.
                    vm.schedule_callback(self_id, callback);
                } else {
                    self.get_mut(vm.heap).callbacks.push(callback);
                }
                Ok(CallResult::Value(Value::None))
            }
            "remove_done_callback" => {
                let Some(callback) = args.get_zero_one_arg("remove_done_callback", vm.heap)? else {
                    return Err(ExcType::type_error(
                        "Future.remove_done_callback() takes exactly one argument (0 given)",
                    ));
                };
                defer_drop!(callback, vm);
                let mut removed = 0;
                let mut kept = Vec::new();
                for existing in mem::take(&mut self.get_mut(vm.heap).callbacks) {
                    if existing.is(callback) {
                        removed += 1;
                        existing.drop_with(vm);
                    } else {
                        kept.push(existing);
                    }
                }
                self.get_mut(vm.heap).callbacks = kept;
                Ok(CallResult::Value(Value::Int(removed)))
            }
            "get_name" if is_task => {
                args.check_zero_args("get_name", vm.heap)?;
                let name = match &self.get(vm.heap).name {
                    Some(EitherStr::Interned(id)) => Value::InternString(*id),
                    Some(EitherStr::Heap(name)) => allocate_string(name.clone(), vm.heap),
                    None => {
                        let number = self.get(vm.heap).run.as_ref().map_or(0, |run| run.task_id.raw());
                        allocate_string(format!("Task-{number}"), vm.heap)
                    }
                };
                Ok(CallResult::Value(name))
            }
            "set_name" if is_task => {
                let Some(name) = args.get_zero_one_arg("set_name", vm.heap)? else {
                    return Err(ExcType::type_error(
                        "Task.set_name() takes exactly one argument (0 given)",
                    ));
                };
                let rendered = name.py_str(vm)?;
                name.drop_with(vm);
                let interned = rendered.as_either_str(vm.heap);
                rendered.drop_with(vm);
                self.get_mut(vm.heap).name = interned;
                Ok(CallResult::Value(Value::None))
            }
            "get_coro" if is_task => {
                args.check_zero_args("get_coro", vm.heap)?;
                let coroutine = self.get(vm.heap).run.as_ref().and_then(|run| run.coroutine);
                Ok(CallResult::Value(match coroutine {
                    Some(id) => {
                        vm.heap.inc_ref(id);
                        Value::Ref(id)
                    }
                    None => Value::None,
                }))
            }
            "cancelling" if is_task => {
                args.check_zero_args("cancelling", vm.heap)?;
                Ok(CallResult::Value(Value::Int(i64::from(self.get(vm.heap).cancelling))))
            }
            "uncancel" if is_task => {
                args.check_zero_args("uncancel", vm.heap)?;
                let state = self.get_mut(vm.heap);
                state.cancelling = state.cancelling.saturating_sub(1);
                Ok(CallResult::Value(Value::Int(i64::from(state.cancelling))))
            }
            "__await__" => {
                args.check_zero_args("__await__", vm.heap)?;
                vm.heap.inc_ref(self_id);
                Ok(CallResult::Value(Value::Ref(self_id)))
            }
            other => {
                let other = other.to_owned();
                let type_ = self.get(vm.heap).py_type();
                args.drop_with(vm);
                Err(ExcType::attribute_error(type_, &other))
            }
        }
    }
}

/// `asyncio.InvalidStateError` with the message CPython uses.
fn invalid_state(message: &str) -> RunError {
    SimpleException::new_msg(ExcType::InvalidStateError, message).into()
}

/// What CPython's `_asyncio` accelerator raises for setting a settled future.
/// Its pure-Python fallback spells the state and the repr out; the accelerator
/// is what ships, and this matches it.
fn invalid_state_settled() -> RunError {
    invalid_state("invalid state")
}

/// The exception object standing for `error`, which is what
/// `Future.exception()` and `gather(return_exceptions=True)` hand back.
///
/// A resource-limit error has no Python object: it is uncatchable by design, so
/// there is nothing to hand out and callers re-raise instead.
pub(crate) fn error_as_value_opt(error: &RunError, vm: &mut VM<'_>) -> Option<Value> {
    match error {
        RunError::Exc(exc) => {
            let object = ExceptionObject::from_summary(exc.exc.clone(), vm);
            Some(Value::Ref(vm.heap.allocate(HeapData::Exception(Box::new(object)))))
        }
        RunError::UncatchableExc(_) | RunError::Internal(_) => None,
    }
}

/// [`error_as_value_opt`] for the sites that have already excluded the
/// uncatchable case, or where `None` is not an option.
fn error_as_value(error: &RunError, vm: &mut VM<'_>) -> Value {
    error_as_value_opt(error, vm).unwrap_or(Value::None)
}

/// Allocates a primitive on the heap and hands back a reference to it.
pub(crate) fn allocate_primitive(primitive: AsyncPrimitive, vm: &mut VM<'_>) -> Value {
    Value::Ref(vm.heap.allocate(HeapData::AsyncPrimitive(Box::new(primitive))))
}

/// `asyncio.Future()` and `asyncio.Task(coro)`.
pub(crate) fn init_future(type_: Type, vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    match type_ {
        Type::Future => {
            args.check_zero_args("Future", vm.heap)?;
            let id = vm.alloc_future(FutureKind::Future, None);
            Ok(Value::Ref(id))
        }
        Type::Task => {
            let Some(coroutine) = args.get_zero_one_arg("Task", vm.heap)? else {
                return Err(ExcType::type_error(
                    "Task() missing 1 required positional argument: 'coro'",
                ));
            };
            defer_drop!(coroutine, vm);
            let Value::Ref(id) = coroutine else {
                return Err(ExcType::type_error("a coroutine was expected"));
            };
            if !matches!(vm.heap.get(*id), HeapData::Coroutine(_)) {
                return Err(ExcType::type_error("a coroutine was expected"));
            }
            vm.spawn_task(*id)
        }
        other => panic!("asyncio::init_future called for {other:?}"),
    }
}

/// `Lock()`, `Event()`, `Semaphore(value)`, ... : the constructors the
/// `asyncio` module exposes as type objects.
pub(crate) fn construct(type_: Type, vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let primitive = match type_ {
        Type::Lock => {
            args.check_zero_args("Lock", vm.heap)?;
            AsyncPrimitive::Lock(Lock::default())
        }
        Type::Event => {
            args.check_zero_args("Event", vm.heap)?;
            AsyncPrimitive::Event(Event::default())
        }
        Type::Semaphore | Type::BoundedSemaphore => {
            let value = match args.get_zero_one_arg("Semaphore", vm.heap)? {
                Some(value) => value.as_int(vm)?,
                None => 1,
            };
            if value < 0 {
                return Err(ExcType::value_error("Semaphore initial value must be >= 0"));
            }
            AsyncPrimitive::Semaphore(Semaphore {
                value,
                bound: (type_ == Type::BoundedSemaphore).then_some(value),
                waiters: VecDeque::new(),
            })
        }
        Type::Barrier => {
            let Some(parties) = args.get_zero_one_arg("Barrier", vm.heap)? else {
                return Err(ExcType::type_error(
                    "Barrier() missing 1 required positional argument: 'parties'",
                ));
            };
            let parties = parties.as_int(vm)?;
            if parties < 1 {
                return Err(ExcType::value_error("parties must be > 0"));
            }
            AsyncPrimitive::Barrier(Barrier {
                parties,
                waiters: VecDeque::new(),
            })
        }
        Type::Queue => {
            let maxsize = match args.get_zero_one_arg("Queue", vm.heap)? {
                Some(value) => value.as_int(vm)?,
                None => 0,
            };
            AsyncPrimitive::Queue(Queue {
                items: VecDeque::new(),
                maxsize: maxsize.max(0),
                getters: VecDeque::new(),
                putters: VecDeque::new(),
                unfinished: 0,
                join_waiters: VecDeque::new(),
            })
        }
        Type::TaskGroup => {
            args.check_zero_args("TaskGroup", vm.heap)?;
            AsyncPrimitive::TaskGroup(TaskGroup::default())
        }
        other => panic!("asyncio::construct called for {other:?}"),
    };
    Ok(allocate_primitive(primitive, vm))
}

impl<'h> PyTrait<'h> for HeapRead<'h, AsyncPrimitive> {
    fn py_type(&self, vm: &VM<'h>) -> Type {
        self.get(vm.heap).py_type()
    }

    /// None of them define `__len__`, `asyncio.Queue` included: its size is
    /// `qsize()`.
    fn py_len(&self, _vm: &VM<'h>) -> Option<usize> {
        None
    }

    fn py_eq_impl(&self, _other: &Value, _vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        Ok(None)
    }

    fn py_hash(&self, self_id: HeapId, _vm: &mut VM<'h>) -> RunResult<Option<HashValue>> {
        Ok(Some(identity_hash(self_id)))
    }

    fn py_bool(&self, _vm: &mut VM<'h>) -> RunResult<bool> {
        Ok(true)
    }

    fn py_repr_fmt(&self, f: &mut impl Write, vm: &mut VM<'h>, _heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        let name = self.get(vm.heap).py_type().name(vm.heap, vm.interns).into_owned();
        Ok(write!(f, "<{name}>")?)
    }

    fn py_getattr(&self, attr: &EitherStr, vm: &mut VM<'h>) -> RunResult<Option<CallResult>> {
        let value = match (self.get(vm.heap), attr.as_str(vm.interns)) {
            (AsyncPrimitive::Barrier(barrier), "parties") => Value::Int(barrier.parties),
            (AsyncPrimitive::Barrier(barrier), "n_waiting") => {
                Value::Int(i64::try_from(barrier.waiters.len()).unwrap_or(i64::MAX))
            }
            (AsyncPrimitive::Queue(queue), "maxsize") => Value::Int(queue.maxsize),
            _ => return Ok(None),
        };
        Ok(Some(CallResult::Value(value)))
    }

    fn py_call_attr(
        &mut self,
        self_id: HeapId,
        vm: &mut VM<'h>,
        attr: &EitherStr,
        args: ArgValues,
    ) -> RunResult<CallResult> {
        let method = attr.as_str(vm.interns).to_owned();
        call_primitive_method(self_id, &method, vm, args).map(CallResult::Value)
    }
}

// ====================================================================
// as_completed
// ====================================================================

impl<'h> PyTrait<'h> for HeapRead<'h, Combinator> {
    fn py_type(&self, _vm: &VM<'h>) -> Type {
        Type::AsCompleted
    }

    fn py_len(&self, vm: &VM<'h>) -> Option<usize> {
        Some(self.get(vm.heap).children.len())
    }

    fn py_eq_impl(&self, _other: &Value, _vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        Ok(None)
    }

    fn py_hash(&self, self_id: HeapId, _vm: &mut VM<'h>) -> RunResult<Option<HashValue>> {
        Ok(Some(identity_hash(self_id)))
    }

    fn py_bool(&self, _vm: &mut VM<'h>) -> RunResult<bool> {
        Ok(true)
    }

    fn py_repr_fmt(&self, f: &mut impl Write, vm: &mut VM<'h>, _heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        Ok(write!(f, "<as_completed({})>", self.get(vm.heap).children.len())?)
    }

    fn py_call_attr(
        &mut self,
        self_id: HeapId,
        vm: &mut VM<'h>,
        attr: &EitherStr,
        args: ArgValues,
    ) -> RunResult<CallResult> {
        match attr.as_str(vm.interns) {
            "__aiter__" => {
                args.check_zero_args("__aiter__", vm.heap)?;
                vm.heap.inc_ref(self_id);
                Ok(CallResult::Value(Value::Ref(self_id)))
            }
            // Each step hands back a future settling with the next child to
            // finish, so `async for` over it yields them in completion order.
            "__anext__" => {
                args.check_zero_args("__anext__", vm.heap)?;
                vm.as_completed_next(self_id).map(CallResult::Value)
            }
            other => {
                let other = other.to_owned();
                args.drop_with(vm);
                Err(ExcType::attribute_error(Type::AsCompleted, &other))
            }
        }
    }
}
