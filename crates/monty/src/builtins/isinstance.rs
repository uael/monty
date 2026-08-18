//! Implementation of the isinstance() builtin function.

use super::{Builtins, BuiltinsFunctions};
use crate::{
    args::ArgValues,
    bytecode::VM,
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunResult},
    heap::{HeapData, HeapId, HeapRead, HeapReadOutput},
    types::{PyTrait, Tuple, Type, class_is_subclass, instance::instance_exc_base},
    value::Value,
};

/// Implementation of the isinstance() builtin function.
///
/// Checks if an object is an instance of a class or a tuple of classes.
pub fn builtin_isinstance(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let (obj, classinfo) = args.get_two_args("isinstance", vm.heap)?;
    defer_drop!(obj, vm);
    defer_drop!(classinfo, vm);

    isinstance_check(obj, classinfo, vm).map(Value::Bool)
}

/// Checks if `obj` matches a single classinfo entry.
///
/// Supports:
/// - Single builtin types: `isinstance(x, int)`
/// - Exception types and their hierarchy: `isinstance(err, LookupError)`
/// - User-defined classes: `isinstance(obj, Foo)`, walking the instance's base
///   chain, so a subclass instance matches its bases
/// - Tuples (possibly nested) of the above
fn isinstance_check(obj: &Value, classinfo: &Value, vm: &mut VM<'_>) -> RunResult<bool> {
    match classinfo {
        // `type` asks whether the object *is* a class, not what class it is.
        Value::Builtin(Builtins::Function(BuiltinsFunctions::Type)) => Ok(is_class_object(obj, vm)),
        // The two descriptor wrappers reach here as builtin functions rather
        // than `Type` values, since that is what their names resolve to.
        Value::Builtin(Builtins::Function(BuiltinsFunctions::Staticmethod)) => {
            Ok(obj.py_type(vm).is_instance_of(Type::StaticMethod))
        }
        Value::Builtin(Builtins::Function(BuiltinsFunctions::Classmethod)) => {
            Ok(obj.py_type(vm).is_instance_of(Type::ClassMethod))
        }
        Value::Builtin(Builtins::Type(t)) => Ok(obj.py_type(vm).is_instance_of(*t)),
        Value::Builtin(Builtins::ExcType(handler_type)) => Ok(exception_instance_of(obj, *handler_type, vm)),
        // A user-defined class: true iff `obj` is an instance of exactly this class.
        Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::Class(_)) => Ok(instance_of_class(obj, *id, vm)),
        // A `collections.namedtuple` class, matched by the instance's `class_id`.
        Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::NamedTupleClass(_)) => {
            Ok(instance_of_namedtuple_class(obj, *id, vm))
        }
        Value::Ref(id) if let HeapReadOutput::Tuple(tuple) = vm.heap.read(*id) => {
            isinstance_check_tuple(obj, &tuple, vm)
        }
        _ => Err(ExcType::isinstance_arg2_error()),
    }
}

/// Whether `obj` is a class object, which is what `isinstance(x, type)` asks.
///
/// Every builtin type and exception type is one, as is every class the sandbox
/// defined; `iter` is excluded because CPython's is a function rather than the
/// iterator class Monty resolves the name to.
fn is_class_object(obj: &Value, vm: &VM<'_>) -> bool {
    match obj {
        Value::Builtin(Builtins::Type(Type::Iterator)) => false,
        Value::Builtin(Builtins::Type(_) | Builtins::ExcType(_)) => true,
        Value::Builtin(Builtins::Function(f)) => matches!(
            f,
            BuiltinsFunctions::Type
                | BuiltinsFunctions::Classmethod
                | BuiltinsFunctions::Staticmethod
                | BuiltinsFunctions::Enumerate
                | BuiltinsFunctions::Super
        ),
        Value::Ref(id) => matches!(vm.heap.get(*id), HeapData::Class(_) | HeapData::NamedTupleClass(_)),
        _ => false,
    }
}

/// Whether `obj` is an instance of `class_id` or of one of its subclasses.
fn instance_of_class(obj: &Value, class_id: HeapId, vm: &VM<'_>) -> bool {
    matches!(obj, Value::Ref(obj_id) if matches!(vm.heap.get(*obj_id), HeapData::Instance(inst) if class_is_subclass(inst.class(), class_id, vm)))
}

/// Whether `obj` is an exception caught by the builtin class `handler_type`:
/// a builtin exception through the hard-coded hierarchy, or a sandbox-defined
/// one through the nearest builtin ancestor its class chain reaches.
fn exception_instance_of(obj: &Value, handler_type: ExcType, vm: &VM<'_>) -> bool {
    match obj.py_type(vm) {
        Type::Exception(exc_type) => exc_type.is_subclass_of(handler_type),
        Type::Instance(_) => match obj {
            Value::Ref(id) => instance_exc_base(*id, vm).is_some_and(|base| base.is_subclass_of(handler_type)),
            _ => false,
        },
        _ => false,
    }
}

/// Whether `obj` is a namedtuple instance built from the class `class_id`.
///
/// Instances created by Monty internally (`sys.version_info`, host imports)
/// carry no `class_id`, so they never match a factory class.
fn instance_of_namedtuple_class(obj: &Value, class_id: HeapId, vm: &VM<'_>) -> bool {
    matches!(obj, Value::Ref(obj_id) if matches!(vm.heap.get(*obj_id), HeapData::NamedTuple(nt) if nt.class_id() == Some(class_id)))
}

/// Recursively walks a tuple of classinfo entries.
fn isinstance_check_tuple<'h>(obj: &Value, tuple: &HeapRead<'h, Tuple>, vm: &mut VM<'h>) -> RunResult<bool> {
    let len = tuple.get(vm.heap).as_slice().len();
    let mut guard = vm.recursion_guard()?;
    let vm = &mut *guard;
    for i in 0..len {
        match &tuple.get(vm.heap).as_slice()[i] {
            Value::Builtin(Builtins::Type(t)) => {
                if obj.py_type(vm).is_instance_of(*t) {
                    return Ok(true);
                }
            }
            Value::Builtin(Builtins::ExcType(exc)) => {
                if exception_instance_of(obj, *exc, vm) {
                    return Ok(true);
                }
            }
            Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::Class(_)) => {
                if instance_of_class(obj, *id, vm) {
                    return Ok(true);
                }
            }
            Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::NamedTupleClass(_)) => {
                if instance_of_namedtuple_class(obj, *id, vm) {
                    return Ok(true);
                }
            }
            Value::Ref(nested_id) if let HeapReadOutput::Tuple(nested) = vm.heap.read(*nested_id) => {
                if isinstance_check_tuple(obj, &nested, vm)? {
                    return Ok(true);
                }
            }
            _ => return Err(ExcType::isinstance_arg2_error()),
        }
    }
    Ok(false)
}
