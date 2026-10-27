//! Implementation of the min() and max() builtin functions.

use std::{cmp::Ordering, mem};

use crate::{
    args::{ArgValues, FromArgs},
    builtins::Builtins,
    bytecode::{CallResult, VM},
    defer_drop, defer_drop_mut,
    exception_private::{ExcType, ExcTypeExt, RunError, RunResult, SimpleException},
    frozen::{FrozenFunction, FrozenFunctions},
    heap::{DropGuard, DropWithContext},
    modules::ModuleFunctions,
    types::{CmpOrder, PyTrait, allocate_tuple, tuple::TupleVec},
    value::Value,
};

/// The loop of `min()` and `max()` with a key, which runs on the VM's own loop
/// as a frame of its own.
///
/// So the key and the read of the items step there, and a call to the host in
/// either suspends the run. `items` is the one iterable, or a tuple of the
/// positional arguments, which is never empty.
pub(crate) const MIN_MAX_KEY_SOURCE: &str = r"
def __monty_min_max_key(items, key, default, has_default, is_min, wins, value_error):
    first = True
    best = None
    best_key = None
    for item in items:
        item_key = key(item)
        if first:
            first = False
            best = item
            best_key = item_key
        elif wins(best_key, item_key, is_min):
            best = item
            best_key = item_key
    if not first:
        return best
    if has_default:
        return default
    raise value_error(('min' if is_min else 'max') + '() iterable argument is empty')
";

/// Implementation of the min() builtin function.
///
/// Returns the smallest item in an iterable or the smallest of two or more arguments.
/// Supports two forms:
/// - `min(iterable)` - returns smallest item from iterable
/// - `min(arg1, arg2, ...)` - returns smallest of the arguments
pub fn builtin_min(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let MinArgs { args, key, default } = MinArgs::from_args(args, vm)?;
    run_min_max(vm, args, key, default, true)
}

/// Implementation of the max() builtin function.
///
/// Returns the largest item in an iterable or the largest of two or more arguments.
/// Supports two forms:
/// - `max(iterable)` - returns largest item from iterable
/// - `max(arg1, arg2, ...)` - returns largest of the arguments
pub fn builtin_max(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let MaxArgs { args, key, default } = MaxArgs::from_args(args, vm)?;
    run_min_max(vm, args, key, default, false)
}

/// Shared implementation for min() and max() after argument extraction.
///
/// When `is_min` is true, returns the minimum; otherwise returns the maximum.
/// A key runs in [`MIN_MAX_KEY_SOURCE`], a frame on the VM's own loop, so it
/// may call the host.
fn run_min_max(
    vm: &mut VM<'_>,
    mut args: Vec<Value>,
    key: Value,
    default: Option<Value>,
    is_min: bool,
) -> RunResult<CallResult> {
    let func_name = if is_min { "min" } else { "max" };

    if args.is_empty() {
        ((args, key), default).drop_with(vm);
        return Err(SimpleException::new_msg(
            ExcType::TypeError,
            format!("{func_name} expected at least 1 argument, got 0"),
        )
        .into());
    }
    if args.len() > 1 && default.is_some() {
        ((args, key), default).drop_with(vm);
        return Err(default_with_multiple_args(func_name));
    }
    if !matches!(key, Value::None) {
        let items = if args.len() == 1 {
            args.pop().expect("checked one argument")
        } else {
            allocate_tuple(TupleVec::from_vec(args), vm.heap)
        };
        let has_default = default.is_some();
        return vm.call_frozen_exact(
            FrozenFunction::MinMaxKey,
            [
                items,
                key,
                default.unwrap_or(Value::None),
                Value::Bool(has_default),
                Value::Bool(is_min),
                Value::ModuleFunction(ModuleFunctions::Frozen(FrozenFunctions::KeyWins)),
                Value::Builtin(Builtins::ExcType(ExcType::ValueError)),
            ],
        );
    }
    min_max_by_value(vm, args, default, is_min).map(CallResult::Value)
}

/// Arguments of the private helper that compares two keys of `min()` or `max()`.
#[derive(FromArgs)]
#[from_args(name = "key_wins")]
struct KeyWinsArgs {
    #[from_args(pos_only)]
    best: Value,
    #[from_args(pos_only)]
    candidate: Value,
    #[from_args(pos_only)]
    is_min: Value,
}

/// Whether `candidate` wins over `best` as a key of `min()` or `max()`, for
/// [`MIN_MAX_KEY_SOURCE`]; see [`candidate_wins`].
pub(crate) fn call_key_wins(args: ArgValues, vm: &mut VM<'_>) -> RunResult<Value> {
    let KeyWinsArgs {
        best,
        candidate,
        is_min,
    } = KeyWinsArgs::from_args(args, vm)?;
    defer_drop!(best, vm);
    defer_drop!(candidate, vm);
    let is_min = matches!(is_min, Value::Bool(true));
    candidate_wins(best, candidate, is_min, vm).map(Value::Bool)
}

/// `min()` and `max()` with no key: the items compare as they are.
fn min_max_by_value(vm: &mut VM<'_>, args: Vec<Value>, default: Option<Value>, is_min: bool) -> RunResult<Value> {
    let func_name = if is_min { "min" } else { "max" };
    defer_drop_mut!(default, vm);
    defer_drop_mut!(args, vm);

    let first_arg = args.remove(0);

    if args.is_empty() {
        // Single argument: iterate over it
        let iter = first_arg.into_py_iter(vm)?.require_synchronous(vm)?;
        defer_drop!(iter, vm);
        let mut iter = iter.read(vm);

        let Some(result) = iter.py_next(vm)? else {
            if let Some(default) = default.take() {
                return Ok(default);
            }
            return Err(SimpleException::new_msg(
                ExcType::ValueError,
                format!("{func_name}() iterable argument is empty"),
            )
            .into());
        };

        let mut result_guard = DropGuard::new(result, vm);
        let (result, vm) = result_guard.as_parts_mut();

        while let Some(item) = iter.py_next(vm)? {
            defer_drop_mut!(item, vm);

            if candidate_wins(result, item, is_min, vm)? {
                mem::swap(result, item);
            }
        }

        Ok(result_guard.into_inner())
    } else {
        // Multiple arguments: compare them directly
        let mut result_guard = DropGuard::new(first_arg, vm);
        let (result, vm) = result_guard.as_parts_mut();

        for item in args.drain(..) {
            defer_drop_mut!(item, vm);

            if candidate_wins(result, item, is_min, vm)? {
                mem::swap(result, item);
            }
        }

        Ok(result_guard.into_inner())
    }
}

/// Argument shape for `min(*args, key=None, default=...)`.
///
/// `key` is held as `Value` so `key=None` can be normalised to "no key
/// function". `default` is `Option<Value>` (with `default` attr) so the
/// implementation can distinguish "not provided" from "provided with any
/// value" — that distinction matters because `default=` is only valid with a
/// single iterable argument.
#[derive(FromArgs)]
#[from_args(name = "min")]
struct MinArgs {
    #[from_args(varargs)]
    args: Vec<Value>,
    #[from_args(default = Value::None)]
    key: Value,
    #[from_args(default)]
    default: Option<Value>,
}

/// Argument shape for `max(*args, key=None, default=...)`.
///
/// See [`MinArgs`] for field semantics; the only difference is the function
/// name used in error messages.
#[derive(FromArgs)]
#[from_args(name = "max")]
struct MaxArgs {
    #[from_args(varargs)]
    args: Vec<Value>,
    #[from_args(default = Value::None)]
    key: Value,
    #[from_args(default)]
    default: Option<Value>,
}

/// Returns whether `candidate` should replace `current` as the best value seen so far.
///
/// `min()` replaces the current winner when the new candidate compares smaller,
/// while `max()` replaces it when the new candidate compares larger. Equal values
/// keep the existing winner so ties preserve the first-seen item, matching CPython.
pub(crate) fn candidate_wins(current: &Value, candidate: &Value, is_min: bool, vm: &mut VM<'_>) -> RunResult<bool> {
    let ordering = match candidate.py_cmp(current, vm)? {
        CmpOrder::Ordered(ordering) => ordering,
        // A `NaN` candidate (or `NaN`-carrying container) is neither smaller nor
        // larger, so it never displaces the incumbent — matching CPython, where
        // `min`/`max` only swap on a strict `<`/`>` and `NaN` yields neither.
        CmpOrder::Unordered => return Ok(false),
        CmpOrder::Incomparable => return Err(ord_not_supported(candidate, current, is_min, vm)),
    };

    Ok((is_min && ordering == Ordering::Less) || (!is_min && ordering == Ordering::Greater))
}

/// Creates the CPython-compatible error for `default=` with multiple positional args.
#[cold]
fn default_with_multiple_args(func_name: &str) -> RunError {
    SimpleException::new_msg(
        ExcType::TypeError,
        format!("Cannot specify a default for {func_name}() with multiple positional arguments"),
    )
    .into()
}

#[cold]
fn ord_not_supported(left: &Value, right: &Value, is_min: bool, vm: &VM<'_>) -> RunError {
    let left_type = left.py_type_name(vm);
    let right_type = right.py_type_name(vm);
    let operator = if is_min { "<" } else { ">" };
    ExcType::type_error_ordering(operator, &left_type, &right_type)
}
