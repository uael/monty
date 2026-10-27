//! Implementation of the zip() builtin function.

use std::mem;

use crate::{
    args::{ArgValues, FromArgs},
    builtins::{Builtins, BuiltinsFunctions},
    bytecode::{CallResult, VM},
    defer_drop, defer_drop_mut,
    exception_private::{ExcType, RunError, RunResult, SimpleException},
    frozen::FrozenFunction,
    heap::{DropGuard, HeapData},
    types::{List, PyTrait, Type, allocate_tuple, tuple::TupleVec},
    value::Value,
};

/// The loop of `zip()` over a generator, which runs on the VM's own loop as a
/// frame of its own.
///
/// So each generator steps there, one value at a time and in turn with the
/// other iterables as CPython steps them, and a call to the host in its body
/// suspends the run. `strict` is already a `bool`.
pub(crate) const ZIP_SOURCE: &str = r"
def __monty_zip(iterables, strict, iter_, next_, stop_iteration, tuple_, value_error):
    iterators = []
    count = 0
    for iterable in iterables:
        iterators.append(iter_(iterable))
        count += 1
    out = []
    while True:
        items = []
        stopped = -1
        index = 0
        for iterator in iterators:
            try:
                items.append(next_(iterator))
            except stop_iteration:
                stopped = index
                break
            index += 1
        if stopped < 0:
            out.append(tuple_(items))
            continue
        if not strict:
            return out
        relation = 'shorter'
        longer = stopped
        if not stopped:
            relation = 'longer'
            index = 1
            while index < count and not longer:
                try:
                    next_(iterators[index])
                    longer = index
                except stop_iteration:
                    pass
                index += 1
            if not longer:
                return out
        others = 'argument 1' if longer == 1 else f'arguments 1-{longer}'
        raise value_error(f'zip() argument {longer + 1} is {relation} than {others}')
";

/// Implementation of the zip() builtin function.
///
/// Returns a list of tuples, where the i-th tuple contains the i-th element
/// from each of the argument iterables. Stops when the shortest iterable is exhausted.
/// When `strict=True`, raises `ValueError` if any iterable has a different length.
/// A generator among the iterables is read in [`ZIP_SOURCE`], a frame on the
/// VM's own loop, so its body may call the host.
/// Note: In Python this returns an iterator, but we return a list for simplicity.
pub fn builtin_zip(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let ZipArgs { iterables, strict } = ZipArgs::from_args(args, vm)?;
    defer_drop_mut!(iterables, vm);
    // CPython's `strict` is truthy-checked (not strict typed), so test the raw
    // value rather than asking the macro to coerce to a strict `bool`.
    defer_drop!(strict, vm);
    let strict = strict.py_bool(vm)?;

    if iterables.is_empty() {
        // zip() with no arguments returns empty list
        let heap_id = vm.heap.allocate(HeapData::List(List::new(Vec::new())));
        return Ok(CallResult::Value(Value::Ref(heap_id)));
    }
    if iterables
        .iter()
        .any(|iterable| vm.unfinished_generator(iterable).is_some())
    {
        let iterables = allocate_tuple(TupleVec::from_vec(mem::take(iterables)), vm.heap);
        return vm.call_frozen_exact(
            FrozenFunction::Zip,
            [
                iterables,
                Value::Bool(strict),
                Value::Builtin(Builtins::Type(Type::Iterator)),
                Value::Builtin(Builtins::Function(BuiltinsFunctions::Next)),
                Value::Builtin(Builtins::ExcType(ExcType::StopIteration)),
                Value::Builtin(Builtins::Type(Type::Tuple)),
                Value::Builtin(Builtins::ExcType(ExcType::ValueError)),
            ],
        );
    }

    // Create iterators for each iterable
    let iterators: Vec<Value> = Vec::with_capacity(iterables.len());
    defer_drop_mut!(iterators, vm);
    for iterable in iterables.drain(..) {
        iterators.push(iterable.into_py_iter(vm)?.require_synchronous(vm)?);
    }
    let mut iterators = iterators.iter().map(|iter| iter.read(vm)).collect::<Vec<_>>();

    let mut result_guard = DropGuard::new(Vec::new(), vm);
    let (result, vm) = result_guard.as_parts_mut();

    // Zip until shortest iterator is exhausted
    'outer: loop {
        let mut items_guard = DropGuard::new(TupleVec::with_capacity(iterators.len()), vm);
        let (tuple_items, vm) = items_guard.as_parts_mut();

        for (i, iter) in iterators.iter_mut().enumerate() {
            if let Some(item) = iter.py_next(vm)? {
                tuple_items.push(item);
            } else {
                // This iterator is exhausted - stop zipping

                if strict {
                    // In strict mode, if i > 0 then argument i+1 ran out before
                    // the earlier ones, so it is "shorter."
                    if i > 0 {
                        return Err(strict_length_error(i + 1, i, "shorter"));
                    }
                    // i == 0: first iterator exhausted — verify every remaining
                    // iterator is also exhausted. If any still yields a value,
                    // that argument is "longer" than all preceding exhausted ones.
                    // j is the 0-based index; iterators 0..j are all exhausted,
                    // so j gives the count for the error message.
                    for (j, remaining) in iterators.iter_mut().enumerate().skip(1) {
                        if let Some(extra) = remaining.py_next(vm)? {
                            extra.drop_with(vm);
                            return Err(strict_length_error(j + 1, j, "longer"));
                        }
                    }
                }

                break 'outer;
            }
        }

        // Create tuple from collected items
        let (tuple_items, vm) = items_guard.into_parts();
        let tuple_val = allocate_tuple(tuple_items, vm.heap);
        result.push(tuple_val);
    }

    let (result, vm) = result_guard.into_parts();
    let heap_id = vm.heap.allocate(HeapData::List(List::new(result)));
    Ok(CallResult::Value(Value::Ref(heap_id)))
}

/// Argument shape for `zip(*iterables, strict=False)`.
///
/// `strict` is held as a `Value` because CPython evaluates it via truthiness
/// (`py_bool`), not strict-type equality, so accepting any value is correct.
#[derive(FromArgs)]
#[from_args(name = "zip")]
struct ZipArgs {
    #[from_args(varargs)]
    iterables: Vec<Value>,
    #[from_args(default = Value::Bool(false))]
    strict: Value,
}

/// Builds the `ValueError` for `zip(strict=True)` when iterables have different lengths.
///
/// Matches CPython's error format:
/// - `"zip() argument 2 is shorter than argument 1"` (singular)
/// - `"zip() argument 4 is shorter than arguments 1-3"` (plural)
fn strict_length_error(exhausted_arg: usize, num_longer_args: usize, relation: &str) -> RunError {
    let others = if num_longer_args == 1 {
        "argument 1".to_owned()
    } else {
        format!("arguments 1-{num_longer_args}")
    };
    SimpleException::new_msg(
        ExcType::ValueError,
        format!("zip() argument {exhausted_arg} is {relation} than {others}"),
    )
    .into()
}
