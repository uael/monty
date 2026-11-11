//! Implementation of the any() builtin function.

use crate::{
    args::ArgValues,
    bytecode::{CallResult, VM},
    defer_drop,
    exception_private::RunResult,
    types::{PyTrait, generator::Reader},
    value::Value,
};

/// Implementation of the any() builtin function.
///
/// Returns True if any element of the iterable is true.
/// Returns False for an empty iterable. Short-circuits on the first truthy value.
/// A generator is read on the VM's own loop, one value at a time, so a call to
/// the host in its body suspends the run as it would anywhere.
pub fn builtin_any(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let iterable = args.get_one_arg("any", vm.heap)?;
    if let Some(gen_id) = vm.unfinished_generator(&iterable) {
        vm.push(iterable);
        return vm.read_generator(gen_id, Reader::Test(false));
    }
    let iter = iterable.into_py_iter(vm)?.require_synchronous(vm)?;
    defer_drop!(iter, vm);
    let mut iter = iter.read(vm);

    while let Some(item) = iter.py_next(vm)? {
        defer_drop!(item, vm);
        if item.py_bool(vm)? {
            return Ok(CallResult::Value(Value::Bool(true)));
        }
    }

    Ok(CallResult::Value(Value::Bool(false)))
}
