//! Implementation of the sorted() builtin function.

use crate::{
    args::ArgValues,
    bytecode::{CallResult, VM},
    exception_private::{ExcType, ExcTypeExt, RunResult},
    heap::{DropGuard, DropWithContext, HeapData},
    sorting::{parse_sort_args, sort_by_key, sort_values},
    types::{List, iter::collect_owned_iterable},
    value::Value,
};

/// Implementation of the sorted() builtin function.
///
/// Returns a new sorted list from the items in an iterable. CPython's
/// `sorted(iterable, /, *, key=None, reverse=False)` is implemented by
/// converting the iterable to a list and delegating to `list.sort`, so
/// the kwargs and any kwarg-related errors are owned by `sort`. We mirror
/// that by extracting the iterable positionally and handing the rest off
/// to [`parse_sort_args`] — the same entry point `list.sort` uses — so
/// unknown-kwarg errors uniformly read `sort() got an unexpected keyword
/// argument 'X'` without any wording overrides.
///
/// A key runs in a frame on the VM's own loop, so it may call the host; see
/// [`sort_by_key`].
pub fn builtin_sorted(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let (mut pos_iter, kwargs) = args.into_parts();
    let pos_count = pos_iter.len();
    if pos_count != 1 {
        pos_iter.drop_with(vm);
        kwargs.drop_with(vm);
        return Err(ExcType::type_error_expected_exact("sorted", 1, pos_count));
    }
    let iterable = pos_iter.next().expect("checked pos_count == 1");

    let items: Vec<_> = match collect_owned_iterable(iterable, vm) {
        Ok(items) => items,
        Err(err) => {
            kwargs.drop_with(vm);
            return Err(err);
        }
    };
    let mut items_guard = DropGuard::new(items, vm);
    let (items, vm) = items_guard.as_parts_mut();

    let sort_args = if kwargs.is_empty() {
        ArgValues::Empty
    } else {
        ArgValues::Kwargs(kwargs)
    };
    let (key, reverse) = parse_sort_args(sort_args, vm)?;
    if let Some(key) = key {
        let (items, vm) = items_guard.into_parts();
        let items = Value::Ref(vm.heap.allocate(HeapData::List(List::new(items))));
        return sort_by_key(items, key, reverse, Value::None, vm);
    }
    sort_values(items, reverse, vm)?;

    let (items, vm) = items_guard.into_parts();
    let heap_id = vm.heap.allocate(HeapData::List(List::new(items)));
    Ok(CallResult::Value(Value::Ref(heap_id)))
}
