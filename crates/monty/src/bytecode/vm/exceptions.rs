//! Exception handling helpers for the VM.

use std::{
    fmt::{self, Write},
    mem::ManuallyDrop,
};

use super::VM;
use crate::{
    args::ArgValues,
    builtins::Builtins,
    defer_drop,
    exception_private::{
        ExcType, ExcTypeExt, ExceptionObject, ExceptionRaise, RawStackFrame, RunError, RunResult, SimpleException,
        exception_message,
    },
    expressions::CmpOperator,
    heap::{DropGuard, DropWithContext, HeapData, HeapId, HeapReadOutput},
    intern::{StaticStrings, StringId},
    types::{
        LazyHeapSet, PyTrait, Type, allocate_string, class_is_subclass,
        instance::{class_defines, class_name, instance_args, instance_class_id, instance_exc_base, instance_str},
    },
    value::Value,
};

/// Takes the traceback chain off an error, leaving it without one.
///
/// `None` for an internal error, which carries no Python-level frame.
fn take_error_frame(error: &mut RunError) -> Option<RawStackFrame> {
    match error {
        RunError::Exc(raise) | RunError::UncatchableExc(raise) => raise.frame.take(),
        RunError::Internal(_) => None,
    }
}

/// What the operand of a `raise` turned out to be, classified before the
/// operand's own reference is consumed.
enum RaiseOperand {
    /// A builtin exception class, to instantiate with no arguments.
    BuiltinType(ExcType),
    /// A sandbox-defined exception class, likewise.
    ExceptionClass,
    /// Anything else, raised (or rejected) as-is.
    Object,
}

/// Result of handling an exception until execution can resume, it escapes, or
/// it needs to be propagated in a waiting task.
enum ExceptionHandlingResult {
    /// Execution can resume without propagating an error to the caller.
    Caught,
    /// The error should be returned to the VM caller.
    Unhandled(RunError),
    /// The error should be handled in the waiting task.
    PropagateToWaiter(RunError),
}

impl VM<'_> {
    /// Returns the current function name, or `<module>` outside a function.
    /// The empty-stack fallback keeps traceback generation total after async
    /// paths drain their frames.
    fn current_frame_name(&self) -> StringId {
        match self.frames.last() {
            Some(frame) => match frame.function_id {
                Some(func_id) => self.interns.get_function(func_id).name.name_id,
                None => StaticStrings::Module.into(),
            },
            None => StaticStrings::Module.into(),
        }
    }

    /// Creates a `RawStackFrame` for the current execution point.
    ///
    /// Used when raising exceptions to capture traceback information.
    fn make_stack_frame(&self) -> RawStackFrame {
        RawStackFrame::new(
            self.current_position().unwrap_or_default(),
            self.current_frame_name(),
            None,
        )
    }

    /// Attaches initial frame information to an error if it doesn't have any.
    ///
    /// Only sets the innermost frame if the exception doesn't already have one.
    /// Caller frames are added separately during exception propagation.
    ///
    /// Uses the `hide_caret` flag from `ExceptionRaise` to determine whether to show
    /// the caret marker in the traceback. This flag is set by error creators that know
    /// whether CPython would show a caret for this specific error type.
    fn attach_frame_to_error(&self, error: RunError) -> RunError {
        match error {
            RunError::Exc(mut exc) => {
                if exc.frame.is_none() {
                    let mut frame = self.make_stack_frame();
                    // Use the hide_caret flag from the error (set by error creators)
                    frame.hide_caret = exc.hide_caret;
                    exc.frame = Some(frame);
                }
                RunError::Exc(exc)
            }
            RunError::UncatchableExc(mut exc) => {
                if exc.frame.is_none() {
                    let mut frame = self.make_stack_frame();
                    frame.hide_caret = exc.hide_caret;
                    exc.frame = Some(frame);
                }
                RunError::UncatchableExc(exc)
            }
            RunError::Internal(_) => error,
        }
    }

    /// Creates a RunError from a Value that should be an exception.
    ///
    /// Borrows the value so callers that already own one can keep it — the
    /// `raise`/`Reraise` paths reuse it as the raised object itself.
    /// The `is_raise` flag indicates if this is from a `raise` statement (hide caret).
    pub(crate) fn make_exception(&mut self, exc_value: &Value, is_raise: bool) -> RunError {
        let simple_exc = match exc_value {
            Value::Ref(heap_id) => match self.heap.get(*heap_id) {
                // Exception instance on heap
                HeapData::Exception(exc) => exc.summary().clone(),
                // An instance of a sandbox-defined exception class: the summary
                // records the real class name, with the nearest builtin ancestor
                // as `exc_type` so `except ValueError:` and the host bindings
                // still see a class they understand.
                HeapData::Instance(_) if instance_exc_base(*heap_id, self).is_some() => {
                    self.user_exception_summary(*heap_id)
                }
                // Not an exception type
                _ => SimpleException::new_msg(ExcType::TypeError, "exceptions must derive from BaseException"),
            },
            // Exception type (e.g., `raise ValueError` instead of `raise ValueError()`)
            // Instantiate with no message
            Value::Builtin(Builtins::ExcType(exc_type)) => SimpleException::new_none(*exc_type),
            // Invalid exception value
            _ => SimpleException::new_msg(ExcType::TypeError, "exceptions must derive from BaseException"),
        };

        // Create frame with appropriate hide_caret setting
        let frame = if is_raise {
            RawStackFrame::from_raise(self.current_position().unwrap_or_default(), self.current_frame_name())
        } else {
            self.make_stack_frame()
        };

        RunError::Exc(ExceptionRaise {
            exc: simple_exc,
            frame: Some(frame),
            hide_caret: false,
            token: 0,
        })
    }

    /// Turns the operand of a `raise` into the error to propagate plus the
    /// object `except ... as e` should bind, consuming `exc`.
    ///
    /// A bare class is instantiated first, as CPython does, so `raise MyError`
    /// and `raise MyError()` reach the same object; that runs `__init__`
    /// synchronously, so an `__init__` that calls an external function raises
    /// instead of suspending. Anything that is not an exception yields
    /// CPython's `TypeError: exceptions must derive from BaseException`.
    pub(super) fn prepare_raise(&mut self, exc: Value) -> (RunError, Option<Value>) {
        // Classified before `exc` is consumed: every binding a `match exc` could
        // make here is `Copy`, so the arms would leave the operand's own
        // reference undropped rather than moving it.
        let operand = match &exc {
            Value::Builtin(Builtins::ExcType(exc_type)) => RaiseOperand::BuiltinType(*exc_type),
            Value::Ref(id) if matches!(self.heap.get(*id), HeapData::Class(class) if class.exc_base().is_some()) => {
                RaiseOperand::ExceptionClass
            }
            // Already an exception object (builtin or sandbox-defined), or
            // something that is not an exception at all, for which
            // produces the `TypeError` for the latter.
            _ => RaiseOperand::Object,
        };
        let instance = match operand {
            // `raise ValueError` / `raise MyError`: instantiate with no args.
            RaiseOperand::BuiltinType(exc_type) => {
                exc.drop_with(self);
                match exc_type.call(self, ArgValues::Empty) {
                    Ok(value) => value,
                    Err(e) => return (e, None),
                }
            }
            RaiseOperand::ExceptionClass => {
                let built = self.evaluate_function("raise", &exc, ArgValues::Empty);
                exc.drop_with(self);
                match built {
                    Ok(value) => value,
                    Err(e) => return (e, None),
                }
            }
            RaiseOperand::Object => exc,
        };
        let error = self.make_exception(&instance, true); // is_raise=true, hide caret
        if self.is_exception_object(&instance) {
            self.attach_context(&instance);
            (error, Some(instance))
        } else {
            instance.drop_with(self);
            (error, None)
        }
    }

    /// Whether `value` is something `raise` accepts: a builtin exception object
    /// or an instance of a sandbox-defined exception class.
    fn is_exception_object(&self, value: &Value) -> bool {
        match value {
            Value::Ref(id) => match self.heap.get(*id) {
                HeapData::Exception(_) => true,
                HeapData::Instance(_) => instance_exc_base(*id, self).is_some(),
                _ => false,
            },
            _ => false,
        }
    }

    /// Records `__cause__` on the object being raised by `raise X from Y`,
    /// consuming `cause`. A rebuilt exception (no object to write to) drops it.
    pub(super) fn attach_cause(&mut self, raised: Option<Value>, cause: Value) -> Option<Value> {
        match raised.as_ref() {
            Some(&Value::Ref(id)) => self.set_chain_slot(id, "__cause__", cause),
            _ => cause.drop_with(self),
        }
        raised
    }

    /// Records `__context__`: the exception currently being handled, if any and
    /// if it is not the one being raised (CPython skips self-chaining).
    fn attach_context(&mut self, raised: &Value) {
        let &Value::Ref(id) = raised else { return };
        let Some(context) = self
            .exception_stack
            .last()
            .filter(|active| !matches!(active, Value::Ref(active_id) if *active_id == id))
            .map(|active| active.clone_with_heap(self.heap))
        else {
            return;
        };
        self.set_chain_slot(id, "__context__", context);
    }

    /// Writes one of the chaining slots on a raised exception, consuming `value`.
    fn set_chain_slot(&mut self, id: HeapId, slot: &'static str, value: Value) {
        match self.heap.read(id) {
            HeapReadOutput::Exception(mut exc) => {
                if slot == "__cause__" {
                    exc.set_cause(value, self);
                } else {
                    exc.set_context(value, self);
                }
            }
            HeapReadOutput::Instance(mut inst) => {
                let name = allocate_string(slot, self.heap);
                // A failure here is a resource limit, which is terminal
                // anyway; the chain slot is not worth aborting the raise for.
                if let Ok(previous) = inst.set_attr(name, value, self) {
                    previous.drop_with(self);
                }
            }
            other => {
                drop(other);
                value.drop_with(self);
            }
        }
    }

    /// Summarizes a sandbox-defined exception instance for the error path.
    ///
    /// The message is `str(e)`: a user `__str__` when the class defines one,
    /// otherwise `BaseException.__str__` over `e.args`. A `__str__` that itself
    /// raises falls back to the args form rather than replacing the exception
    /// being raised, which is what CPython's traceback printer does when it
    /// cannot stringify a value.
    fn user_exception_summary(&mut self, instance_id: HeapId) -> SimpleException {
        let exc_base = instance_exc_base(instance_id, self).unwrap_or(ExcType::Exception);
        let name = {
            let class_id = instance_class_id(instance_id, self).expect("checked to be an instance");
            class_name(class_id, self.heap, self.interns).into_owned()
        };
        let message = self
            .user_exception_message(instance_id)
            .unwrap_or_else(|_| Some(format!("<unprintable {name} object>")));
        SimpleException::new(exc_base, message).with_user_type(name)
    }

    /// `str(e)` for a sandbox-defined exception instance.
    fn user_exception_message(&mut self, instance_id: HeapId) -> RunResult<Option<String>> {
        let class_id = instance_class_id(instance_id, self).expect("checked to be an instance");
        let this = self;
        if class_defines(class_id, "__str__", this) {
            let text = instance_str(instance_id, this)?;
            defer_drop!(text, this);
            let text = text.to_str(this)?.to_owned();
            return Ok((!text.is_empty()).then_some(text));
        }
        let args = instance_args(instance_id, this);
        let message = exception_message(&args, this);
        for arg in args {
            arg.drop_with(this);
        }
        message
    }

    /// Runs fused bare `assert test`.
    ///
    /// Truthy values pass; falsy values raise with their repr, except literal
    /// `False` has no detail because `assert False` adds no information.
    pub(super) fn assert_test(&mut self) -> Result<(), RunError> {
        let this = self;
        let test = this.pop();
        defer_drop!(test, this);
        if test.py_bool(this)? {
            Ok(())
        } else if matches!(test, Value::Bool(false)) {
            Err(this.assertion_error(None))
        } else {
            let detail = assert_operand_repr(test, this).map(Some);
            Err(this.assert_failure(detail))
        }
    }

    /// Runs fused bare `assert lhs OP rhs`.
    ///
    /// Shares [`cmp_values`](VM::cmp_values) with the `Compare*` opcodes, so
    /// the comparison (and any `TypeError` it raises) behaves identically to
    /// the unfused form; only a `false` result diverges, raising with both
    /// operand reprs.
    pub(super) fn assert_cmp(&mut self, op: CmpOperator) -> Result<(), RunError> {
        let this = self;
        let rhs = this.pop();
        defer_drop!(rhs, this);
        let lhs = this.pop();
        defer_drop!(lhs, this);
        if this.cmp_values(op, lhs, rhs)? {
            Ok(())
        } else {
            let detail = assert_operand_repr(lhs, this).and_then(|lhs_repr| {
                let rhs_repr = assert_operand_repr(rhs, this)?;
                Ok(Some(format!("{lhs_repr} {op} {rhs_repr}")))
            });
            Err(this.assert_failure(detail))
        }
    }

    /// Converts best-effort detail into an `AssertionError` message.
    /// Catchable formatting errors fall back to no detail; terminal errors propagate.
    fn assert_failure(&self, detail: RunResult<Option<String>>) -> RunError {
        match detail {
            Ok(Some(detail)) => self.assertion_error(Some(format!("assert {detail}"))),
            Ok(None) | Err(RunError::Exc(_)) => self.assertion_error(None),
            Err(e) => e,
        }
    }

    /// Raises for failed `assert test, msg`.
    ///
    /// Uses `msg` first and appends introspected detail when available. If
    /// either formatting path raises a Python exception, the other still wins.
    pub(super) fn assert_failed_msg(&mut self, cmp_op: Option<CmpOperator>) -> RunError {
        let this = self;
        let msg_value = this.pop();
        defer_drop!(msg_value, this);
        // Format the operands first so they are popped and released even if
        // the message itself fails to stringify.
        let detail = match this.assert_detail(cmp_op) {
            Ok(detail) => detail,
            Err(RunError::Exc(_)) => None,
            Err(e) => return e,
        };
        let msg = match assert_msg_str(msg_value, this) {
            // An empty message adds nothing, so treat it like an absent one and
            // show only the detail — avoids a stray leading `\n` before `assert`.
            Ok(msg) if msg.is_empty() => None,
            Ok(msg) => Some(msg),
            Err(RunError::Exc(_)) => None,
            Err(e) => return e,
        };
        let full = match (msg, detail) {
            (Some(msg), Some(detail)) => Some(format!("{msg}\nassert {detail}")),
            (Some(msg), None) => Some(msg),
            (None, Some(detail)) => Some(format!("assert {detail}")),
            (None, None) => None,
        };
        this.assertion_error(full)
    }

    /// Pops failed assert operands and formats their detail.
    ///
    /// Comparisons produce `{lhs!r} {op} {rhs!r}`; other tests use the falsy
    /// value repr. Literal `False` returns no detail.
    fn assert_detail(&mut self, cmp_op: Option<CmpOperator>) -> RunResult<Option<String>> {
        let this = self;
        if let Some(op) = cmp_op {
            let rhs = this.pop();
            defer_drop!(rhs, this);
            let lhs = this.pop();
            defer_drop!(lhs, this);
            let lhs_repr = assert_operand_repr(lhs, this)?;
            let rhs_repr = assert_operand_repr(rhs, this)?;
            Ok(Some(format!("{lhs_repr} {op} {rhs_repr}")))
        } else {
            let test = this.pop();
            defer_drop!(test, this);
            if matches!(test, Value::Bool(false)) {
                Ok(None)
            } else {
                assert_operand_repr(test, this).map(Some)
            }
        }
    }

    /// Creates an `AssertionError` raised at the current source position.
    fn assertion_error(&self, msg: Option<String>) -> RunError {
        let frame = RawStackFrame::from_raise(self.current_position().unwrap_or_default(), self.current_frame_name());
        RunError::Exc(ExceptionRaise {
            exc: SimpleException::new(ExcType::AssertionError, msg),
            frame: Some(frame),
            hide_caret: false,
            token: 0,
        })
    }

    /// Handles an exception by searching for a handler in the exception table.
    ///
    /// Returns:
    /// - `Some(VMResult)` if the exception was not caught (should return from run loop)
    /// - `None` if the exception was caught (continue execution)
    ///
    /// When an exception is caught:
    /// 1. Unwinds the stack to the handler's expected depth
    /// 2. Pushes the exception value onto the stack
    /// 3. Sets `current_exception` for bare `raise`
    /// 4. Jumps to the handler code
    pub(super) fn handle_exception(&mut self, error: RunError) -> Option<RunError> {
        self.handle_exception_with_value(error, None)
    }

    /// [`handle_exception`](Self::handle_exception) reusing an already-built
    /// exception instead of reallocating an identical one per level; this also
    /// preserves its identity, as CPython does. Owned: dropped if unused.
    pub(super) fn handle_exception_with_value(
        &mut self,
        mut error: RunError,
        raised: Option<Value>,
    ) -> Option<RunError> {
        let mut raised = raised.or_else(|| self.take_pending_raised(&error));
        loop {
            match self.handle_exception_step(error, raised.take()) {
                ExceptionHandlingResult::Caught => return None,
                ExceptionHandlingResult::Unhandled(error) => return Some(error),
                ExceptionHandlingResult::PropagateToWaiter(waiter_error) => error = waiter_error,
            }
        }
    }

    /// Reclaims the exception object parked by an earlier unwinding step, but
    /// only when `error` is the very raise that parked it.
    ///
    /// Anything left over from an error that was swallowed on the way out (a
    /// `StopIteration` ending an iterator) has a token no live error claims, so
    /// it is released here rather than attached to an unrelated raise.
    fn take_pending_raised(&mut self, error: &RunError) -> Option<Value> {
        let (parked, token) = self.pending_raised.take()?;
        let wanted = match error {
            RunError::Exc(exc) | RunError::UncatchableExc(exc) => exc.token,
            RunError::Internal(_) => 0,
        };
        if wanted == token {
            Some(parked)
        } else {
            parked.drop_with(self);
            None
        }
    }

    /// Puts back the traceback of the raise `exc` came from, so a re-raise
    /// reports the line the exception was first raised at rather than the line
    /// re-raising it, which is what CPython's traceback shows.
    ///
    /// Silent when the parked chain belongs to a different object, which is the
    /// case for an exception raised before this build parked anything and for
    /// one that survived a suspension.
    pub(super) fn restore_caught_origin(&self, exc: &Value, error: &mut RunError) {
        let Some((origin_id, origin)) = &self.caught_origin else {
            return;
        };
        if exc.ref_id() != Some(*origin_id) {
            return;
        }
        match error {
            RunError::Exc(raise) | RunError::UncatchableExc(raise) => raise.frame = Some(origin.clone()),
            RunError::Internal(_) => {}
        }
    }

    /// Parks `raised` so the next handler in an outer `run()` can bind the very
    /// object that was raised, stamping `error` with the matching token.
    ///
    /// Only sandbox-defined exceptions need this: every other raised value is
    /// rebuilt losslessly from the error's own summary.
    fn park_raised(&mut self, error: &mut RunError, raised: Value) {
        if !matches!(&raised, Value::Ref(id) if matches!(self.heap.get(*id), HeapData::Instance(_))) {
            raised.drop_with(self);
            return;
        }
        let RunError::Exc(exc) = error else {
            raised.drop_with(self);
            return;
        };
        self.raise_seq = self.raise_seq.wrapping_add(1);
        exc.token = self.raise_seq;
        if let Some((previous, _)) = self.pending_raised.replace((raised, self.raise_seq)) {
            previous.drop_with(self);
        }
    }

    /// Handles one propagation step, yielding when the error moves to a waiter.
    fn handle_exception_step(&mut self, mut error: RunError, raised: Option<Value>) -> ExceptionHandlingResult {
        // Ensure exception has initial frame info
        error = self.attach_frame_to_error(error);

        // For terminal resource errors such as memory limits,
        // we still need to unwind the stack to collect all frames for the traceback
        if matches!(error, RunError::UncatchableExc(_) | RunError::Internal(_)) {
            if let Some(raised) = raised {
                raised.drop_with(self);
            }
            return ExceptionHandlingResult::Unhandled(self.unwind_for_traceback(error));
        }

        let exc_value = if let Some(raised) = raised {
            raised
        } else {
            // Nothing to reuse: build it from `error`, borrowed rather than
            // cloned since it is a local and so cannot alias `self`.
            let RunError::Exc(exc) = &error else {
                unreachable!("terminal errors returned above")
            };
            self.create_exception_value(exc)
        };

        // Use DropGuard because exc_value is conditionally consumed (pushed onto
        // exception_stack when handler found) or dropped (when no handler found)
        let mut exc_guard = DropGuard::new(exc_value, self);

        // Search for handler in current and outer frames
        loop {
            let (exc_value, this) = exc_guard.as_parts();
            let frame = this.current_frame();
            let ip = u32::try_from(this.instruction_ip).expect("instruction IP exceeds u32");

            // Search exception table for a handler covering this IP
            if let Some(entry) = frame.code.find_exception_handler(ip) {
                // Unwind operands to the compiler-recorded region depth,
                // including any in-flight comprehension values.
                let handler_offset = usize::try_from(entry.handler()).expect("handler offset exceeds usize");
                let target_stack_depth = frame.stack_base + frame.locals_count as usize + entry.stack_depth() as usize;
                let target_exc_stack_depth = frame.exception_stack_base + entry.exception_stack_count() as usize;
                let pushes_exception = entry.pushes_exception();

                // Unwind stack to target depth (drop excess values)
                for value in this.stack.drain(target_stack_depth..).rev() {
                    value.drop_with(this.heap);
                }

                // Drop exceptions from bypassed handlers so a later bare
                // `raise` cannot revive them.
                while this.exception_stack.len() > target_exc_stack_depth {
                    let value = this.exception_stack.pop().unwrap();
                    value.drop_with(this);
                }

                // Push the exception only for handlers that read it; cleanup
                // handlers re-raise straight from `exception_stack`.
                if pushes_exception {
                    let exc_for_stack = exc_value.clone_with_heap(this);
                    this.push(exc_for_stack);
                }

                // Reclaim exc_value from guard - it's being pushed onto exception_stack
                let (exc_value, this) = exc_guard.into_parts();

                // Park where this exception was raised before the error carrying
                // that chain is dropped, so a re-raise of the same object reports
                // the original raise rather than the re-raise (see `caught_origin`).
                this.caught_origin = exc_value.ref_id().zip(take_error_frame(&mut error));

                // Push exception onto the exception_stack for bare raise.
                // This allows nested except handlers to restore outer
                // exception context.
                this.exception_stack.push(exc_value);

                // Jump to handler
                this.current_frame_mut().ip = handler_offset;

                return ExceptionHandlingResult::Caught;
            }

            // No handler in this frame - pop frame and try outer
            if this.frames.len() <= 1 {
                // No more frames - exception is unhandled
                let is_spawned = this.is_spawned_task();

                // Reclaim exc_value before potentially switching tasks; a
                // sandbox-defined exception is parked so an outer `run()` can
                // still catch the object itself, the rest are released.
                let (exc_value, this) = exc_guard.into_parts();
                this.park_raised(&mut error, exc_value);

                // For spawned tasks, fail the task instead of propagating
                if is_spawned {
                    return match self.handle_task_failure(error) {
                        Ok(()) => {
                            // Switched to next task - continue execution
                            ExceptionHandlingResult::Caught
                        }
                        Err(waiter_error) => ExceptionHandlingResult::PropagateToWaiter(waiter_error),
                    };
                }

                return ExceptionHandlingResult::Unhandled(error);
            }

            // A generator dies with an exception escaping its body; the
            // error then belongs to whoever asked it for a value.
            let generator_boundary = this.at_generator_frame();

            // Get the caller's call-site offset before popping frame.
            // This is where the caller invoked the function that's failing.
            let call_offset = this.current_frame().call_offset;

            // Pop this frame
            let stop = this.pop_frame();
            if generator_boundary {
                this.close_unwound_generator();
            }
            if stop {
                // The frame indicated evaluation should stop - e.g. inside
                // `evaluate_function` - return the error now to stop unwinding,
                // parking the raised object so the native caller's own handler
                // search can still bind it.
                let (exc_value, this) = exc_guard.into_parts();
                this.park_raised(&mut error, exc_value);
                return ExceptionHandlingResult::Unhandled(error);
            }

            // Add caller frame info to traceback (if we have a call site).
            // Resolve the offset now — against the caller, which is the current
            // frame after the pop above.
            if let Some(off) = call_offset {
                let pos = this.resolve_offset(off);
                let frame_name = this.current_frame_name();
                match &mut error {
                    RunError::Exc(exc) => exc.add_caller_frame(pos, frame_name),
                    RunError::UncatchableExc(exc) => exc.add_caller_frame(pos, frame_name),
                    RunError::Internal(_) => {}
                }
            }
        }
    }

    /// Unwinds the call stack to collect all frames for a traceback.
    ///
    /// Used for terminal resource errors that can't be handled
    /// but still need a complete traceback showing all active call frames.
    fn unwind_for_traceback(&mut self, mut error: RunError) -> RunError {
        // Pop frames and add caller frame info to the traceback
        while self.frames.len() > 1 {
            let generator_boundary = self.at_generator_frame();

            // Get the caller's call-site offset before popping frame
            let call_offset = self.current_frame().call_offset;

            // Pop this frame (cleans up namespace, etc.)
            self.pop_frame();
            if generator_boundary {
                self.close_unwound_generator();
            }

            // Add caller frame info to traceback. Resolve the offset against the
            // caller, which is the current frame after the pop above.
            if let Some(off) = call_offset {
                let pos = self.resolve_offset(off);
                let frame_name = self.current_frame_name();
                match &mut error {
                    RunError::Exc(exc) => exc.add_caller_frame(pos, frame_name),
                    RunError::UncatchableExc(exc) => exc.add_caller_frame(pos, frame_name),
                    RunError::Internal(_) => {}
                }
            }
        }
        error
    }

    /// Creates an exception Value from exception info.
    ///
    /// Allocates an Exception on the heap and returns a Value::Ref to it.
    fn create_exception_value(&mut self, exc: &ExceptionRaise) -> Value {
        let exception = ExceptionObject::from_summary(exc.exc.clone(), self);
        let heap_id = self.heap.allocate(HeapData::Exception(Box::new(exception)));
        Value::Ref(heap_id)
    }

    /// Checks if an exception matches an `except` clause's exception type.
    ///
    /// `exc_type` must be either a single exception class, or a *flat* tuple of
    /// exception classes. Returns `Ok(true)` if the exception matches, `Ok(false)`
    /// if it doesn't, or `Err` if `exc_type` is not a valid exception type.
    ///
    /// This deliberately does **not** recurse into nested tuples. The exception
    /// type handed to `except` is constructed at runtime, so a tuple could be
    /// nested arbitrarily deeply regardless of source nesting limits; a recursive
    /// matcher would overflow the host's native stack inside this single bytecode
    /// instruction. Mirroring CPython's `check_except_type_valid` (the
    /// `CHECK_EXC_MATCH` opcode), only one level of tuple is accepted: a nested
    /// tuple element — or any non-exception value — raises
    /// `TypeError: catching classes that do not inherit from BaseException is not
    /// allowed`. Removing the recursion both keeps parity with CPython and
    /// eliminates the unbounded-recursion footgun entirely, so no recursion-depth
    /// or time bound is needed here.
    ///
    /// Like CPython, the *whole* tuple is validated rather than short-circuiting
    /// on the first match: an invalid element raises the `TypeError` even when an
    /// earlier element already matched (e.g. `except (TypeError, (ValueError,))`
    /// raising `TypeError` still raises the `TypeError` about catching classes).
    pub(super) fn check_exc_match(&self, exception: &Value, exc_type: &Value) -> Result<bool, RunError> {
        match exc_type {
            // Flat tuple of exception classes. CPython does not descend into
            // nested tuples in this position, so neither do we.
            Value::Ref(id) if let HeapData::Tuple(tuple) = self.heap.get(*id) => {
                let mut matched = false;
                for handler in tuple.as_slice() {
                    // A nested tuple or any non-exception value is rejected
                    // exactly as CPython rejects it, even if a previous element
                    // already matched.
                    let Some(hit) = self.exc_matches_handler(exception, handler) else {
                        return Err(ExcType::except_invalid_type_error());
                    };
                    matched |= hit;
                }
                Ok(matched)
            }
            // A single exception class, builtin or sandbox-defined.
            single => self
                .exc_matches_handler(exception, single)
                .ok_or_else(ExcType::except_invalid_type_error),
        }
    }

    /// Whether the exception object at `exc` is caught by the class `handler`,
    /// the same rule an `except` clause applies, so a `contextlib.suppress`
    /// swallows what the equivalent handler would catch, sandbox-defined
    /// exception classes included.
    ///
    /// Takes the raised object by id because the unwinding machinery owns it:
    /// the `Value` built here never reaches a drop, so it takes no reference of
    /// its own.
    pub(crate) fn exc_id_matches_class(&self, exc: HeapId, handler: &Value) -> Option<bool> {
        let exception = ManuallyDrop::new(Value::Ref(exc));
        self.exc_matches_handler(&exception, handler)
    }

    /// Returns whether the raised `exception` is caught by the single class
    /// `handler`, or `None` when `handler` is not an exception class at all
    /// (which the caller turns into CPython's "catching classes that do not
    /// inherit from BaseException" `TypeError`).
    ///
    /// Shared by the single-class and flat-tuple arms of [`check_exc_match`].
    fn exc_matches_handler(&self, exception: &Value, handler: &Value) -> Option<bool> {
        match handler {
            // A builtin exception type catches builtin exceptions by the
            // hard-coded hierarchy, and a sandbox-defined one through the
            // nearest builtin ancestor its class chain reaches.
            Value::Builtin(Builtins::ExcType(handler_type)) => Some(match exception.py_type(self) {
                Type::Exception(raised) => raised.is_subclass_of(*handler_type),
                Type::Instance(_) => match exception {
                    Value::Ref(id) => instance_exc_base(*id, self).is_some_and(|b| b.is_subclass_of(*handler_type)),
                    _ => false,
                },
                _ => false,
            }),
            // A sandbox-defined exception class catches its own instances and
            // those of its subclasses. A builtin exception never matches one:
            // it cannot have a sandbox class as an ancestor.
            Value::Ref(handler_id) => match self.heap.get(*handler_id) {
                HeapData::Class(class) if class.exc_base().is_some() => Some(match exception {
                    Value::Ref(raised_id) => match self.heap.get(*raised_id) {
                        HeapData::Instance(inst) => class_is_subclass(inst.class(), *handler_id, self),
                        _ => false,
                    },
                    _ => false,
                }),
                _ => None,
            },
            _ => None,
        }
    }
}

/// Streams an assert operand's repr into the configured byte-capped writer.
/// Reaching the cap stops formatting the remainder and appends `…`.
fn assert_operand_repr(value: &Value, vm: &mut VM<'_>) -> RunResult<String> {
    let mut writer = TruncatingWriter::new(vm.assert_repr_max_bytes as usize);
    let mut heap_ids = LazyHeapSet::default();
    match value.py_repr_fmt(&mut writer, vm, &mut heap_ids) {
        Ok(()) => Ok(writer.into_string()),
        // The cap abort is ours: the partial repr is the result. Genuine
        // errors can only surface before the cap (post-cap writes do no VM work).
        Err(_) if writer.truncated => Ok(writer.into_string()),
        Err(e) => Err(e),
    }
}

/// `str()` of an explicit assert message, matching how the message renders in
/// `AssertionError: {msg}` — not truncated, since the user chose it explicitly.
fn assert_msg_str(value: &Value, vm: &mut VM<'_>) -> RunResult<String> {
    let str_value = value.py_str(vm)?;
    defer_drop!(str_value, vm);
    Ok(str_value.to_str(vm)?.to_owned())
}

/// Byte-capped sink that stops repr formatting on a character boundary.
/// Its buffer is untracked because `py_repr_fmt` also needs mutable VM access.
struct TruncatingWriter {
    buf: String,
    /// Bytes still accepted before the cap.
    remaining: usize,
    /// Set when input was cut at the cap; `into_string` then appends `…`.
    truncated: bool,
}

impl TruncatingWriter {
    fn new(max_bytes: usize) -> Self {
        Self {
            buf: String::new(),
            remaining: max_bytes,
            truncated: false,
        }
    }

    /// Consumes the writer, appending `…` when input was cut at the cap.
    fn into_string(mut self) -> String {
        if self.truncated {
            self.buf.push('…');
        }
        self.buf
    }
}

impl Write for TruncatingWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if self.truncated {
            Err(fmt::Error)
        } else if let Some(remaining) = self.remaining.checked_sub(s.len()) {
            self.remaining = remaining;
            self.buf.push_str(s);
            Ok(())
        } else {
            // Over budget: cut at the last char boundary in budget (≤3 steps back).
            let mut idx = self.remaining;
            while !s.is_char_boundary(idx) {
                idx -= 1;
            }
            self.buf.push_str(&s[..idx]);
            self.remaining = 0;
            self.truncated = true;
            Err(fmt::Error)
        }
    }
}
