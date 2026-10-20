//! Implementation of the next() builtin function.

use crate::{
    args::{ArgValues, FromArgs},
    bytecode::{CallResult, VM},
    defer_drop,
    exception_private::RunResult,
    types::{generator::Reader, iter::iterator_next},
    value::Value,
};

/// Argument shape for `next(iterator, default=...)` — positional-only in
/// CPython, so `style = unpack` gives the `PyArg_UnpackTuple` arity wording
/// and the blanket `next() takes no keyword arguments` rejection.
#[derive(FromArgs)]
#[from_args(name = "next", style = unpack)]
struct NextArgs {
    #[from_args(pos_only)]
    iterator: Value,
    #[from_args(pos_only, default)]
    default: Option<Value>,
}

/// Implementation of the next() builtin function.
///
/// Retrieves the next item from an iterator.
///
/// Two forms are supported:
/// - `next(iterator)` - Returns the next item from the iterator. Raises
///   `StopIteration` when the iterator is exhausted.
/// - `next(iterator, default)` - Returns the next item from the iterator, or
///   `default` if the iterator is exhausted.
///
/// A generator is read on the VM's own loop, as its `__next__()` reads it, so a
/// call to the host in its body suspends the run as it would anywhere.
pub fn builtin_next(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let NextArgs { iterator, default } = NextArgs::from_args(args, vm)?;
    if let Some(gen_id) = vm.unfinished_generator(&iterator) {
        let has_default = default.is_some();
        if let Some(default) = default {
            vm.push(default);
        }
        vm.push(iterator);
        return vm.read_generator(gen_id, Reader::Next(has_default));
    }
    defer_drop!(iterator, vm);
    iterator_next(iterator, default, vm).map(CallResult::Value)
}
