//! The `callable()` builtin.

use crate::{args::ArgValues, bytecode::VM, defer_drop, exception_private::RunResult, value::Value};

/// `callable(object)`: whether calling `object` would dispatch to something.
///
/// Monty does not dispatch `__call__`, so an instance of a sandbox class that
/// defines one answers `False` where CPython answers `True`; see
/// `limitations/classes.md`.
pub(crate) fn builtin_callable(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let value = args.get_one_arg("callable", vm.heap)?;
    defer_drop!(value, vm);
    Ok(Value::Bool(value.is_callable(vm.heap)))
}
