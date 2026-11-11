//! The event loop `asyncio.get_running_loop()` hands back.
//!
//! Monty is always inside its own loop while sandbox code runs, so a program
//! that asks whether one is running is asking a question the answer to is
//! always yes. That is the whole of what this object is for: it proves a loop
//! is running, which is what the `RuntimeError` of CPython's
//! `get_running_loop()` outside one is the absence of. It schedules nothing,
//! and `limitations/asyncio.md` records the methods it does not answer.

use std::fmt::Write;

use super::py_trait::PyObjectIdentity;
use crate::{
    args::ArgValues,
    bytecode::{CallResult, VM},
    exception_private::{ExcType, ExcTypeExt, RunResult},
    hash::{HashValue, identity_hash},
    heap::{DropWithContext, HeapId, HeapItem, HeapObjectRead},
    types::{LazyHeapSet, PyTrait, Type},
    value::{EitherStr, Value},
};

/// The running event loop, which holds nothing because it does nothing.
///
/// A new one is made by each `get_running_loop()` call rather than shared, so
/// two calls give two objects where CPython gives the same one twice; nothing
/// here keeps loop state for them to share.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct EventLoop;

impl HeapItem for EventLoop {
    fn py_dec_ref_ids(&mut self, _stack: &mut Vec<HeapId>) {
        // The loop holds no references.
    }
}

impl<'h> PyTrait<'h> for HeapObjectRead<'h, EventLoop> {
    fn py_type(&self, _vm: &VM<'h>) -> Type {
        Type::EventLoop
    }

    fn py_len(&self, _vm: &VM<'h>) -> Option<usize> {
        None
    }

    fn py_eq_impl(&self, _other: &Value, _vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        Ok(None)
    }

    fn py_hash(&self, _vm: &mut VM<'h>) -> RunResult<Option<HashValue>> {
        Ok(Some(identity_hash(self.id())))
    }

    fn py_bool(&self, _vm: &mut VM<'h>) -> RunResult<bool> {
        Ok(true)
    }

    fn py_repr_fmt(&self, f: &mut impl Write, _vm: &mut VM<'h>, _heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        Ok(write!(
            f,
            "<EventLoop running=True at 0x{:x}>",
            self.py_identity().encoded()
        )?)
    }

    fn py_call_attr(&mut self, vm: &mut VM<'h>, attr: &EitherStr, args: ArgValues) -> RunResult<CallResult> {
        match attr.as_str(vm.interns) {
            // The loop a program can reach is the one it is running inside.
            "is_running" => {
                args.check_zero_args("is_running", vm.heap)?;
                Ok(CallResult::Value(Value::Bool(true)))
            }
            "is_closed" => {
                args.check_zero_args("is_closed", vm.heap)?;
                Ok(CallResult::Value(Value::Bool(false)))
            }
            other => {
                let other = other.to_owned();
                args.drop_with(vm);
                Err(ExcType::attribute_error(Type::EventLoop, &other))
            }
        }
    }
}
