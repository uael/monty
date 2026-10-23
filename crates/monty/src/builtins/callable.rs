//! The `callable()` builtin.

use crate::{args::ArgValues, bytecode::VM, defer_drop, exception_private::RunResult, value::Value};

/// `callable(object)`: whether calling `object` would dispatch to something.
///
/// This is the check that `functools.partial()` and the other builtins that
/// take a callable already make, so an instance of a class that defines
/// `__call__` answers `True`, as in CPython.
pub(crate) fn builtin_callable(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let value = args.get_one_arg("callable", vm.heap)?;
    defer_drop!(value, vm);
    Ok(Value::Bool(value.is_callable(vm)))
}
