//! Implementation of the divmod() builtin function.

use num_bigint::BigInt;
use num_integer::Integer;
use smallvec::smallvec;

use crate::{
    args::ArgValues,
    bytecode::VM,
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunResult, SimpleException},
    heap::HeapData,
    resource_checks::check_div_size,
    types::{LongInt, allocate_tuple, long_int_as_f64},
    value::{Value, floor_divmod, immediate_int_value, py_float_floordiv, py_float_mod},
};

/// Implementation of the divmod() builtin function.
///
/// Returns a tuple (quotient, remainder) from integer division.
/// Equivalent to (a // b, a % b).
pub fn builtin_divmod(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let (a, b) = args.get_two_args("divmod", vm.heap)?;
    let a = immediate_int_value(a);
    let b = immediate_int_value(b);
    defer_drop!(a, vm);
    defer_drop!(b, vm);

    match (a, b) {
        (Value::Int(x), Value::Int(y)) => {
            if *y == 0 {
                Err(ExcType::divmod_by_zero())
            } else if let Some((quot, rem)) = floor_divmod(*x, *y) {
                Ok(allocate_tuple(smallvec![Value::Int(quot), Value::Int(rem)], vm.heap))
            } else {
                // Overflow - promote to BigInt
                check_div_size(64, &vm.heap.tracker)?;
                let (quot, rem) = bigint_floor_divmod(&BigInt::from(*x), &BigInt::from(*y));
                let quot_val = LongInt::new(quot).into_value(vm.heap);
                let rem_val = LongInt::new(rem).into_value(vm.heap);
                Ok(allocate_tuple(smallvec![quot_val, rem_val], vm.heap))
            }
        }
        (Value::Int(x), Value::Ref(id)) if let HeapData::LongInt(li) = vm.heap.get(*id) => {
            if li.is_zero() {
                Err(ExcType::divmod_by_zero())
            } else {
                let x_bi = BigInt::from(*x);
                let (quot, rem) = bigint_floor_divmod(&x_bi, li.inner());
                let quot_val = LongInt::new(quot).into_value(vm.heap);
                let rem_val = LongInt::new(rem).into_value(vm.heap);
                Ok(allocate_tuple(smallvec![quot_val, rem_val], vm.heap))
            }
        }
        (Value::Ref(id), Value::Int(y)) if let HeapData::LongInt(li) = vm.heap.get(*id) => {
            if *y == 0 {
                Err(ExcType::divmod_by_zero())
            } else {
                let y_bi = BigInt::from(*y);
                let (quot, rem) = bigint_floor_divmod(li.inner(), &y_bi);
                let quot_val = LongInt::new(quot).into_value(vm.heap);
                let rem_val = LongInt::new(rem).into_value(vm.heap);
                Ok(allocate_tuple(smallvec![quot_val, rem_val], vm.heap))
            }
        }
        (Value::Ref(id1), Value::Ref(id2))
            if let HeapData::LongInt(x_li) = vm.heap.get(*id1)
                && let HeapData::LongInt(y_li) = vm.heap.get(*id2) =>
        {
            if y_li.is_zero() {
                Err(ExcType::divmod_by_zero())
            } else {
                let (quot, rem) = bigint_floor_divmod(x_li.inner(), y_li.inner());
                let quot_val = LongInt::new(quot).into_value(vm.heap);
                let rem_val = LongInt::new(rem).into_value(vm.heap);
                Ok(allocate_tuple(smallvec![quot_val, rem_val], vm.heap))
            }
        }
        (Value::Float(x), Value::Float(y)) => float_divmod(*x, *y, vm),
        (Value::Int(x), Value::Float(y)) => float_divmod(*x as f64, *y, vm),
        (Value::Float(x), Value::Int(y)) => float_divmod(*x, *y as f64, vm),
        (Value::Ref(id), Value::Float(y)) if let HeapData::LongInt(li) = vm.heap.get(*id) => {
            let x = long_int_as_f64(li)?;
            float_divmod(x, *y, vm)
        }
        (Value::Float(x), Value::Ref(id)) if let HeapData::LongInt(li) = vm.heap.get(*id) => {
            let x = *x;
            let y = long_int_as_f64(li)?;
            float_divmod(x, y, vm)
        }
        _ => {
            let a_type = a.py_type_name(vm);
            let b_type = b.py_type_name(vm);
            Err(SimpleException::new_msg(
                ExcType::TypeError,
                format!("unsupported operand type(s) for divmod(): '{a_type}' and '{b_type}'"),
            )
            .into())
        }
    }
}

/// Computes Python-style floor division and modulo for BigInts.
///
/// Uses `div_mod_floor` from num_integer for correct floor semantics.
fn bigint_floor_divmod(a: &BigInt, b: &BigInt) -> (BigInt, BigInt) {
    a.div_mod_floor(b)
}

/// `divmod` over a pair already converted to floats, whichever operands it came
/// from: an int mixed with a float is converted and then divided as floats.
fn float_divmod(a: f64, b: f64, vm: &mut VM<'_>) -> RunResult<Value> {
    if b == 0.0 {
        return Err(ExcType::divmod_by_zero());
    }
    let quotient = py_float_floordiv(a, b);
    // `a - quotient * b` cancels catastrophically once `a` is large: it answers
    // `0.0` for `divmod(2**64, 1.5)` where the remainder is `1.0`. Taking it
    // from `%` instead keeps `divmod(a, b)` equal to `(a // b, a % b)`.
    let remainder = py_float_mod(a, b);
    Ok(allocate_tuple(
        smallvec![Value::Float(quotient), Value::Float(remainder)],
        vm.heap,
    ))
}
