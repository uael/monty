//! Implementation of the issubclass() builtin function.

use super::Builtins;
use crate::{
    args::ArgValues,
    bytecode::VM,
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunResult},
    heap::{HeapData, HeapId},
    types::{
        Type,
        class::class_exc_base,
        class_is_subclass,
        native_class::{class_derives_from, class_has_structural_members},
        protocol::{ProtocolCheck, protocol_check, protocol_check_refused, protocol_subclass_of},
    },
    value::Value,
};

/// Implementation of the issubclass() builtin function.
///
/// `issubclass(C, B)` walks `C`'s base chain, and matches a builtin exception
/// class through the nearest builtin ancestor `C` reaches. A builtin type is
/// also accepted as the first argument, so `issubclass(dict, Mapping)` and
/// `issubclass(bool, int)` are answerable. As in CPython, a non-class first
/// argument is a `TypeError`, and the second may be a flat tuple of classes.
pub fn builtin_issubclass(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let (class, classinfo) = args.get_two_args("issubclass", vm.heap)?;
    defer_drop!(class, vm);
    defer_drop!(classinfo, vm);

    let Some(subject) = subject_of(class, vm) else {
        return Err(ExcType::type_error("issubclass() arg 1 must be a class"));
    };
    match classinfo {
        Value::Ref(id) if let HeapData::Tuple(tuple) = vm.heap.get(*id) => {
            let entries: Vec<Value> = tuple.as_slice().iter().map(|v| v.clone_with_heap(vm.heap)).collect();
            defer_drop!(entries, vm);
            let mut matched = false;
            for entry in entries {
                let Some(hit) = subclass_of(subject, entry, vm)? else {
                    return Err(ExcType::isinstance_arg2_error());
                };
                matched |= hit;
            }
            Ok(Value::Bool(matched))
        }
        single => subclass_of(subject, single, vm)?
            .map(Value::Bool)
            .ok_or_else(ExcType::isinstance_arg2_error),
    }
}

/// What `issubclass` can ask questions about: a sandbox class, or a builtin
/// type standing for itself.
#[derive(Clone, Copy)]
enum Subject {
    /// A class defined in the sandbox, identified by its class object.
    Sandbox(HeapId),
    /// A builtin type, which has no class object of its own.
    Builtin(Type),
}

/// The subject `value` names, or `None` when it is not a class at all.
///
/// A builtin exception type is a class like any other: the name evaluates to
/// its own form rather than a `Type`, which is the only reason it needs an arm
/// of its own here.
fn subject_of(value: &Value, vm: &VM<'_>) -> Option<Subject> {
    match value {
        Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::Class(_)) => Some(Subject::Sandbox(*id)),
        Value::Builtin(Builtins::Type(t)) => Some(Subject::Builtin(*t)),
        Value::Builtin(Builtins::ExcType(exc)) => Some(Subject::Builtin(Type::Exception(*exc))),
        _ => None,
    }
}

/// Whether `subject` is a subclass of the single class `handler`; `Ok(None)`
/// when `handler` is not a class at all, and an error when it is a protocol
/// that refuses the question.
fn subclass_of(subject: Subject, handler: &Value, vm: &mut VM<'_>) -> RunResult<Option<bool>> {
    Ok(match (subject, handler) {
        // An abstract class answers for the type itself, exactly as it would
        // for one of its instances.
        (subject, Value::Builtin(Builtins::Type(Type::Native(native)))) => Some(match subject {
            Subject::Sandbox(class_id) => {
                class_derives_from(class_id, *native, vm) || class_has_structural_members(class_id, *native, vm)
            }
            Subject::Builtin(t) => native.registers_type(t),
        }),
        (Subject::Sandbox(class_id), Value::Builtin(Builtins::ExcType(base))) => {
            Some(class_exc_base(class_id, vm).is_some_and(|own| own.is_subclass_of(*base)))
        }
        (Subject::Sandbox(class_id), Value::Ref(id)) if matches!(vm.heap.get(*id), HeapData::Class(_)) => {
            match protocol_check(*id, vm) {
                ProtocolCheck::Ordinary => Some(class_is_subclass(class_id, *id, vm)),
                ProtocolCheck::Structural => {
                    Some(class_is_subclass(class_id, *id, vm) || protocol_subclass_of(class_id, *id, vm))
                }
                ProtocolCheck::Refused => return Err(protocol_check_refused()),
            }
        }
        // Both sides an exception class: the builtin hierarchy answers it.
        (Subject::Builtin(Type::Exception(sub)), Value::Builtin(Builtins::ExcType(base))) => {
            Some(sub.is_subclass_of(*base))
        }
        (Subject::Builtin(t), Value::Builtin(Builtins::Type(base))) => Some(t.is_instance_of(*base)),
        // Any other builtin type is never a sandbox class, and never an
        // exception class Monty can place in the hierarchy.
        (Subject::Builtin(_), Value::Builtin(Builtins::ExcType(_))) => Some(false),
        (Subject::Builtin(_), Value::Ref(id)) if matches!(vm.heap.get(*id), HeapData::Class(_)) => {
            match protocol_check(*id, vm) {
                ProtocolCheck::Refused => return Err(protocol_check_refused()),
                _ => Some(false),
            }
        }
        _ => None,
    })
}
