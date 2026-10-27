//! Implementation of the filter() builtin function.
//!
//! This module provides the filter() builtin which filters elements from an iterable
//! based on a predicate function, or on the truth of each element when it is `None`.

use crate::{
    args::ArgValues,
    bytecode::{CallResult, VM},
    defer_drop,
    exception_private::RunResult,
    frozen::FrozenFunction,
    heap::{DropGuard, HeapData},
    types::{List, PyTrait},
    value::Value,
};

/// The loop of `filter()` with a predicate, or over a generator, which runs on
/// the VM's own loop as a frame of its own.
///
/// So the predicate and the read of the iterable step there, and a call to the
/// host in either suspends the run.
pub(crate) const FILTER_SOURCE: &str = r"
def __monty_filter(function, iterable):
    out = []
    if function is None:
        for item in iterable:
            if item:
                out.append(item)
        return out
    for item in iterable:
        if function(item):
            out.append(item)
    return out
";

/// Implementation of the filter() builtin function.
///
/// Filters elements from an iterable based on a predicate function.
/// If the predicate is None, filters out falsy values.
/// A predicate, or a generator to read, runs in [`FILTER_SOURCE`], a frame on
/// the VM's own loop, so it may call the host.
///
/// Note: In Python this returns an iterator, but we return a list for simplicity.
///
/// Examples:
/// ```python
/// filter(lambda x: x > 0, [-1, 0, 1, 2])  # [1, 2]
/// filter(None, [0, 1, False, True, ''])   # [1, True]
/// ```
pub fn builtin_filter(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let (function, iterable) = args.get_two_args("filter", vm.heap)?;
    if !matches!(function, Value::None) || vm.unfinished_generator(&iterable).is_some() {
        return vm.call_frozen_exact(FrozenFunction::Filter, [function, iterable]);
    }

    let iter = iterable.into_py_iter(vm)?.require_synchronous(vm)?;
    defer_drop!(iter, vm);
    let mut iter = iter.read(vm);

    let out: Vec<Value> = Vec::new();
    let mut out_guard = DropGuard::new(out, vm);
    let (out, vm) = out_guard.as_parts_mut();

    while let Some(item) = iter.py_next(vm)? {
        let mut item_guard = DropGuard::new(item, vm);
        let (item, vm) = item_guard.as_parts_mut();
        if item.py_bool(vm)? {
            out.push(item_guard.into_inner());
        }
    }

    let (out, vm) = out_guard.into_parts();
    let heap_id = vm.heap.allocate(HeapData::List(List::new(out)));
    Ok(CallResult::Value(Value::Ref(heap_id)))
}
