//! The event loop: which task runs next, what time it is, and which external
//! calls the host still owes an answer for.
//!
//! # Task model
//!
//! Task 0 is the main task, which runs on the VM's own stacks. Every other task
//! carries its own copy of those stacks and is swapped in and out around a
//! suspension. A task drives either a coroutine, on behalf of the
//! `asyncio.Task` watching it, or one done-callback the loop owes a call.
//!
//! # Time
//!
//! Time is the scheduler's own counter, never the host clock, because a run has
//! to replay identically. It advances only when nothing else can happen: no
//! ready task, and no external call outstanding. So a host call costs zero
//! sandbox time, and the order sandbox code observes is a function of the
//! program alone.

use std::{collections::VecDeque, mem};

use ahash::AHashMap;

use crate::{
    asyncio::{CallId, TaskId},
    bytecode::vm::generator::GenActivation,
    exception_private::RunError,
    heap::{ContainsHeap, DropWithContext, Heap, HeapId, HeapReader},
    intern::FunctionId,
    value::Value,
};

/// Task execution state for async scheduling.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum TaskState {
    /// Ready to execute, and so in the ready queue (or the current task).
    Ready,
    /// Parked on the future at this id, which holds a `Waiter::Task` pointing
    /// back here. Owns an inc_ref so the future outlives the wait.
    Blocked(HeapId),
    /// Ran to its end with this value.
    Completed(Value),
    /// Ended with this error.
    Failed(RunError),
}

impl<C: ContainsHeap> DropWithContext<C> for TaskState {
    fn drop_with(self, heap: &mut C) {
        match self {
            Self::Ready | Self::Failed(_) => {}
            Self::Blocked(id) => heap.heap_mut().dec_ref(id),
            Self::Completed(value) => value.drop_with(heap),
        }
    }
}

/// What a task exists to run.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum TaskBody {
    /// The main task: module-level code, whose frames the VM already holds.
    /// Its `asyncio.Task` object is made only when something asks for one
    /// (`current_task()`, `asyncio.timeout`), since module-level code has no
    /// coroutine to drive and nothing to report a result to. Owned.
    Main { future: Option<HeapId> },
    /// A coroutine, driven for the `asyncio.Task` at `future`. Both ids are
    /// owned; `future` is what the outcome is reported to.
    Coroutine { coroutine: HeapId, future: HeapId },
    /// One `add_done_callback` callable, called with `arg` and then discarded.
    /// Both values are owned.
    Callback { func: Value, arg: Value },
}

impl TaskBody {
    /// Pushes every heap id this body owns; mirrors its `drop_with`.
    #[cfg(feature = "ref-count-return")]
    fn owned_ids(&self, push: &mut impl FnMut(HeapId)) {
        match self {
            Self::Main { future } => push_opt(*future, push),
            Self::Coroutine { coroutine, future } => {
                push(*coroutine);
                push(*future);
            }
            Self::Callback { func, arg } => {
                push_value(func, push);
                push_value(arg, push);
            }
        }
    }
}

impl<C: ContainsHeap> DropWithContext<C> for TaskBody {
    fn drop_with(self, heap: &mut C) {
        match self {
            Self::Main { future } => {
                if let Some(future) = future {
                    heap.heap_mut().dec_ref(future);
                }
            }
            Self::Coroutine { coroutine, future } => {
                heap.heap_mut().dec_ref(coroutine);
                heap.heap_mut().dec_ref(future);
            }
            Self::Callback { func, arg } => {
                func.drop_with(heap);
                arg.drop_with(heap);
            }
        }
    }
}

/// A single async task with its own execution context.
///
/// The main task does not store frames or stacks here: it uses the VM's
/// directly, and only borrows these fields while another task runs.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Task {
    /// Unique identifier for this task.
    pub id: TaskId,
    /// Serialized call frames, empty while this task is the current one.
    pub frames: Vec<SerializedTaskFrame>,
    /// Operand stack, empty while this task is the current one.
    pub stack: Vec<Value>,
    /// Exception stack for nested except blocks.
    pub exception_stack: Vec<Value>,
    /// In-flight generator steps, which index into this task's own stacks and
    /// so must travel with them.
    pub gen_activations: Vec<GenActivation>,
    /// VM-level instruction_ip (for exception table lookup).
    pub instruction_ip: usize,
    /// What this task runs.
    pub body: TaskBody,
    /// Current execution state.
    pub state: TaskState,
}

impl Task {
    /// Pushes every heap id this task owns; mirrors its `drop_with`, which is
    /// what makes it the right answer to "what would the loop release if it
    /// were torn down now".
    #[cfg(feature = "ref-count-return")]
    fn owned_ids(&self, push: &mut impl FnMut(HeapId)) {
        for value in self.stack.iter().chain(&self.exception_stack) {
            push_value(value, push);
        }
        for activation in &self.gen_activations {
            push(activation.generator_id());
        }
        match &self.state {
            TaskState::Blocked(id) => push(*id),
            TaskState::Completed(value) => push_value(value, push),
            TaskState::Ready | TaskState::Failed(_) => {}
        }
        self.body.owned_ids(push);
    }
}

impl<C: ContainsHeap> DropWithContext<C> for Task {
    fn drop_with(mut self, heap: &mut C) {
        self.stack.drain(..).drop_with(heap);
        self.exception_stack.drain(..).drop_with(heap);
        self.gen_activations.drain(..).drop_with(heap);
        self.state.drop_with(heap);
        self.body.drop_with(heap);
    }
}

/// Serialized call frame for task storage.
///
/// Similar to `SerializedFrame` but used within the scheduler for task context.
/// Cannot store `&Code` references - uses `FunctionId` to look up code on resume.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct SerializedTaskFrame {
    /// Which function's code this frame executes (None = module-level).
    pub function_id: Option<FunctionId>,
    /// Instruction pointer within this frame's bytecode.
    pub ip: usize,
    /// Base index into the VM stack for this frame's locals region.
    pub stack_base: usize,
    /// Number of local variable slots (0 for module-level frames).
    pub locals_count: u16,
    /// Which global namespace this frame resolves its globals against, so a
    /// task parked in one namespace resumes in it however many tasks in other
    /// namespaces ran in between. See `CallFrame.scope`.
    #[serde(default)]
    pub scope: crate::namespace::ScopeId,
    /// Base index into the VM-wide `exception_stack` for this frame.
    /// See `CallFrame.exception_stack_base`.
    pub exception_stack_base: usize,
    /// Caller's bytecode offset at the call site (for tracebacks). See
    /// `CallFrame.call_offset`.
    pub call_offset: Option<u32>,
    /// Whether this frame is a class `__init__` (see `CallFrame.is_initializer`).
    #[serde(default)]
    pub is_initializer: bool,
}

/// One armed timer: what the clock owes at `deadline`.
///
/// `seq` breaks ties so two timers armed for the same instant fire in the order
/// they were armed, which is what makes an interleaving reproducible.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Timer {
    /// Scheduler time, in nanoseconds, at which this fires.
    pub deadline: u64,
    /// Arming order, the tie-break within one instant.
    pub seq: u64,
    /// The future this settles. Owned. `asyncio.timeout` reads its state to
    /// tell whether the deadline passed, so it settles even when the timer's
    /// real job is the cancellation below.
    pub future: HeapId,
    /// What `future` settles with, which is `sleep`'s `result` argument.
    pub value: Value,
    /// A future to cancel when this fires, which is how `asyncio.timeout`
    /// interrupts the task running its body. Owned when present.
    pub cancel_target: Option<HeapId>,
}

impl<C: ContainsHeap> DropWithContext<C> for Timer {
    fn drop_with(self, heap: &mut C) {
        heap.heap_mut().dec_ref(self.future);
        self.value.drop_with(heap);
        if let Some(target) = self.cancel_target {
            heap.heap_mut().dec_ref(target);
        }
    }
}

impl Task {
    /// A task in the `Ready` state with no frames yet.
    pub fn new(id: TaskId, body: TaskBody) -> Self {
        Self {
            id,
            frames: Vec::new(),
            stack: Vec::new(),
            exception_stack: Vec::new(),
            gen_activations: Vec::new(),
            instruction_ip: 0,
            body,
            state: TaskState::Ready,
        }
    }

    /// The `asyncio.Task` this task reports its outcome to, if it has one.
    #[inline]
    pub fn future(&self) -> Option<HeapId> {
        match &self.body {
            TaskBody::Coroutine { future, .. } => Some(*future),
            TaskBody::Main { future } => *future,
            TaskBody::Callback { .. } => None,
        }
    }

    /// Records the `asyncio.Task` object made on demand for the main task.
    /// Takes ownership of `future`.
    #[inline]
    pub fn set_main_future(&mut self, future: HeapId) {
        match &mut self.body {
            TaskBody::Main { future: slot } => *slot = Some(future),
            other => panic!("set_main_future on a {other:?} task"),
        }
    }
}

/// The event loop's state: tasks, the ready queue, the clock, its timers, and
/// the external calls the host still owes.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Scheduler {
    /// All tasks keyed by their `TaskId`.
    tasks: AHashMap<TaskId, Task>,
    /// Queue of task IDs ready to execute.
    ready_queue: VecDeque<TaskId>,
    /// Currently executing task (None only during task switching).
    current_task: Option<TaskId>,
    /// Counter for generating new task IDs.
    next_task_id: u32,
    /// Counter for external call IDs (always incremented, even for sync resolution).
    next_call_id: u32,
    /// Unresolved external calls, each mapped to the future the host settles.
    /// The scheduler holds an inc_ref so the entry outlives the round trip even
    /// when nothing in the sandbox still refers to it.
    pending_externals: AHashMap<CallId, HeapId>,
    /// Armed timers, earliest first. Small enough that a sorted vector beats a
    /// heap, and it serializes without ceremony.
    timers: Vec<Timer>,
    /// Scheduler time in nanoseconds. See the module docs for why it is not the
    /// host's.
    now: u64,
    /// Arming counter, the tie-break in [`Timer::seq`].
    next_timer_seq: u64,
}

impl Scheduler {
    /// Creates a new scheduler with the main task (task 0) as current.
    pub fn new() -> Self {
        let main_task_id = TaskId::default();
        let mut tasks = AHashMap::new();
        tasks.insert(main_task_id, Task::new(main_task_id, TaskBody::Main { future: None }));
        Self {
            tasks,
            ready_queue: VecDeque::new(),
            current_task: Some(main_task_id),
            next_task_id: 1,
            next_call_id: 0,
            pending_externals: AHashMap::new(),
            timers: Vec::new(),
            now: 0,
            next_timer_seq: 0,
        }
    }

    /// Returns the currently executing task ID.
    #[inline]
    pub fn current_task_id(&self) -> Option<TaskId> {
        self.current_task
    }

    /// Returns a reference to a task by ID.
    ///
    /// # Panics
    /// Panics if the task ID doesn't exist.
    #[inline]
    pub fn get_task(&self, task_id: TaskId) -> &Task {
        self.tasks.get(&task_id).expect("Scheduler::get_task: task not found")
    }

    /// Returns a mutable reference to a task by ID.
    ///
    /// # Panics
    /// Panics if the task ID doesn't exist.
    #[inline]
    pub fn get_task_mut(&mut self, task_id: TaskId) -> &mut Task {
        self.tasks
            .get_mut(&task_id)
            .expect("Scheduler::get_task_mut: task not found")
    }

    /// The task, if it still exists.
    #[inline]
    pub fn try_task(&self, task_id: TaskId) -> Option<&Task> {
        self.tasks.get(&task_id)
    }

    /// Allocates a new CallId for an external function call.
    pub fn allocate_call_id(&mut self) -> CallId {
        let id = CallId::new(self.next_call_id);
        self.next_call_id += 1;
        id
    }

    /// Registers a freshly created future as the answer slot for `call_id`.
    pub fn add_pending_external(&mut self, call_id: CallId, future_id: HeapId, heap: &Heap) {
        heap.inc_ref(future_id);
        let prev = self.pending_externals.insert(call_id, future_id);
        debug_assert!(prev.is_none(), "add_pending_external: CallId already registered");
    }

    /// Removes and returns the future the host was going to settle for
    /// `call_id`. The caller inherits the scheduler's inc_ref.
    pub fn take_pending_external(&mut self, call_id: CallId) -> Option<HeapId> {
        self.pending_externals.remove(&call_id)
    }

    /// Returns all pending (unresolved) CallIds.
    pub fn pending_call_ids(&self) -> Vec<CallId> {
        self.pending_externals.keys().copied().collect()
    }

    /// Whether the host still owes an answer for anything.
    #[inline]
    pub fn has_pending_externals(&self) -> bool {
        !self.pending_externals.is_empty()
    }

    /// Marks the current task as parked on `awaitable_id`, taking an inc_ref so
    /// the future outlives the wait.
    pub fn block_current_on(&mut self, awaitable_id: HeapId, heap: &Heap) {
        if let Some(task_id) = self.current_task {
            heap.inc_ref(awaitable_id);
            let task = self.get_task_mut(task_id);
            task.state = TaskState::Blocked(awaitable_id);
        }
    }

    /// Removes a task from the ready queue.
    pub fn remove_from_ready_queue(&mut self, task_id: TaskId) {
        self.ready_queue.retain(|&id| id != task_id);
    }

    /// Whether anything is ready to run right now.
    #[inline]
    pub fn ready_is_empty(&self) -> bool {
        self.ready_queue.is_empty()
    }

    /// Creates a task for `body` and queues it to run.
    ///
    /// The caller must have transferred ownership of whatever heap ids `body`
    /// carries; [`Task::drop_with`] releases them.
    pub fn spawn(&mut self, body: TaskBody) -> TaskId {
        let task_id = TaskId::new(self.next_task_id);
        self.next_task_id += 1;
        self.tasks.insert(task_id, Task::new(task_id, body));
        self.ready_queue.push_back(task_id);
        task_id
    }

    /// Gets the next ready task from the queue.
    pub fn next_ready_task(&mut self) -> Option<TaskId> {
        self.ready_queue.pop_front()
    }

    /// Replaces a task's state, properly releasing any heap references owned
    /// by the previous state.
    pub fn set_state(&mut self, task_id: TaskId, new_state: TaskState, heap: &mut Heap) {
        let task = self.get_task_mut(task_id);
        let old_state = mem::replace(&mut task.state, new_state);
        old_state.drop_with(heap);
    }

    /// Queues a task whose state the caller has already set.
    pub fn push_ready(&mut self, task_id: TaskId) {
        self.ready_queue.push_back(task_id);
    }

    /// Adds a task back to the ready queue.
    pub fn make_ready(&mut self, task_id: TaskId, heap: &mut Heap) {
        self.set_state(task_id, TaskState::Ready, heap);
        self.ready_queue.push_back(task_id);
    }

    /// Sets the current task.
    pub fn set_current_task(&mut self, task_id: Option<TaskId>) {
        self.current_task = task_id;
    }

    /// Removes a task and releases everything it held.
    ///
    /// Idempotent: finalization walks can name a task that is already gone.
    pub fn drop_task(&mut self, task_id: TaskId, heap: &mut HeapReader<'_>) {
        let Some(task) = self.tasks.remove(&task_id) else {
            return;
        };
        if self.current_task == Some(task_id) {
            self.current_task = None;
        }
        self.ready_queue.retain(|&id| id != task_id);
        task.drop_with(heap);
    }

    /// Returns true if a task with `task_id` currently exists.
    #[inline]
    pub fn has_task(&self, task_id: TaskId) -> bool {
        self.tasks.contains_key(&task_id)
    }

    /// The current reading of the scheduler's clock, in nanoseconds.
    #[inline]
    pub fn now(&self) -> u64 {
        self.now
    }

    /// Arms `timer` `delay_nanos` from now. The caller transfers ownership of
    /// every reference the timer carries.
    pub fn arm_timer(&mut self, delay_nanos: u64, future: HeapId, value: Value, cancel_target: Option<HeapId>) {
        let timer = Timer {
            deadline: self.now.saturating_add(delay_nanos),
            seq: self.next_timer_seq,
            future,
            value,
            cancel_target,
        };
        self.next_timer_seq += 1;
        let at = self
            .timers
            .partition_point(|t| (t.deadline, t.seq) < (timer.deadline, timer.seq));
        self.timers.insert(at, timer);
    }

    /// Disarms every timer aimed at `future`, handing the caller everything
    /// they owned.
    pub fn disarm_timers(&mut self, future: HeapId) -> Vec<Timer> {
        let mut released = Vec::new();
        let mut kept = Vec::new();
        for timer in mem::take(&mut self.timers) {
            if timer.future == future {
                released.push(timer);
            } else {
                kept.push(timer);
            }
        }
        self.timers = kept;
        released
    }

    /// Takes every timer whose deadline has arrived, transferring what they own.
    pub fn take_due_timers(&mut self) -> Vec<Timer> {
        let due = self.timers.partition_point(|timer| timer.deadline <= self.now);
        self.timers.drain(..due).collect()
    }

    /// Moves the clock to the earliest armed deadline, if there is one.
    ///
    /// Only called once nothing else can run, which is what keeps the clock
    /// from measuring anything but the sandbox's own idleness.
    pub fn advance_to_next_deadline(&mut self) -> bool {
        match self.timers.first() {
            Some(timer) => {
                self.now = self.now.max(timer.deadline);
                true
            }
            None => false,
        }
    }

    /// Every heap id the loop still owns.
    ///
    /// These are live because the loop can still act on them, not because
    /// something forgot to release them: a task the scheduler holds is one it
    /// can still run or wake, exactly as an imported module is live because
    /// the module cache holds it. Mirrors [`Self::cleanup`], which releases
    /// this same set.
    #[cfg(feature = "ref-count-return")]
    pub fn root_ids(&self) -> Vec<HeapId> {
        let mut roots = Vec::new();
        let push = &mut |id: HeapId| roots.push(id);
        for future_id in self.pending_externals.values() {
            push(*future_id);
        }
        for timer in &self.timers {
            push(timer.future);
            push_value(&timer.value, push);
            push_opt(timer.cancel_target, push);
        }
        for task in self.tasks.values() {
            task.owned_ids(push);
        }
        roots
    }

    /// Cleans up every scheduler-held reference: pending externals, armed
    /// timers, and every remaining task.
    pub fn cleanup(&mut self, heap: &mut HeapReader<'_>) {
        for (_, future_id) in mem::take(&mut self.pending_externals) {
            heap.dec_ref(future_id);
        }
        for timer in mem::take(&mut self.timers) {
            timer.drop_with(heap);
        }
        let task_ids: Vec<TaskId> = self.tasks.keys().copied().collect();
        for task_id in task_ids {
            self.drop_task(task_id, heap);
        }
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

/// Pushes `value`'s heap id, if it has one.
#[cfg(feature = "ref-count-return")]
fn push_value(value: &Value, push: &mut impl FnMut(HeapId)) {
    if let Value::Ref(id) = value {
        push(*id);
    }
}

/// Pushes `id`, if there is one.
#[cfg(feature = "ref-count-return")]
fn push_opt(id: Option<HeapId>, push: &mut impl FnMut(HeapId)) {
    if let Some(id) = id {
        push(id);
    }
}
