//! Shared sorting utilities for `sorted()` and `list.sort()`.
//!
//! Both `sorted()` and `list.sort()` use index-based sorting: they build
//! a vector of indices `[0, 1, 2, ...]`, sort the indices by comparing the
//! corresponding items (or key values), then rearrange items according to
//! the sorted indices.
//!
//! This module provides [`sort_indices`] for the comparison step and
//! [`apply_permutation`] for the in-place rearrangement step.
//!
//! A sort with a key computes the keys in [`SORT_BY_KEY_SOURCE`], a frame on
//! the VM's own loop, so a key may call the host and suspend the run.

use std::{cmp::Ordering, mem};

use crate::{
    args::{ArgValues, FromArgs, LaxBool},
    builtins::Builtins,
    bytecode::{CallResult, VM},
    defer_drop, defer_drop_mut,
    exception_private::{ExcType, ExcTypeExt, RunError, RunResult},
    frozen::{FrozenFunction, FrozenFunctions},
    heap::HeapReadOutput,
    modules::ModuleFunctions,
    types::{CmpOrder, PyTrait},
    value::Value,
};

/// The sort with a key of `sorted()` and `list.sort()`.
///
/// It calls the key on each item in order, as CPython does, then sorts the
/// private list `items` by the keys. `original` is the list that `list.sort()`
/// detached, which takes the items back whether the sort ends or raises, or
/// `None` for `sorted()`, which answers `items`.
pub(crate) const SORT_BY_KEY_SOURCE: &str = r"
def __monty_sort_by_key(items, key, reverse, original, order, restore, base_exception):
    keys = []
    if original is None:
        for item in items:
            keys.append(key(item))
        order(items, keys, reverse)
        return items
    try:
        for item in items:
            keys.append(key(item))
        order(items, keys, reverse)
    except base_exception:
        restore(original, items, False)
        raise
    restore(original, items, True)
";

/// Argument shape for `list.sort(*, key=None, reverse=False)` and, by
/// extension, the kwargs accepted by the `sorted()` builtin. Both fields
/// are keyword-only (CPython rejects positional `key`/`reverse`). `key` is
/// held as a raw `Option<Value>` so callers can normalise `key=None` to
/// "no key"; `reverse` uses [`LaxBool`] to match CPython's `bool()`-style
/// truth test (so `reverse=[]` is `False`, not a `TypeError`).
#[derive(FromArgs)]
#[from_args(name = "sort")]
struct ListSortArgs {
    #[from_args(kw_only, default)]
    key: Option<Value>,
    #[from_args(kw_only, default = LaxBool::new(false))]
    reverse: LaxBool,
}

/// Parses the `key`/`reverse` kwargs of a sort. The single entry point for
/// both `list.sort` and the `sorted()` builtin — sharing here is what makes
/// unknown-kwarg errors uniformly read `sort() got an unexpected keyword
/// argument 'X'` (matching CPython, whose `sorted` delegates to `list.sort`
/// internally). `key=None` gives no key.
pub fn parse_sort_args(args: ArgValues, vm: &mut VM<'_>) -> RunResult<(Option<Value>, bool)> {
    let ListSortArgs { key, reverse } = ListSortArgs::from_args(args, vm)?;
    let key = match key {
        Some(Value::None) => None,
        other => other,
    };
    Ok((key, reverse.bool()))
}

/// Starts the sort with a key of the private list `items`, as a frame of
/// [`SORT_BY_KEY_SOURCE`] on the VM's own loop.
///
/// `original` is the list that `list.sort()` detached the items from, or
/// `None` for `sorted()`, whose call answers `items`.
pub fn sort_by_key(items: Value, key: Value, reverse: bool, original: Value, vm: &mut VM<'_>) -> RunResult<CallResult> {
    vm.call_frozen_exact(
        FrozenFunction::SortByKey,
        [
            items,
            key,
            Value::Bool(reverse),
            original,
            Value::ModuleFunction(ModuleFunctions::Frozen(FrozenFunctions::OrderByKeys)),
            Value::ModuleFunction(ModuleFunctions::Frozen(FrozenFunctions::RestoreSorted)),
            Value::Builtin(Builtins::ExcType(ExcType::BaseException)),
        ],
    )
}

/// Sorts a vector of values by the values themselves, with no key.
pub fn sort_values(values: &mut [Value], reverse: bool, vm: &mut VM<'_>) -> RunResult<()> {
    let mut sort_result: RunResult<()> = Ok(());
    let mut n = 0usize;
    values.sort_by(|a, b| {
        n += 1;
        compare_values(n, a, b, reverse, &mut sort_result, vm)
    });
    sort_result
}

/// Arguments of the private helper that sorts a list by its keys.
#[derive(FromArgs)]
#[from_args(name = "order_by_keys")]
struct OrderByKeysArgs {
    #[from_args(pos_only)]
    items: Value,
    #[from_args(pos_only)]
    keys: Value,
    #[from_args(pos_only)]
    reverse: Value,
}

/// Sorts the private list `items` in place by the list `keys`, which holds
/// the key of each item at its index, for [`SORT_BY_KEY_SOURCE`].
///
/// Both lists are private to that frame, so no Python code reaches them while
/// the comparisons run.
pub(crate) fn call_order_by_keys(args: ArgValues, vm: &mut VM<'_>) -> RunResult<Value> {
    let OrderByKeysArgs { items, keys, reverse } = OrderByKeysArgs::from_args(args, vm)?;
    defer_drop!(items, vm);
    defer_drop!(keys, vm);
    let reverse = matches!(reverse, Value::Bool(true));
    let values = take_list_items(items, vm);
    defer_drop_mut!(values, vm);
    let key_values = take_list_items(keys, vm);
    defer_drop_mut!(key_values, vm);
    assert_eq!(
        values.len(),
        key_values.len(),
        "a sort by key has one key for each item"
    );
    let mut indices = (0..values.len()).collect::<Vec<_>>();
    let sorted = sort_indices(&mut indices, key_values, reverse, vm);
    apply_permutation(values, &mut indices);
    put_list_items(items, mem::take(values), vm);
    sorted.map(|()| Value::None)
}

/// Takes the items out of the list `list`, which stays empty.
pub(crate) fn take_list_items(list: &Value, vm: &mut VM<'_>) -> Vec<Value> {
    let Value::Ref(id) = *list else {
        unreachable!("a frozen sort passes lists")
    };
    let HeapReadOutput::List(list) = vm.heap.read(id) else {
        unreachable!("a frozen sort passes lists")
    };
    mem::take(list.get_mut(vm.heap).as_vec_mut())
}

/// Puts `items` into the list `list` in place of what it holds, and gives
/// back what it held.
pub(crate) fn put_list_items(list: &Value, items: Vec<Value>, vm: &mut VM<'_>) -> Vec<Value> {
    let Value::Ref(id) = *list else {
        unreachable!("a frozen sort passes lists")
    };
    let HeapReadOutput::List(list) = vm.heap.read(id) else {
        unreachable!("a frozen sort passes lists")
    };
    let has_refs = items.iter().any(|v| matches!(v, Value::Ref(_)));
    let held = mem::replace(list.get_mut(vm.heap).as_vec_mut(), items);
    if has_refs {
        list.get_mut(vm.heap).set_contains_refs();
    }
    held
}

/// Sorts a vector of indices by comparing items at those positions.
///
/// Compares `values[a]` vs `values[b]` using `py_cmp`, optionally reversing
/// the ordering. If any comparison fails (type error or runtime error), the
/// sort finishes early and the error is returned.
///
/// The `values` slice is typically either the items themselves (no key function)
/// or the pre-computed key values.
pub fn sort_indices(indices: &mut [usize], values: &[Value], reverse: bool, vm: &mut VM<'_>) -> Result<(), RunError> {
    let mut sort_result: RunResult<()> = Ok(());
    let mut n = 0usize;
    indices.sort_by(|&a, &b| {
        n += 1;
        compare_values(n, &values[a], &values[b], reverse, &mut sort_result, vm)
    });
    sort_result
}

/// Rearranges `items` in-place according to a permutation of indices.
///
/// After calling this, `items[i]` will hold the element that was originally at
/// `items[indices[i]]`. The algorithm chases permutation cycles and swaps
/// elements into their final positions, using O(1) extra memory beyond the
/// `indices` slice (which is mutated to track visited positions).
///
/// The helper is generic so callers can avoid allocating a second buffer when
/// reordering either raw `Value`s or compound structures that already own their
/// contents. Each element is moved at most twice (one swap = two moves), so
/// the total work is O(n) moves while preserving the target permutation.
pub fn apply_permutation<T>(items: &mut [T], indices: &mut [usize]) {
    for i in 0..items.len() {
        if indices[i] == i {
            continue;
        }
        let mut current = i;
        loop {
            let target = indices[current];
            indices[current] = current;
            if target == i {
                break;
            }
            items.swap(current, target);
            current = target;
        }
    }
}

/// Helper for the sort functions which compares two values, handling any exceptions and timeouts.
/// `n` is the caller's running comparison count, keying the amortized time check.
fn compare_values(
    n: usize,
    a: &Value,
    b: &Value,
    reverse: bool,
    sort_result: &mut RunResult<()>,
    vm: &mut VM<'_>,
) -> Ordering {
    if sort_result.is_err() {
        // short-circuit if we've already encountered an error in a previous comparison
        return Ordering::Equal;
    }
    if let Err(e) = vm.heap.tracker.check_time_every(n) {
        *sort_result = Err(e.into());
        return Ordering::Equal;
    }
    let err = match a.py_cmp(b, vm) {
        Ok(CmpOrder::Ordered(ord)) => return if reverse { ord.reverse() } else { ord },
        // A `NaN` (or `NaN`-carrying container) has no ordering but must not
        // raise: CPython's `sorted`/`list.sort` leave such elements wherever the
        // comparisons happen to place them. Treat it as "equal" — no swap.
        Ok(CmpOrder::Unordered) => return Ordering::Equal,
        Ok(CmpOrder::Incomparable) => ExcType::type_error(format!(
            "'<' not supported between instances of '{}' and '{}'",
            a.py_type_name(vm),
            b.py_type_name(vm)
        )),
        Err(e) => e,
    };
    *sort_result = Err(err);
    Ordering::Equal
}
