//! Driving generators: splicing a suspended frame back onto the VM's stacks,
//! running it, and draining it back out at the next `yield`.
//!
//! # Why the VM stacks and not a nested interpreter
//!
//! A generator step runs on the *real* frame stack, above whoever asked for the
//! value. That is what lets a generator body suspend to the host (an OS call, an
//! external function) mid-step: the VM snapshot captures the generator's frame
//! like any other, and `resume` continues inside it. The bookkeeping needed to
//! find the way back out lives in [`GenActivation`], one per in-flight step, and
//! is part of the snapshot for the same reason.
//!
//! [`ResumeMode`] records who asked, and so what to do with a yielded value and
//! with the eventual return. Three of the four modes never leave the run loop.
//! The fourth, [`ResumeMode::Native`], serves Rust-side consumers (`list(gen)`,
//! `sum(gen)`, `next(gen)`, ...) which hold a Rust stack frame across the step
//! and therefore cannot suspend to the host; it re-enters `run()` and unwinds
//! back to the consumer at the `yield`.

use std::mem;

use super::{CallFrame, FrameExit, VM, recursion::RunReentryGuard};
use crate::{
    exception_private::{ExcType, ExcTypeExt, RunError, RunResult, SimpleException},
    generator::{Generator, GeneratorState},
    heap::{ContainsHeap, DropWithContext, HeapData, HeapId, HeapReadOutput},
    intern::FunctionId,
    types::PyTrait,
    value::Value,
};

/// Who is driving a generator step, and so where control goes at the `yield`.
///
/// Every mode but [`Self::Native`] pushes the yielded value onto the caller's
/// operand stack and carries straight on in the run loop; they differ only in
/// what the generator's *return* means.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum ResumeMode {
    /// A Rust-side consumer re-entered `run()` for one step. The `yield`
    /// returns `FrameExit::Return(value)` out of that nested loop instead of
    /// pushing; the generator's state tells the consumer which it was.
    Native,
    /// The `ForIter` opcode. Exhaustion pops the iterator and jumps to
    /// `loop_end`, discarding the return value exactly as CPython's `for`
    /// discards `StopIteration.value`.
    ForIter { loop_end: usize },
    /// The `SendIter` opcode of a `yield from`. Exhaustion replaces the
    /// delegate with its return value, which is the `yield from` expression's
    /// value, and jumps to `loop_end`.
    Delegate { loop_end: usize },
    /// `Await` on an async generator's `__anext__`. Exhaustion raises
    /// `StopAsyncIteration`, which ends the enclosing `async for`.
    Await,
}

/// One in-flight generator step.
///
/// Records where the generator's region starts on each of the VM's three
/// stacks so the step can be drained back out, and the caller's
/// `instruction_ip` so an exception escaping the generator is attributed to
/// the instruction that asked for the value rather than to the generator's
/// `yield`.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct GenActivation {
    /// The generator being stepped, OWNED: the activation holds an inc_ref for
    /// the whole step. Awaiting an async generator drops the driver's own
    /// reference as soon as the frame is pushed, so nothing else guarantees the
    /// heap entry outlives the frame running out of it.
    gen_id: HeapId,
    /// `frames.len()` before the generator's frame was pushed.
    frame_base: usize,
    /// `stack.len()` before the generator's region was spliced in.
    stack_base: usize,
    /// `exception_stack.len()` before the generator's entries were spliced in.
    exc_base: usize,
    /// `instruction_ip` of the driving instruction, restored on the way out.
    caller_instruction_ip: usize,
    /// What the yield and the return mean; see [`ResumeMode`].
    mode: ResumeMode,
}

/// What one generator step produced.
pub(crate) enum GeneratorStep {
    /// Paused at a `yield` with this value.
    Yielded(Value),
    /// Ran off the end or hit `return`; the generator is now exhausted.
    Returned(Value),
}

/// What a step feeds back into a paused generator.
pub(crate) enum GeneratorInput {
    /// The value the paused `yield` expression evaluates to. `None` for the
    /// plain `__next__` step.
    Send(Value),
    /// Raised at the suspension point, so the generator's own `except` /
    /// `finally` blocks see it (`throw()`, and `close()` with `GeneratorExit`).
    Throw(RunError),
}

impl VM<'_> {
    /// Runs one generator step from Rust, re-entering the run loop.
    ///
    /// This is the path every synchronous consumer takes — `__next__`, `send`,
    /// `throw`, `close`, and every builtin that drains an iterator. It cannot
    /// suspend to the host: the consumer's Rust frame is live across the step,
    /// so an OS/external call inside the generator body becomes a
    /// `NotImplementedError` (`for` loops and `yield from` use the opcode-level
    /// modes instead, which have no such limit).
    pub(crate) fn generator_step(&mut self, gen_id: HeapId, input: GeneratorInput) -> RunResult<GeneratorStep> {
        match self.generator_state(gen_id) {
            GeneratorState::Running => {
                input.drop_with(self);
                return Err(already_executing(self.is_async_generator(gen_id)));
            }
            GeneratorState::Completed => return self.step_completed(gen_id, input),
            // A generator that never started has no frame to unwind, so a
            // thrown exception (`close()` included) closes it without running
            // a single `finally`, as CPython does.
            GeneratorState::Created if matches!(input, GeneratorInput::Throw(_)) => {
                let GeneratorInput::Throw(error) = input else {
                    unreachable!("matched above")
                };
                self.complete_generator(gen_id, Value::None);
                return Err(error);
            }
            GeneratorState::Created | GeneratorState::Suspended => {}
        }

        if let Err(e) = self.enter_run_reentry() {
            input.drop_with(self);
            return Err(e.into());
        }
        let mut guard = RunReentryGuard::new(self);
        let this = &mut *guard;

        // `throw` into a `yield from` goes to the delegate first, as CPython
        // does, so the inner generator's cleanup runs.
        let input = match this.delegate_throw(gen_id, input) {
            DelegatedThrow::Handled(step) => return Ok(step),
            DelegatedThrow::Resume(input) => input,
        };

        let started = this.generator_state(gen_id) == GeneratorState::Suspended;
        // Rejected before anything moves: a generator that refuses a sent value
        // has not run, and stays exactly as startable as it was.
        if let GeneratorInput::Send(value) = &input
            && let Err(error) = check_send(value, started, this.is_async_generator(gen_id))
        {
            input.drop_with(this);
            return Err(error);
        }

        this.splice_in(gen_id, ResumeMode::Native)?;
        let frame_base = this.frames.len() - 1;

        match input {
            GeneratorInput::Send(value) => this.apply_send(value, started),
            GeneratorInput::Throw(error) => {
                if let Some(error) = this.handle_exception(error) {
                    // Escaped the generator; the boundary hook already drained
                    // its context and marked it Completed.
                    return Err(error);
                }
            }
        }

        match this.run() {
            Ok(FrameExit::Return(value)) => match this.generator_state(gen_id) {
                GeneratorState::Completed => Ok(GeneratorStep::Returned(value)),
                _ => Ok(GeneratorStep::Yielded(value)),
            },
            Ok(exit) => {
                // A host suspension the nested loop cannot preserve. Tear the
                // half-run step down before reporting it.
                this.abandon_step(frame_base);
                Err(this.unsupported_frame_exit("generator", exit))
            }
            Err(error) => {
                this.abandon_step(frame_base);
                Err(error)
            }
        }
    }

    /// Steps a generator from an opcode, leaving it running on the VM stacks.
    ///
    /// Returns `Ok(true)` when the generator was resumed and the run loop
    /// should reload its cached frame, `Ok(false)` when it was already
    /// exhausted and the caller must take its own exhaustion path.
    pub(super) fn generator_resume_op(&mut self, gen_id: HeapId, mode: ResumeMode, sent: Value) -> RunResult<bool> {
        match self.generator_state(gen_id) {
            GeneratorState::Running => {
                sent.drop_with(self);
                Err(already_executing(self.is_async_generator(gen_id)))
            }
            GeneratorState::Completed => {
                sent.drop_with(self);
                Ok(false)
            }
            state => {
                let started = state == GeneratorState::Suspended;
                if let Err(error) = check_send(&sent, started, self.is_async_generator(gen_id)) {
                    sent.drop_with(self);
                    return Err(error);
                }
                self.splice_in(gen_id, mode)?;
                self.apply_send(sent, started);
                Ok(true)
            }
        }
    }

    /// Splices a suspended generator's context onto the VM stacks and pushes
    /// its frame, so the run loop continues inside the generator body.
    fn splice_in(&mut self, gen_id: HeapId, mode: ResumeMode) -> RunResult<()> {
        // Charged before anything moves, so a depth failure needs no rollback.
        if !self.frames.is_empty() {
            self.incr_recursion()?;
        }

        let caller_instruction_ip = self.instruction_ip;
        let call_offset = self.current_offset();
        let stack_base = self.stack.len();
        let exc_base = self.exception_stack.len();
        let frame_base = self.frames.len();

        let HeapReadOutput::Generator(mut generator) = self.heap.read(gen_id) else {
            unreachable!("splice_in called with a non-generator heap id")
        };
        let (func_id, ip, instruction_ip, mut stack, mut exception_stack) = {
            let generator = generator.get_mut(self.heap);
            generator.state = GeneratorState::Running;
            (
                generator.func_id,
                generator.ip,
                generator.instruction_ip,
                mem::take(&mut generator.stack),
                mem::take(&mut generator.exception_stack),
            )
        };
        drop(generator);

        self.stack.append(&mut stack);
        self.exception_stack.append(&mut exception_stack);

        let func = self.interns.get_function(func_id);
        let locals_count = u16::try_from(func.namespace_size).expect("generator namespace size exceeds u16");
        let mut frame = CallFrame::new_function(&func.code, stack_base, locals_count, exc_base, func_id, call_offset);
        frame.ip = ip;
        // A native step must stop unwinding at the generator boundary rather
        // than tearing into the Rust consumer's caller.
        frame.should_return = matches!(mode, ResumeMode::Native);
        self.frames.push(frame);

        self.instruction_ip = instruction_ip;
        self.heap.inc_ref(gen_id);
        self.gen_activations.push(GenActivation {
            gen_id,
            frame_base,
            stack_base,
            exc_base,
            caller_instruction_ip,
            mode,
        });
        Ok(())
    }

    /// Feeds a sent value to the resumed `yield` expression.
    ///
    /// A generator that has not started has no paused `yield` for the value to
    /// become, so it is discarded; [`check_send`] has already established that
    /// it was `None`.
    fn apply_send(&mut self, value: Value, started: bool) {
        if started {
            self.push(value);
        } else {
            value.drop_with(self);
        }
    }

    /// Pauses the running generator at a `yield`, draining its context back
    /// into the heap object and handing `yielded` to whoever drove the step.
    ///
    /// The caller must have synced the frame's `ip` to the instruction after
    /// the `Yield` first; that offset is where the generator resumes.
    pub(super) fn suspend_generator(&mut self, yielded: Value, delegating: bool) -> GeneratorYield {
        let activation = self
            .gen_activations
            .pop()
            .expect("Yield executed outside a generator step");
        debug_assert_eq!(
            activation.frame_base + 1,
            self.frames.len(),
            "Yield must run in the generator's own frame"
        );

        let frame = self.frames.pop().expect("generator frame missing at suspend");
        if !self.frames.is_empty() {
            self.decr_recursion();
        }
        let stack: Vec<Value> = self.stack.drain(activation.stack_base..).collect();
        let exception_stack: Vec<Value> = self.exception_stack.drain(activation.exc_base..).collect();

        let HeapReadOutput::Generator(mut generator) = self.heap.read(activation.gen_id) else {
            unreachable!("generator activation id is not a generator")
        };
        {
            let generator = generator.get_mut(self.heap);
            generator.state = GeneratorState::Suspended;
            generator.ip = frame.ip;
            generator.instruction_ip = self.instruction_ip;
            generator.stack = stack;
            generator.exception_stack = exception_stack;
            generator.delegating = delegating;
        }
        drop(generator);

        self.instruction_ip = activation.caller_instruction_ip;
        self.heap.dec_ref(activation.gen_id);
        match activation.mode {
            ResumeMode::Native => GeneratorYield::ExitNestedRun(yielded),
            ResumeMode::ForIter { .. } | ResumeMode::Delegate { .. } | ResumeMode::Await => {
                self.push(yielded);
                GeneratorYield::Resumed
            }
        }
    }

    /// Ends the running generator at a `return`, dropping its context and
    /// routing the return value per the mode that drove the step.
    ///
    /// Returns `Some(value)` only for a native step, whose consumer receives
    /// the value through the nested `run()`.
    pub(super) fn finish_generator(&mut self, returned: Value) -> RunResult<Option<Value>> {
        let activation = self
            .gen_activations
            .pop()
            .expect("generator return outside a generator step");
        let gen_id = activation.gen_id;
        self.discard_generator_context(&activation);
        self.complete_generator(gen_id, returned);

        let outcome = match activation.mode {
            ResumeMode::Native => Ok(Some(self.take_generator_result(gen_id))),
            ResumeMode::ForIter { loop_end } => {
                let iterator = self.pop();
                iterator.drop_with(self);
                // A `for` loop swallows the `StopIteration` whole, value and
                // all, so nothing downstream should still see it parked.
                let discarded = self.take_generator_result(gen_id);
                discarded.drop_with(self);
                self.current_frame_mut().ip = loop_end;
                Ok(None)
            }
            ResumeMode::Delegate { loop_end } => {
                let delegate = self.pop();
                delegate.drop_with(self);
                let value = self.take_generator_result(gen_id);
                self.push(value);
                self.current_frame_mut().ip = loop_end;
                Ok(None)
            }
            ResumeMode::Await => Err(stop_async_iteration()),
        };
        self.heap.dec_ref(gen_id);
        outcome
    }

    /// Closes the generator whose frame the exception unwinder has just popped.
    ///
    /// An exception escaping a generator body kills the generator, and then
    /// belongs to whoever asked it for a value: the context is discarded and
    /// the error keeps propagating outwards.
    ///
    /// Callers must have established the boundary with
    /// [`at_generator_frame`](Self::at_generator_frame) BEFORE popping the
    /// frame; the frame count no longer identifies it afterwards.
    pub(super) fn close_unwound_generator(&mut self) {
        let activation = self
            .gen_activations
            .pop()
            .expect("close_unwound_generator called off a generator boundary");
        // The frame itself is popped by the unwinder, which drains the operand
        // region with it; the exception stack and the generator's own state
        // are what is left to clean.
        self.exception_stack
            .drain(activation.exc_base..)
            .for_each(|value| value.drop_with(&mut *self.heap));
        let gen_id = activation.gen_id;
        self.complete_generator(gen_id, Value::None);
        self.instruction_ip = activation.caller_instruction_ip;
        self.heap.dec_ref(gen_id);
    }

    /// Tears down a native step that ended without reaching `yield` or
    /// `return` (a host suspension the nested loop cannot preserve, or an
    /// error that unwound past our boundary hook).
    fn abandon_step(&mut self, frame_base: usize) {
        while self.frames.len() > frame_base {
            self.pop_frame();
        }
        let ours = self
            .gen_activations
            .last()
            .is_some_and(|activation| activation.frame_base == frame_base);
        if ours {
            let activation = self.gen_activations.pop().expect("checked above");
            self.discard_generator_context(&activation);
            let gen_id = activation.gen_id;
            self.complete_generator(gen_id, Value::None);
            self.instruction_ip = activation.caller_instruction_ip;
            self.heap.dec_ref(gen_id);
        }
    }

    /// Drops everything a step spliced onto the VM stacks above `activation`.
    fn discard_generator_context(&mut self, activation: &GenActivation) {
        while self.frames.len() > activation.frame_base {
            self.pop_frame();
        }
        self.stack
            .drain(activation.stack_base..)
            .for_each(|value| value.drop_with(&mut *self.heap));
        self.exception_stack
            .drain(activation.exc_base..)
            .for_each(|value| value.drop_with(&mut *self.heap));
    }

    /// Marks a generator exhausted and parks its return value.
    ///
    /// The value stays on the object because a `yield from` can observe the
    /// delegate's exhaustion on a later step than the one that produced it.
    fn complete_generator(&mut self, gen_id: HeapId, returned: Value) {
        let HeapReadOutput::Generator(mut generator) = self.heap.read(gen_id) else {
            unreachable!("generator activation id is not a generator")
        };
        let generator = generator.get_mut(self.heap);
        generator.state = GeneratorState::Completed;
        generator.delegating = false;
        generator.stack = Vec::new();
        generator.exception_stack = Vec::new();
        let previous = mem::replace(&mut generator.result, returned);
        previous.drop_with(&mut *self.heap);
    }

    /// Takes an exhausted generator's parked return value, leaving `None`.
    pub(crate) fn take_generator_result(&mut self, gen_id: HeapId) -> Value {
        let HeapReadOutput::Generator(mut generator) = self.heap.read(gen_id) else {
            unreachable!("take_generator_result called with a non-generator heap id")
        };
        mem::replace(&mut generator.get_mut(self.heap).result, Value::None)
    }

    /// Reports a step on an already-exhausted generator.
    ///
    /// `send`/`__next__` report exhaustion; a thrown exception is raised at the
    /// call site instead, matching CPython's closed-generator behaviour.
    fn step_completed(&mut self, gen_id: HeapId, input: GeneratorInput) -> RunResult<GeneratorStep> {
        match input {
            GeneratorInput::Send(value) => {
                value.drop_with(self);
                Ok(GeneratorStep::Returned(self.take_generator_result(gen_id)))
            }
            GeneratorInput::Throw(error) => Err(error),
        }
    }

    /// Routes a `throw` at a `yield from` into the delegate first.
    ///
    /// CPython gives the inner iterator the exception so its `except`/`finally`
    /// run. A delegate that yields means the outer generator re-yields that
    /// value with no state change of its own; a delegate that returns leaves
    /// the outer to observe exhaustion on its next `SendIter`; a delegate that
    /// raises hands the new exception back for the outer's own handlers.
    fn delegate_throw(&mut self, gen_id: HeapId, input: GeneratorInput) -> DelegatedThrow {
        let GeneratorInput::Throw(error) = input else {
            return DelegatedThrow::Resume(input);
        };
        let Some(delegate_id) = self.resumable_delegate(gen_id) else {
            return DelegatedThrow::Resume(GeneratorInput::Throw(error));
        };

        match self.generator_step(delegate_id, GeneratorInput::Throw(error)) {
            Ok(GeneratorStep::Yielded(value)) => DelegatedThrow::Handled(GeneratorStep::Yielded(value)),
            Ok(GeneratorStep::Returned(value)) => {
                // Park it: the outer's next `SendIter` sees the delegate
                // exhausted and takes this as the `yield from` value.
                self.park_generator_result(delegate_id, value);
                DelegatedThrow::Resume(GeneratorInput::Send(Value::None))
            }
            Err(error) => DelegatedThrow::Resume(GeneratorInput::Throw(error)),
        }
    }

    /// Stores a value as an exhausted generator's parked result.
    pub(crate) fn park_generator_result(&mut self, gen_id: HeapId, value: Value) {
        let HeapReadOutput::Generator(mut generator) = self.heap.read(gen_id) else {
            unreachable!("park_generator_result called with a non-generator heap id")
        };
        let previous = mem::replace(&mut generator.get_mut(self.heap).result, value);
        previous.drop_with(&mut *self.heap);
    }

    /// The generator a paused `yield from` is delegating to, when it is itself
    /// a resumable generator. Other delegates (a list iterator, an exhausted
    /// generator) have no `throw` to forward to.
    fn resumable_delegate(&self, gen_id: HeapId) -> Option<HeapId> {
        let HeapData::Generator(generator) = self.heap.get(gen_id) else {
            unreachable!("resumable_delegate called with a non-generator heap id")
        };
        if !generator.delegating {
            return None;
        }
        let &Value::Ref(delegate_id) = generator.stack.last()? else {
            return None;
        };
        match self.heap.get(delegate_id) {
            HeapData::Generator(delegate) if delegate.is_resumable() => Some(delegate_id),
            _ => None,
        }
    }

    /// One `SendIter` step of a `yield from`.
    ///
    /// Stack on entry is `[..., delegate, sent]`. Returns `Ok(true)` when the
    /// delegate was resumed (the run loop must reload its frame), and
    /// `Ok(false)` when it was already exhausted and the caller should take the
    /// jump with the `yield from` value now on the stack.
    pub(super) fn exec_send_iter(&mut self, loop_end: usize) -> RunResult<bool> {
        let sent = self.pop();
        let Value::Ref(delegate_id) = *self.peek() else {
            sent.drop_with(self);
            let name = self.peek().py_type_name(self);
            return Err(ExcType::type_error_not_iterator(&name));
        };

        if self.is_sync_generator(delegate_id) {
            if self.generator_resume_op(delegate_id, ResumeMode::Delegate { loop_end }, sent)? {
                return Ok(true);
            }
            // Already exhausted, so its parked return value is the `yield from`
            // expression's value.
            let delegate = self.pop();
            delegate.drop_with(self);
            let value = self.take_generator_result(delegate_id);
            self.push(value);
            return Ok(false);
        }

        // A plain iterator has no `send`; only the implicit `None` of the
        // first step and of every re-entry through `__next__` is accepted.
        if !matches!(sent, Value::None) {
            let name = self.peek().py_type_name(self);
            sent.drop_with(self);
            return Err(ExcType::attribute_error(&name, "send"));
        }
        sent.drop_with(self);

        if let Some(value) = self.heap.read(delegate_id).py_next(Some(delegate_id), self)? {
            self.push(value);
            Ok(true)
        } else {
            let delegate = self.pop();
            delegate.drop_with(self);
            // A non-generator iterator has no return value, so the `yield from`
            // evaluates to `None`.
            self.push(Value::None);
            Ok(false)
        }
    }

    /// The `EndAsyncFor` handler body: `true` when the pending exception was a
    /// `StopAsyncIteration` and the loop should end, `false` when it must be
    /// re-raised.
    ///
    /// Either way the async iterator and the handler's exception copy are
    /// dropped, since neither path returns to the loop body.
    pub(super) fn exec_end_async_for(&mut self) -> bool {
        let exception = self.pop();
        let ends_loop = matches!(
            &exception,
            Value::Ref(id) if matches!(
                self.heap.get(*id),
                HeapData::Exception(exc) if exc.exc_type() == ExcType::StopAsyncIteration
            )
        );
        exception.drop_with(self);
        let aiter = self.pop();
        aiter.drop_with(self);
        if ends_loop && let Some(active) = self.exception_stack.pop() {
            active.drop_with(self);
        }
        ends_loop
    }

    /// Whether the current frame is a running generator's own frame.
    pub(super) fn at_generator_frame(&self) -> bool {
        self.gen_activations
            .last()
            .is_some_and(|activation| activation.frame_base + 1 == self.frames.len())
    }

    /// Whether `id` is a generator that the synchronous iteration protocol
    /// drives. Async generators are stepped through `__anext__` instead.
    pub(super) fn is_sync_generator(&self, id: HeapId) -> bool {
        matches!(self.heap.get(id), HeapData::Generator(generator) if !generator.is_async)
    }

    /// Reads a generator's lifecycle phase.
    pub(super) fn generator_state(&self, gen_id: HeapId) -> GeneratorState {
        match self.heap.get(gen_id) {
            HeapData::Generator(generator) => generator.state,
            _ => unreachable!("generator_state called with a non-generator heap id"),
        }
    }

    /// Whether this is an `async def` generator, which reports itself and
    /// raises differently from a plain one.
    pub(crate) fn is_async_generator(&self, gen_id: HeapId) -> bool {
        match self.heap.get(gen_id) {
            HeapData::Generator(generator) => generator.is_async,
            _ => unreachable!("is_async_generator called with a non-generator heap id"),
        }
    }
}

/// Rejects a value sent into a generator that has not started, which has no
/// paused `yield` for it to become. Checked before the step commits anything,
/// so the generator is left exactly as startable as it was.
fn check_send(value: &Value, started: bool, is_async: bool) -> RunResult<()> {
    if started || matches!(value, Value::None) {
        Ok(())
    } else {
        let kind = if is_async { "async generator" } else { "generator" };
        Err(ExcType::type_error(format!(
            "can't send non-None value to a just-started {kind}"
        )))
    }
}

/// What the `Yield` opcode should do once the generator has been drained out.
pub(super) enum GeneratorYield {
    /// The value went onto the driving frame's operand stack; carry on.
    Resumed,
    /// A native step: unwind out of the nested `run()` with this value.
    ExitNestedRun(Value),
}

/// Outcome of forwarding a `throw` into a `yield from` delegate.
enum DelegatedThrow {
    /// The delegate produced the step's result; the outer never ran.
    Handled(GeneratorStep),
    /// Resume the outer generator with this input.
    Resume(GeneratorInput),
}

impl<C: ContainsHeap> DropWithContext<C> for GeneratorInput {
    fn drop_with(self, heap: &mut C) {
        match self {
            Self::Send(value) => value.drop_with(heap),
            Self::Throw(_) => {}
        }
    }
}

/// `ValueError: generator already executing` — a step re-entered from inside
/// its own body.
fn already_executing(is_async: bool) -> RunError {
    let kind = if is_async { "async generator" } else { "generator" };
    SimpleException::new_msg(ExcType::ValueError, format!("{kind} already executing")).into()
}

/// `StopAsyncIteration`, which ends an `async for`.
pub(crate) fn stop_async_iteration() -> RunError {
    SimpleException::new_none(ExcType::StopAsyncIteration).into()
}

/// Builds the `StopIteration` a returning generator raises for `send`/`next`,
/// carrying the return value the way CPython's `str(exc)` renders it.
///
/// Monty exceptions hold a message rather than an argument tuple, so the value
/// reaches Python as `str(exc)` and not as `exc.value`; see
/// `limitations/iter.md`.
pub(crate) fn stop_iteration_with(value: Value, vm: &mut VM<'_>) -> RunError {
    if matches!(value, Value::None) {
        value.drop_with(vm);
        return ExcType::stop_iteration();
    }
    let rendered = value.py_str(vm).and_then(|text| {
        let owned = text.to_str(vm)?.to_owned();
        text.drop_with(vm);
        Ok(owned)
    });
    value.drop_with(vm);
    match rendered {
        Ok(message) => SimpleException::new_msg(ExcType::StopIteration, message).into(),
        Err(error) => error,
    }
}

/// Creates a generator over `namespace`, the frame region the caller built by
/// binding the call's arguments.
pub(crate) fn allocate_generator(
    func_id: FunctionId,
    namespace: Vec<Value>,
    is_async: bool,
    vm: &mut VM<'_>,
) -> Value {
    let generator = Generator::new(func_id, namespace, is_async);
    Value::Ref(vm.heap.allocate(HeapData::Generator(Box::new(generator))))
}
