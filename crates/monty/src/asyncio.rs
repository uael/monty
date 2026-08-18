//! The objects `asyncio` is made of: coroutines, futures, tasks, and the
//! combinators that watch several of them at once.
//!
//! There is one future type. A plain `asyncio.Future` is settled by whoever
//! holds it, an `asyncio.Task` is settled by the coroutine the scheduler drives
//! for it, and a future standing for an external call is settled by the host,
//! but all three are the same object, so `await`, `gather`, `cancel` and the
//! rest need no special case per flavour.
//!
//! Waiting is registered on the future itself: every awaiting task and every
//! combinator slot is a [`Waiter`] in the future's own list, and settling walks
//! that list once. Nothing else in the VM knows who is waiting on what.

use crate::{
    exception_private::RunError,
    heap::{ContainsHeap, DropWithContext, HeapId},
    intern::FunctionId,
    namespace::ScopeId,
    value::{EitherStr, Value},
};

/// Unique identifier for external function calls.
///
/// Sequential integers allocated by the scheduler. Used to correlate
/// external function calls with their results when the host resolves them.
/// The counter always increments, even for sync resolution, to keep IDs unique.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(crate) struct CallId(u32);

impl CallId {
    /// Creates a new CallId from a raw value.
    #[inline]
    pub fn new(id: u32) -> Self {
        Self(id)
    }

    /// Returns the raw u32 value.
    #[inline]
    pub fn raw(self) -> u32 {
        self.0
    }
}

/// Unique identifier for an async task.
///
/// Sequential integers allocated by the scheduler. Task 0 is always the main task
/// which uses the VM's stack/frames directly. Spawned tasks (1+) store their own context,
/// hence `TaskId::default()` is the main task.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(crate) struct TaskId(u32);

impl TaskId {
    /// Creates a new TaskId from a raw value.
    #[inline]
    pub fn new(id: u32) -> Self {
        Self(id)
    }

    /// Returns true if this is the main task (task 0).
    #[inline]
    pub fn is_main(self) -> bool {
        self.0 == 0
    }

    /// The raw counter value, which is also the number in a task's default name.
    #[inline]
    pub fn raw(self) -> u32 {
        self.0
    }
}

/// Coroutine execution state (single-shot semantics).
///
/// Coroutines in Monty follow single-shot semantics - they can only be awaited once.
/// This differs from Python generators which can be resumed multiple times.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum CoroutineState {
    /// Coroutine has been created but not yet awaited.
    New,
    /// Coroutine is currently executing (has been awaited).
    Running,
    /// Coroutine has finished execution.
    Completed,
}

/// What one coroutine may spend, and whether it has already overrun.
///
/// A session's own `max_steps` bounds everything it will ever run and is
/// physics: nothing inside may catch it. This bounds one piece of work the host
/// put into a session, and the point of it is that the code which started that
/// work sees the overrun, so the first one is an ordinary exception raised in
/// the coroutine's frames. A second is not: source that catches the first and
/// carries on is no longer bounded by anything, so once `spent` is set the
/// overrun is raised uncatchably and the run ends.
///
/// `spent` is counted at the dispatch checkpoint's granularity, the same one
/// the session's own `max_steps` is counted at, so a budget is spent in whole
/// intervals.
///
/// Carried behind a box wherever it is optional, so a frame that has no budget,
/// which is nearly every frame, pays one pointer for the possibility rather than
/// three words.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct StepBudget {
    /// Instructions this coroutine's own body may execute.
    pub limit: u64,
    /// Instructions it has executed.
    pub spent: u64,
    /// Whether the overrun has already been raised once, catchably.
    pub raised: bool,
}

impl StepBudget {
    /// A budget of `limit` instructions, nothing spent.
    pub fn new(limit: u64) -> Self {
        Self {
            limit,
            spent: 0,
            raised: false,
        }
    }
}

/// A coroutine object representing an async function call result.
///
/// Created when an `async def` function is called. Argument binding happens at call time;
/// awaiting the coroutine starts execution. Coroutines use single-shot semantics -
/// they can only be awaited once.
///
/// # Namespace Layout
///
/// The `namespace` vector is pre-sized to match the function's namespace size and contains:
/// ```text
/// [params...][cell_vars...][free_vars...][locals...]
/// ```
/// - Parameter slots are filled with bound argument values at call time
/// - Cell/free var slots contain `Value::Ref` to captured cells
/// - Local slots start as `Value::Undefined`
///
/// When the coroutine is awaited, these values are pushed onto the VM's stack
/// as inline locals, and a new frame is pushed to execute the async function body.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Coroutine {
    /// The async function to execute.
    pub func_id: FunctionId,
    /// The global namespace the `async def` ran in, which its body resolves
    /// globals against whichever task ends up driving it.
    #[serde(default)]
    pub scope: ScopeId,
    /// Pre-bound namespace values (sized to function namespace).
    /// Contains bound parameters, captured cells, and uninitialized locals.
    pub namespace: Vec<Value>,
    /// Current execution state.
    pub state: CoroutineState,
    /// What this coroutine's own body may spend, when a host gave it a budget
    /// of its own. Carried here because the coroutine is what the host holds
    /// before anything runs; the frame it starts takes it from here.
    #[serde(default)]
    pub budget: Option<Box<StepBudget>>,
}

impl Coroutine {
    /// Creates a new coroutine for an async function call.
    ///
    /// # Arguments
    /// * `func_id` - The async function to execute
    /// * `namespace` - Pre-bound namespace with parameters and captured variables
    pub fn new(func_id: FunctionId, scope: ScopeId, namespace: Vec<Value>) -> Self {
        Self {
            func_id,
            scope,
            namespace,
            state: CoroutineState::New,
            budget: None,
        }
    }

    /// The same coroutine, bounded to `limit` instructions of its own.
    pub fn with_budget(mut self, limit: Option<u64>) -> Self {
        self.budget = limit.map(|limit| Box::new(StepBudget::new(limit)));
        self
    }
}

/// What a [`Future`] is to Python, which is only a matter of its name and of
/// which extra methods answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum FutureKind {
    /// `asyncio.Future`: settled by whoever holds it, or by the host when it
    /// stands for an external call.
    Future,
    /// `asyncio.Task`: settled by the coroutine the scheduler drives for it.
    Task,
}

/// Where a [`Future`] is in its one-way trip from pending to settled.
///
/// `Cancelled` is kept apart from `Failed` because `cancelled()` has to answer
/// differently, even though both raise at every awaiting site.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum FutureState {
    /// Nobody has settled it yet. `waiters` is only meaningful in this state.
    Pending,
    /// Settled with a value. Owned, and cloned to each awaiting site.
    Finished(Value),
    /// Settled with an exception, replayed to each awaiting site.
    Failed(RunError),
    /// Cancelled. The error is the `CancelledError` every awaiting site gets.
    Cancelled(RunError),
}

impl FutureState {
    /// Whether the future has settled, and so can no longer be set or cancelled.
    #[inline]
    pub fn is_settled(&self) -> bool {
        !matches!(self, Self::Pending)
    }
}

/// Who a settling future has to tell.
///
/// Registered in the future's own `waiters` list at the moment the wait starts,
/// and walked exactly once when it settles.
///
/// Not `Copy` / `Clone` on purpose: `Slot` owns an inc_ref on its combinator, so
/// every duplication has to go through the heap and every discard through
/// [`DropWithContext`].
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum Waiter {
    /// A parked task: the value lands on its operand stack, or the error is
    /// raised in its frame. `TaskId` is a scheduler-side number, not a heap
    /// reference, so this variant owns nothing.
    Task(TaskId),
    /// Slot `index` of the combinator at `owner`, which decides for itself what
    /// a child's outcome means. Owns an inc_ref on `owner` so the combinator
    /// outlives every child still reporting to it.
    Slot { owner: HeapId, index: u32 },
}

impl<C: ContainsHeap> DropWithContext<C> for Waiter {
    fn drop_with(self, heap: &mut C) {
        if let Self::Slot { owner, .. } = self {
            heap.heap_mut().dec_ref(owner);
        }
    }
}

/// The scheduler-side half of a task: what it runs, and under which id.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct TaskRun {
    /// The coroutine being driven, owned by the future. `None` for the main
    /// task, which runs module-level code rather than a coroutine.
    pub coroutine: Option<HeapId>,
    /// The scheduler task holding this coroutine's frames.
    pub task_id: TaskId,
}

/// An `asyncio.Future`, and so also an `asyncio.Task`.
///
/// A future settles once. Until it does, `waiters` records everyone who will be
/// told and `callbacks` records the Python callables the loop will run
/// afterwards; after it settles both are empty and the outcome is replayed to
/// every later `await`, exactly as CPython caches a result.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Future {
    /// Which of the two names this object answers to.
    pub kind: FutureKind,
    /// The outcome, or `Pending`.
    pub state: FutureState,
    /// Everyone to tell when it settles, in registration order.
    pub waiters: Vec<Waiter>,
    /// `add_done_callback` callables, owned, run by the loop after settling.
    pub callbacks: Vec<Value>,
    /// The external call this future stands for, when the host settles it.
    pub call_id: Option<CallId>,
    /// Set for a `Task`: the coroutine it drives and the id it runs under.
    pub run: Option<TaskRun>,
    /// How many `cancel()` calls this task has outstanding, which is what
    /// `Task.cancelling()` reports.
    pub cancelling: u32,
    /// A cancellation asked for while the task was running rather than parked.
    /// It takes effect at the task's next suspension, as in CPython, so a
    /// `cancel()` cannot interrupt a stretch of straight-line code.
    pub must_cancel: Option<Value>,
    /// `Task.get_name()`, defaulting to `Task-<n>` on first read.
    pub name: Option<EitherStr>,
}

impl Future {
    /// A pending future nobody is waiting on yet.
    pub(crate) fn new(kind: FutureKind, call_id: Option<CallId>) -> Self {
        Self {
            kind,
            state: FutureState::Pending,
            waiters: Vec::new(),
            callbacks: Vec::new(),
            call_id,
            run: None,
            cancelling: 0,
            must_cancel: None,
            name: None,
        }
    }

    /// Pushes every heap id this future owns.
    ///
    /// The one place they are listed: both the GC's child walk and the
    /// destructor's reference release go through it, so neither can drift.
    pub fn owned_ids(&self, push: &mut impl FnMut(HeapId)) {
        if let FutureState::Finished(Value::Ref(id)) = &self.state {
            push(*id);
        }
        for waiter in &self.waiters {
            if let Waiter::Slot { owner, .. } = waiter {
                push(*owner);
            }
        }
        for callback in &self.callbacks {
            if let Value::Ref(id) = callback {
                push(*id);
            }
        }
        if let Some(coroutine) = self.run.as_ref().and_then(|run| run.coroutine) {
            push(coroutine);
        }
        if let Some(Value::Ref(id)) = &self.must_cancel {
            push(*id);
        }
    }
}

/// What a [`Combinator`] is watching its children for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum CombinatorKind {
    /// `asyncio.gather`. With `return_exceptions` the failures become results;
    /// without it the first failure settles the output and the siblings are
    /// left running, as in CPython.
    Gather { return_exceptions: bool },
    /// `asyncio.wait`, which settles with `(done, pending)` sets and never
    /// raises what a child raised.
    Wait { return_when: ReturnWhen },
    /// `asyncio.as_completed`, which hands each child out in settling order.
    AsCompleted,
    /// `asyncio.TaskGroup`, whose `__aexit__` waits for every child and
    /// cancels the rest as soon as one fails.
    TaskGroup,
}

/// When [`CombinatorKind::Wait`] stops waiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum ReturnWhen {
    FirstCompleted,
    FirstException,
    AllCompleted,
}

/// Several futures watched as one.
///
/// The combinator registers a [`Waiter::Slot`] on each child and settles its
/// own `output` future when its rule says so. It is not itself awaitable:
/// callers await `output`, which keeps one settling path for every awaitable.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Combinator {
    /// The rule that decides when `output` settles.
    pub kind: CombinatorKind,
    /// The futures being watched, in the order they were given. Owned.
    pub children: Vec<HeapId>,
    /// Per-child outcome, filled as each settles. Owned.
    pub results: Vec<Option<Value>>,
    /// Children that have not settled yet.
    pub pending: usize,
    /// The future this combinator settles. Owned.
    pub output: HeapId,
    /// `as_completed` only: children that have settled and not yet been handed
    /// out, oldest first.
    pub finished: Vec<HeapId>,
    /// `as_completed` only: the future a waiting `__anext__` is parked on.
    pub next_wait: Option<HeapId>,
    /// `TaskGroup` only: the first error a child raised, which `__aexit__`
    /// re-raises once every sibling has stopped.
    pub error: Option<RunError>,
}

impl Combinator {
    /// A combinator over `children`, settling `output`, with nothing reported yet.
    pub fn new(kind: CombinatorKind, children: Vec<HeapId>, output: HeapId) -> Self {
        let pending = children.len();
        Self {
            kind,
            results: (0..children.len()).map(|_| None).collect(),
            children,
            pending,
            output,
            finished: Vec::new(),
            next_wait: None,
            error: None,
        }
    }

    /// Pushes every heap id this combinator owns; see [`Future::owned_ids`].
    pub fn owned_ids(&self, push: &mut impl FnMut(HeapId)) {
        for child in &self.children {
            push(*child);
        }
        for result in self.results.iter().flatten() {
            if let Value::Ref(id) = result {
                push(*id);
            }
        }
        push(self.output);
        for done in &self.finished {
            push(*done);
        }
        if let Some(next) = self.next_wait {
            push(next);
        }
    }
}
