//! The Python surface of a generator object: the iterator protocol plus
//! `send`, `throw` and `close`.
//!
//! Every method here is one step of the machinery in
//! [`crate::bytecode::vm::generator`]; this module only decides what a step's
//! outcome means to Python. The split matters because a step re-enters the VM,
//! so nothing may hold a borrow into the heap entry across one.

use std::fmt::Write;

use crate::{
    args::{ArgValues, FromArgs},
    bytecode::{CallResult, GeneratorInput, GeneratorStep, VM, stop_iteration_with},
    exception_private::{ExcType, ExcTypeExt, RunError, RunResult, SimpleException},
    generator::Generator,
    heap::{DropWithContext, HeapId, HeapRead},
    intern::StaticStrings,
    types::{LazyHeapSet, PyTrait, Type},
    value::{EitherStr, Value},
};

impl<'h> PyTrait<'h> for HeapRead<'h, Generator> {
    /// Only a plain generator is an iterator. An async one is stepped through
    /// `__anext__`, so reporting it here would let `for`/`list()` accept it.
    fn py_is_iterator(&self, vm: &VM<'h>) -> bool {
        !self.get(vm.heap).is_async
    }

    fn py_is_iterable(&self, vm: &VM<'h>) -> bool {
        !self.get(vm.heap).is_async
    }

    fn py_type(&self, vm: &VM<'h>) -> Type {
        self.get(vm.heap).py_type()
    }

    fn py_len(&self, _: &VM<'h>) -> Option<usize> {
        None
    }

    fn py_eq_impl(&self, _: &Value, _: &mut VM<'h>) -> RunResult<Option<bool>> {
        Ok(None)
    }

    fn py_repr_fmt(&self, f: &mut impl Write, vm: &mut VM<'h>, _: &mut LazyHeapSet) -> RunResult<()> {
        let generator = self.get(vm.heap);
        let type_name: &'static str = generator.py_type().into();
        let name = vm
            .interns
            .get_str(vm.interns.get_function(generator.func_id).name.name_id);
        write!(f, "<{type_name} object {name}>")?;
        Ok(())
    }

    fn py_iter(&self, self_id: Option<HeapId>, vm: &mut VM<'h>) -> RunResult<Value> {
        let self_id = self_id.expect("heap values have an id");
        if self.get(vm.heap).is_async {
            Err(ExcType::type_error_not_iterable(
                &self.py_type(vm).name(vm.heap, vm.interns),
            ))
        } else {
            vm.heap.inc_ref(self_id);
            Ok(Value::Ref(self_id))
        }
    }

    /// One `__next__` step.
    ///
    /// A generator that `return`s a value reports plain exhaustion here, the
    /// way CPython's `for` discards `StopIteration.value`. The value is parked
    /// on the generator so the sites that do care — `next(gen)` and
    /// `yield from` — can still pick it up.
    fn py_next(&mut self, self_id: Option<HeapId>, vm: &mut VM<'h>) -> RunResult<Option<Value>> {
        let self_id = self_id.expect("heap values have an id");
        if self.get(vm.heap).is_async {
            return Err(ExcType::type_error_not_iterator(
                &self.py_type(vm).name(vm.heap, vm.interns),
            ));
        }
        match vm.generator_step(self_id, GeneratorInput::Send(Value::None))? {
            GeneratorStep::Yielded(value) => Ok(Some(value)),
            GeneratorStep::Returned(value) => {
                // Park it again: this signature can only report exhaustion, and
                // `next()` still has to raise `StopIteration(value)`.
                vm.park_generator_result(self_id, value);
                Ok(None)
            }
        }
    }

    fn py_call_attr(
        &mut self,
        self_id: HeapId,
        vm: &mut VM<'h>,
        attr: &EitherStr,
        args: ArgValues,
    ) -> RunResult<CallResult> {
        let is_async = self.get(vm.heap).is_async;
        let type_name = self.py_type(vm);
        let Some(method) = attr.static_string().filter(|m| method_applies(*m, is_async)) else {
            args.drop_with(vm);
            return Err(ExcType::attribute_error(type_name, attr.as_str(vm.interns)));
        };
        call_generator_method(self_id, method, args, vm).map(CallResult::Value)
    }
}

/// Whether `method` exists on this flavour of generator.
///
/// The two flavours share `throw`/`close` but not the stepping protocol:
/// CPython gives a plain generator `__iter__`/`__next__`/`send` and an async
/// one `__aiter__`/`__anext__`/`asend`.
fn method_applies(method: StaticStrings, is_async: bool) -> bool {
    match method {
        StaticStrings::DunderIter | StaticStrings::DunderNext | StaticStrings::Send => !is_async,
        StaticStrings::DunderAiter | StaticStrings::DunderAnext => is_async,
        StaticStrings::Throw | StaticStrings::Close => true,
        _ => false,
    }
}

/// Runs one generator method.
fn call_generator_method(self_id: HeapId, method: StaticStrings, args: ArgValues, vm: &mut VM<'_>) -> RunResult<Value> {
    match method {
        // Both protocols hand back the generator itself as their iterator.
        StaticStrings::DunderIter | StaticStrings::DunderAiter => {
            args.check_zero_args("generator.__iter__", vm.heap)?;
            vm.heap.inc_ref(self_id);
            Ok(Value::Ref(self_id))
        }
        StaticStrings::DunderNext => {
            args.check_zero_args("generator.__next__", vm.heap)?;
            generator_send(self_id, Value::None, vm)
        }
        // `agen.__anext__()` hands the generator back as its own awaitable:
        // `await` on a generator drives exactly one step (see `exec_get_awaitable`).
        StaticStrings::DunderAnext => {
            args.check_zero_args("async_generator.__anext__", vm.heap)?;
            vm.heap.inc_ref(self_id);
            Ok(Value::Ref(self_id))
        }
        StaticStrings::Send => {
            let value = args.get_one_arg("generator.send", vm.heap)?;
            generator_send(self_id, value, vm)
        }
        StaticStrings::Throw => {
            let ThrowArgs { exc } = ThrowArgs::from_args(args, vm)?;
            let error = vm.make_exception(&exc, true);
            let raised = exc;
            let step = vm.generator_step(self_id, GeneratorInput::Throw(error));
            raised.drop_with(vm);
            match step? {
                GeneratorStep::Yielded(value) => Ok(value),
                GeneratorStep::Returned(value) => Err(stop_iteration_with(value, vm)),
            }
        }
        StaticStrings::Close => {
            args.check_zero_args("generator.close", vm.heap)?;
            generator_close(self_id, vm)
        }
        _ => unreachable!("method_applies rejects every other name"),
    }
}

/// Argument shape for `generator.throw(exc, /)`.
///
/// CPython's legacy `throw(type, value, traceback)` form is not accepted; see
/// `limitations/iter.md`.
#[derive(FromArgs)]
#[from_args(name = "throw", style = unpack)]
struct ThrowArgs {
    #[from_args(pos_only)]
    exc: Value,
}

/// `gen.send(value)` / `gen.__next__()`: step, and turn a return into the
/// `StopIteration` the protocol expects.
fn generator_send(self_id: HeapId, value: Value, vm: &mut VM<'_>) -> RunResult<Value> {
    match vm.generator_step(self_id, GeneratorInput::Send(value))? {
        GeneratorStep::Yielded(value) => Ok(value),
        GeneratorStep::Returned(value) => Err(stop_iteration_with(value, vm)),
    }
}

/// `gen.close()`: throw `GeneratorExit` at the suspension point so the body's
/// `finally` blocks run.
///
/// A generator that swallows the exit and yields again is a bug in the
/// generator, and CPython reports it as a `RuntimeError` rather than letting
/// the value escape.
fn generator_close(self_id: HeapId, vm: &mut VM<'_>) -> RunResult<Value> {
    let exit: RunError = SimpleException::new_none(ExcType::GeneratorExit).into();
    match vm.generator_step(self_id, GeneratorInput::Throw(exit)) {
        Ok(GeneratorStep::Returned(value)) => {
            value.drop_with(vm);
            Ok(Value::None)
        }
        Ok(GeneratorStep::Yielded(value)) => {
            value.drop_with(vm);
            Err(SimpleException::new_msg(
                ExcType::RuntimeError,
                format!("{} ignored GeneratorExit", generator_kind(self_id, vm)),
            )
            .into())
        }
        // The exit reaching the caller is the normal shutdown, as is a body
        // that re-raises it; anything else is the generator's own error.
        Err(error) if is_generator_exit(&error) => Ok(Value::None),
        Err(error) => Err(error),
    }
}

/// Whether `error` is the `GeneratorExit` a `close()` threw coming back out.
fn is_generator_exit(error: &RunError) -> bool {
    matches!(error, RunError::Exc(raise) if raise.exc.exc_type() == ExcType::GeneratorExit)
}

/// `generator` or `async generator`, for error text.
fn generator_kind(self_id: HeapId, vm: &VM<'_>) -> &'static str {
    if vm.is_async_generator(self_id) {
        "async generator"
    } else {
        "generator"
    }
}
