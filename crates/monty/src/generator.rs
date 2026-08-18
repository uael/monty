//! The suspended-frame object behind `yield`.
//!
//! A generator owns the whole execution context of one paused invocation of a
//! generator function: its locals-and-operands region, the exceptions it was
//! handling, and where to resume. The VM splices that context back onto its own
//! stacks to run a step and drains it back out at the next `yield` (see
//! [`crate::bytecode::vm::generator`]).
//!
//! Only ONE frame is ever saved, because `yield` is lexically part of the
//! generator's own body: any function it called has returned by the time the
//! `yield` executes. That is why there is an `ip` here and not a frame vector.

use std::iter;

use crate::{
    heap::{ContainsHeap, DropWithContext, HeapId, HeapItem},
    intern::FunctionId,
    types::Type,
    value::Value,
};

/// Lifecycle of a [`Generator`], mirroring CPython's `gi_frame_state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum GeneratorState {
    /// Created by calling the generator function; the body has not run yet.
    /// `send()` with a non-`None` value is rejected in this state.
    Created,
    /// Paused at a `yield`. [`Generator::stack`] holds the frame's region.
    Suspended,
    /// Currently executing: the context lives on the VM's stacks, not here.
    /// Re-entering a generator in this state raises `ValueError`.
    Running,
    /// Returned, raised, or was closed. Further steps report exhaustion.
    Completed,
}

/// A paused generator-function invocation.
///
/// Created by calling a `def` whose body contains `yield`; the arguments are
/// bound immediately (so a signature error raises at the call, as in CPython)
/// and land in [`Self::stack`] as the frame's locals region, with the body
/// starting at `ip == 0`.
///
/// # Stack layout
///
/// `stack` is exactly what the VM frame owns: `[locals..][operands..]`, with
/// slot indices relative to 0. The VM rebases them by the splice offset on
/// resume and back to 0 on suspend, so a generator never depends on where it
/// last ran.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Generator {
    /// Function whose body this generator runs.
    pub func_id: FunctionId,
    /// Lifecycle phase; see [`GeneratorState`].
    pub state: GeneratorState,
    /// Bytecode offset to resume at, `0` before the first step.
    pub ip: usize,
    /// `instruction_ip` at the suspension point, restored on resume so an
    /// exception thrown in finds the handler region covering the `yield`.
    pub instruction_ip: usize,
    /// The frame's stack region (locals then operands), rebased to 0.
    pub stack: Vec<Value>,
    /// Exceptions being handled inside the generator when it paused, i.e. a
    /// `yield` inside an `except` block. Rebased to 0 like `stack`.
    pub exception_stack: Vec<Value>,
    /// Paused inside `yield from`: the delegate is the top of [`Self::stack`],
    /// since `Yield` popped the value it was re-yielding. Drives `throw`/`close`
    /// delegation.
    pub delegating: bool,
    /// The value `return` handed back, parked here once the generator is
    /// `Completed`. It has to outlive the step that produced it because a
    /// `yield from` can observe its delegate's exhaustion on a later step,
    /// and that value is the `yield from` expression's own value.
    pub result: Value,
    /// `async def` containing `yield`: stepped through `__anext__`/`await`
    /// rather than `__next__`, and finishing raises `StopAsyncIteration`.
    pub is_async: bool,
}

impl Generator {
    /// Creates an unstarted generator over `namespace`, the bound-argument
    /// frame region built by the caller.
    pub fn new(func_id: FunctionId, namespace: Vec<Value>, is_async: bool) -> Self {
        Self {
            func_id,
            state: GeneratorState::Created,
            ip: 0,
            instruction_ip: 0,
            stack: namespace,
            exception_stack: Vec::new(),
            delegating: false,
            result: Value::None,
            is_async,
        }
    }

    /// Whether a step would run bytecode rather than report exhaustion.
    pub fn is_resumable(&self) -> bool {
        matches!(self.state, GeneratorState::Created | GeneratorState::Suspended)
    }

    /// `generator` or `async_generator`, the two CPython types a `yield` body
    /// produces depending on whether the `def` was `async`.
    pub fn py_type(&self) -> Type {
        if self.is_async {
            Type::AsyncGenerator
        } else {
            Type::Generator
        }
    }

    /// Invokes `on_child` for each heap id this generator owns (GC trace hook).
    ///
    /// A generator's frame can hold the only reference to a value, and can even
    /// hold a reference back to the generator itself (`g = gen(); g.send(g)`),
    /// so under-tracing here leaks or frees a live object.
    pub fn for_each_child_id(&self, mut on_child: impl FnMut(HeapId)) {
        for value in self
            .stack
            .iter()
            .chain(&self.exception_stack)
            .chain(iter::once(&self.result))
        {
            if let Value::Ref(id) = value {
                on_child(*id);
            }
        }
    }
}

impl HeapItem for Generator {
    fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        for value in self.stack.iter_mut().chain(&mut self.exception_stack) {
            value.py_dec_ref_ids(stack);
        }
        self.result.py_dec_ref_ids(stack);
    }
}

/// Releases a generator context that is being discarded rather than resumed,
/// e.g. when `close()` finds nothing left to run.
impl<C: ContainsHeap> DropWithContext<C> for Generator {
    fn drop_with(self, heap: &mut C) {
        self.stack.drop_with(heap);
        self.exception_stack.drop_with(heap);
        self.result.drop_with(heap);
    }
}
