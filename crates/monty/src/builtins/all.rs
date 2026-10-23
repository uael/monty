//! Implementation of the all() builtin function.

use crate::{
    args::ArgValues,
    bytecode::{CallResult, VM},
    defer_drop,
    exception_private::RunResult,
    types::{PyTrait, generator::Reader},
    value::Value,
};

/// Implementation of the all() builtin function.
///
/// Returns True if all elements of the iterable are true (or if the iterable is empty).
/// Short-circuits on the first falsy value.
/// A generator is read on the VM's own loop, one value at a time, so a call to
/// the host in its body suspends the run as it would anywhere.
pub fn builtin_all(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let iterable = args.get_one_arg("all", vm.heap)?;
    if let Some(gen_id) = vm.unfinished_generator(&iterable) {
        vm.push(iterable);
        return vm.read_generator(gen_id, Reader::Test(true));
    }
    defer_drop!(iterable, vm);
    let iter = iterable.py_iter(vm)?.require_synchronous(vm)?;
    defer_drop!(iter, vm);
    let mut iter = iter.read(vm);

    while let Some(item) = iter.py_next(vm)? {
        defer_drop!(item, vm);
        if !item.py_bool(vm)? {
            return Ok(CallResult::Value(Value::Bool(false)));
        }
    }

    Ok(CallResult::Value(Value::Bool(true)))
}
