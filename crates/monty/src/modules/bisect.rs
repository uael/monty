//! Binary search and sorted insertion, matching CPython's `_bisect` accelerator.

use crate::{
    args::{ArgValues, FromArgs},
    bytecode::{CallResult, VM},
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunResult},
    expressions::CmpOperator,
    frozen::{FrozenFunction, FrozenFunctions},
    heap::{DropGuard, DropWithContext, HeapData, HeapId, HeapReadOutput},
    intern::StaticStrings,
    modules::ModuleFunctions,
    types::{Module, PyTrait, Type},
    value::{EitherStr, Value},
};

/// The aliases share the same callable value as their canonical function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::Display, serde::Serialize, serde::Deserialize)]
#[strum(serialize_all = "snake_case")]
pub(crate) enum BisectFunctions {
    BisectLeft,
    BisectRight,
    InsortLeft,
    InsortRight,
}

/// Creates the module and its two aliases.
pub fn create_module(vm: &mut VM<'_>) -> HeapId {
    let mut module = Module::new(StaticStrings::Bisect, vm.interns);
    for (name, function) in [
        (StaticStrings::BisectLeft, BisectFunctions::BisectLeft),
        (StaticStrings::BisectRight, BisectFunctions::BisectRight),
        (StaticStrings::Bisect, BisectFunctions::BisectRight),
        (StaticStrings::InsortLeft, BisectFunctions::InsortLeft),
        (StaticStrings::InsortRight, BisectFunctions::InsortRight),
        (StaticStrings::Insort, BisectFunctions::InsortRight),
    ] {
        module.set_attr(name, Value::ModuleFunction(ModuleFunctions::Bisect(function)), vm);
    }
    vm.heap.allocate(HeapData::Module(Box::new(module)))
}

/// Clinic's nullable slice-index conversion for `hi`; -1 means the length,
/// even when explicitly supplied. Other negative values are not normalized.
fn convert_hi(value: &Value, vm: &mut VM<'_>) -> RunResult<i64> {
    if matches!(value, Value::None) {
        return Ok(-1);
    }
    let Some(index) = value.py_index_impl(vm)? else {
        return Err(ExcType::type_error(format!(
            "argument should be integer or None, not '{}'",
            value.py_type_name(vm)
        )));
    };
    // The index protocol validated an integer; only overflow can fail here.
    // CPython names the original object, including a user __index__ provider.
    let result = index.as_int(vm).map_err(|_| {
        ExcType::overflow_error(format!(
            "cannot fit '{}' into an index-sized integer",
            value.py_type_name(vm)
        ))
    });
    index.drop_with(vm);
    result
}

// Separate derives preserve each function's name in Clinic binding errors.
macro_rules! arguments {
    ($ty:ident, $name:literal) => {
        #[derive(FromArgs)]
        #[from_args(name = $name, style = c_named)]
        struct $ty {
            a: Value,
            x: Value,
            #[from_args(default = Value::Int(0))]
            lo: Value,
            #[from_args(default = Value::None)]
            hi: Value,
            #[from_args(kw_only, default = Value::None)]
            key: Value,
        }
    };
}
arguments!(BisectLeftArgs, "bisect_left");
arguments!(BisectRightArgs, "bisect_right");
arguments!(InsortLeftArgs, "insort_left");
arguments!(InsortRightArgs, "insort_right");

/// The search with a key, which runs on the VM's own loop as a frame of its
/// own, so a key that calls the host suspends the run.
///
/// It follows CPython's order: the key of `x` first for an insort, then the
/// checks of the bounds, then one key for each probe.
pub(crate) const BISECT_KEY_SOURCE: &str = r"
def __monty_bisect_key(a, x, lo, hi, key, left, insert, bounds, less):
    x_key = key(x) if insert else x
    hi = bounds(a, lo, hi)
    while lo < hi:
        mid = lo + (hi - lo) // 2
        item_key = key(a[mid])
        if left:
            if less(item_key, x_key):
                lo = mid + 1
            else:
                hi = mid
        elif less(x_key, item_key):
            hi = mid
        else:
            lo = mid + 1
    if insert:
        a.insert(lo, x)
        return None
    return lo
";

/// Dispatch with shared search logic and CPython's key evaluation order.
///
/// A key runs in [`BISECT_KEY_SOURCE`], a frame on the VM's own loop.
pub(super) fn call(vm: &mut VM<'_>, function: BisectFunctions, args: ArgValues) -> RunResult<CallResult> {
    let (a, x, lo, hi, key) = match function {
        BisectFunctions::BisectLeft => {
            let BisectLeftArgs { a, x, lo, hi, key } = BisectLeftArgs::from_args(args, vm)?;
            (a, x, lo, hi, key)
        }
        BisectFunctions::BisectRight => {
            let BisectRightArgs { a, x, lo, hi, key } = BisectRightArgs::from_args(args, vm)?;
            (a, x, lo, hi, key)
        }
        BisectFunctions::InsortLeft => {
            let InsortLeftArgs { a, x, lo, hi, key } = InsortLeftArgs::from_args(args, vm)?;
            (a, x, lo, hi, key)
        }
        BisectFunctions::InsortRight => {
            let InsortRightArgs { a, x, lo, hi, key } = InsortRightArgs::from_args(args, vm)?;
            (a, x, lo, hi, key)
        }
    };
    let mut held = DropGuard::new(((a, x), (key, (lo, hi))), vm);
    let ((_, (_, (lo, hi))), vm) = held.as_parts();
    // Clinic validates the complete signature before converting lo, then hi.
    // Keeping raw fields in FromArgs makes unknown/duplicate kwargs win over
    // conversion errors, unlike PyArg_ParseTupleAndKeywords-style extraction.
    let lo = lo.as_int(vm)?;
    let hi = convert_hi(hi, vm)?;
    let insert = matches!(function, BisectFunctions::InsortLeft | BisectFunctions::InsortRight);
    let left = matches!(function, BisectFunctions::BisectLeft | BisectFunctions::InsortLeft);
    if !matches!(held.as_parts().0.1.0, Value::None) {
        let (((a, x), (key, bounds)), vm) = held.into_parts();
        bounds.drop_with(vm);
        return vm.call_frozen_exact(
            FrozenFunction::BisectKey,
            [
                a,
                x,
                Value::Int(lo),
                Value::Int(hi),
                key,
                Value::Bool(left),
                Value::Bool(insert),
                Value::ModuleFunction(ModuleFunctions::Frozen(FrozenFunctions::BisectBounds)),
                Value::ModuleFunction(ModuleFunctions::Frozen(FrozenFunctions::BisectLess)),
            ],
        );
    }
    let (((a, x), _), vm) = held.as_parts();
    let index = search(vm, a, x, (lo, hi), left)?;
    if insert {
        let Value::Ref(_) = a else {
            return Err(ExcType::attribute_error(a.py_type_name(vm), "insert"));
        };
        let argument = x.clone_with_heap(vm);
        let result = a.py_value(vm).py_call_attr(
            vm,
            &EitherStr::Interned(vm.interns.intern_static(StaticStrings::Insert)),
            ArgValues::Two(Value::Int(index), argument),
        )?;
        let CallResult::Value(result) = result else {
            unreachable!("built-in sequence insert methods are synchronous");
        };
        result.drop_with(vm);
        Ok(CallResult::Value(Value::None))
    } else {
        Ok(CallResult::Value(Value::Int(index)))
    }
}

/// Search with no key, which calls no Python but the comparisons.
fn search(vm: &mut VM<'_>, a: &Value, x: &Value, bounds: (i64, i64), left: bool) -> RunResult<i64> {
    let (mut lo, hi) = bounds;
    let mut hi = checked_bounds(vm, a, lo, hi)?;
    let mut steps = 0;
    while lo < hi {
        vm.heap.tracker.check_time_every(steps)?;
        steps += 1;
        let mid = lo + (hi - lo) / 2;
        let item = a.py_getitem(&Value::Int(mid), vm)?;
        defer_drop!(item, vm);
        let less = if left {
            less_than(vm, item, x)?
        } else {
            less_than(vm, x, item)?
        };
        if less == left {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    Ok(lo)
}

/// Arguments of the private helper that checks the bounds of a search.
#[derive(FromArgs)]
#[from_args(name = "bounds")]
struct BoundsArgs {
    #[from_args(pos_only)]
    a: Value,
    #[from_args(pos_only)]
    lo: Value,
    #[from_args(pos_only)]
    hi: Value,
}

/// Checks the bounds of a search with a key and gives its `hi`, for
/// [`BISECT_KEY_SOURCE`]; see [`checked_bounds`].
pub(crate) fn call_bounds(args: ArgValues, vm: &mut VM<'_>) -> RunResult<Value> {
    let BoundsArgs { a, lo, hi } = BoundsArgs::from_args(args, vm)?;
    defer_drop!(a, vm);
    let (Value::Int(lo), Value::Int(hi)) = (lo, hi) else {
        unreachable!("a frozen search passes its bounds as ints")
    };
    checked_bounds(vm, a, lo, hi).map(Value::Int)
}

/// Arguments of the private helper that compares two keys of a search.
#[derive(FromArgs)]
#[from_args(name = "less")]
struct LessArgs {
    #[from_args(pos_only)]
    a: Value,
    #[from_args(pos_only)]
    b: Value,
}

/// Whether `a < b` as a search compares, for [`BISECT_KEY_SOURCE`]; see [`less_than`].
pub(crate) fn call_less(args: ArgValues, vm: &mut VM<'_>) -> RunResult<Value> {
    let LessArgs { a, b } = LessArgs::from_args(args, vm)?;
    defer_drop!(a, vm);
    defer_drop!(b, vm);
    less_than(vm, a, b).map(Value::Bool)
}

/// Checks `lo` and the sequence as `_bisect` does before a search, and gives
/// `hi`, with -1 read as the length of `a`.
fn checked_bounds(vm: &mut VM<'_>, a: &Value, lo: i64, hi: i64) -> RunResult<i64> {
    let mut hi = hi;
    if lo < 0 {
        return Err(ExcType::value_error("lo must be non-negative"));
    }
    if hi == -1 {
        // Ranges can exceed usize without allocating elements. Use their exact
        // length so Monty's fixed i64 index limit also applies on 32-bit wasm.
        hi = if let Some(HeapReadOutput::Range(range)) = a.read_heap(vm) {
            i64::try_from(range.get(vm.heap).len_i128()).map_err(|_| ExcType::overflow_c_ssize_t())?
        } else {
            let len = a
                .py_len(vm)
                .ok_or_else(|| ExcType::type_error(format!("object of type '{}' has no len()", a.py_type_name(vm))))?;
            i64::try_from(len).map_err(|_| ExcType::overflow_c_ssize_t())?
        };
    }
    // _bisect obtains sq_item even when the search interval is empty.
    match a.py_type(vm) {
        Type::List
        | Type::Tuple
        | Type::NamedTuple
        | Type::Str
        | Type::Bytes
        | Type::Range
        | Type::Deque
        | Type::Counter => {}
        Type::Dict | Type::DefaultDict => {
            return Err(ExcType::type_error(format!("{} is not a sequence", a.py_type_name(vm))));
        }
        _ => {
            return Err(ExcType::type_error(format!(
                "'{}' object does not support indexing",
                a.py_type_name(vm)
            )));
        }
    }
    Ok(hi)
}

/// Lists and tuples compare the first unequal pair with `<`, rather than
/// deriving `<` from a three-way ordering. This preserves identity/equality
/// shortcuts (including NaNs) and reports errors on the offending elements.
fn less_than(vm: &mut VM<'_>, a: &Value, b: &Value) -> RunResult<bool> {
    if !matches!(
        (a.py_type(vm), b.py_type(vm)),
        (Type::List, Type::List) | (Type::Tuple | Type::NamedTuple, Type::Tuple | Type::NamedTuple)
    ) {
        return vm.cmp_values(CmpOperator::Lt, a, b);
    }
    let mut guard = vm.recursion_guard()?;
    let vm = &mut *guard;
    let mut i = 0;
    loop {
        let a_len = a.py_len(vm).expect("sequence has length");
        let b_len = b.py_len(vm).expect("sequence has length");
        if i >= a_len.min(b_len) {
            return Ok(a_len < b_len);
        }
        vm.heap.tracker.check_time_every(i)?;
        let index = Value::Int(i64::try_from(i).expect("sequence index fits i64"));
        let av = a.py_getitem(&index, vm)?;
        defer_drop!(av, vm);
        let bv = b.py_getitem(&index, vm)?;
        defer_drop!(bv, vm);
        if !av.py_eq(bv, vm)? {
            return less_than(vm, av, bv);
        }
        i += 1;
    }
}
