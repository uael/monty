//! Implementation of the issubclass() builtin function.

use super::Builtins;
use crate::{
    args::ArgValues,
    bytecode::VM,
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunResult},
    heap::{HeapData, HeapId},
    types::{class::class_exc_base, class_is_subclass},
    value::Value,
};

/// Implementation of the issubclass() builtin function.
///
/// `issubclass(C, B)` walks `C`'s base chain, and matches a builtin exception
/// class through the nearest builtin ancestor `C` reaches. As in CPython, a
/// non-class first argument is a `TypeError`, and the second may be a flat
/// tuple of classes.
pub fn builtin_issubclass(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let (class, classinfo) = args.get_two_args("issubclass", vm.heap)?;
    defer_drop!(class, vm);
    defer_drop!(classinfo, vm);

    let Some(class_id) = class_id_of(class, vm) else {
        return Err(ExcType::type_error("issubclass() arg 1 must be a class"));
    };
    match classinfo {
        Value::Ref(id) if let HeapData::Tuple(tuple) = vm.heap.get(*id) => {
            let mut matched = false;
            for entry in tuple.as_slice() {
                let Some(hit) = subclass_of(class_id, entry, vm) else {
                    return Err(ExcType::isinstance_arg2_error());
                };
                matched |= hit;
            }
            Ok(Value::Bool(matched))
        }
        single => subclass_of(class_id, single, vm)
            .map(Value::Bool)
            .ok_or_else(ExcType::isinstance_arg2_error),
    }
}

/// The class object `value` names, or `None` when it is not a sandbox class.
///
/// A builtin exception type has no class object, so `issubclass(ValueError, …)`
/// is rejected rather than answered; see `limitations/classes.md`.
fn class_id_of(value: &Value, vm: &VM<'_>) -> Option<HeapId> {
    match value {
        Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::Class(_)) => Some(*id),
        _ => None,
    }
}

/// Whether `class_id` is a subclass of the single class `handler`; `None` when
/// `handler` is not a class at all.
fn subclass_of(class_id: HeapId, handler: &Value, vm: &VM<'_>) -> Option<bool> {
    match handler {
        Value::Builtin(Builtins::ExcType(base)) => {
            Some(class_exc_base(class_id, vm).is_some_and(|own| own.is_subclass_of(*base)))
        }
        Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::Class(_)) => Some(class_is_subclass(class_id, *id, vm)),
        _ => None,
    }
}
