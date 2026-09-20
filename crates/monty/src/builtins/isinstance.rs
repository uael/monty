//! Implementation of the isinstance() builtin function.

use super::{Builtins, BuiltinsFunctions};
use crate::{
    args::ArgValues,
    bytecode::VM,
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunResult},
    heap::{HeapData, HeapId, HeapRead, HeapReadOutput},
    intern::StaticStrings,
    types::{
        PyTrait, Tuple, Type,
        instance::{class_chain, instance_builtin_exc},
    },
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
/// - User-defined classes: `isinstance(obj, Foo)`, walking the instance's class
///   chain
/// - Host classes: `isinstance(obj, Point)` for a `HostClassType` (exact class
///   id; the host sends no bases)
/// - The abstract base classes of `collections.abc`: `isinstance(x, Iterable)`,
///   answered from what the value is; see [`abc_instance`]
/// - Tuples (possibly nested) of the above
pub(crate) fn isinstance_check(obj: &Value, classinfo: &Value, vm: &mut VM<'_>) -> RunResult<bool> {
    match classinfo {
        Value::Builtin(Builtins::Type(t)) => Ok(obj.py_type(vm).is_instance_of(*t)),
        // A sandbox exception class's instance matches through the builtin
        // ancestor its class resolved at creation.
        Value::Builtin(Builtins::ExcType(handler_type)) => Ok(match instance_builtin_exc(obj, vm) {
            Some(ancestor) => ancestor.is_subclass_of(*handler_type),
            None => matches!(obj.py_type(vm), Type::Exception(exc_type) if exc_type.is_subclass_of(*handler_type)),
        }),
        // A user-defined class: true for an instance of it or of a subclass.
        Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::Class(_)) => Ok(instance_of_class(obj, *id, vm)),
        // A `collections.namedtuple` class, matched by the instance's `class_id`.
        Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::NamedTupleClass(_)) => {
            Ok(instance_of_namedtuple_class(obj, *id, vm))
        }
        Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::HostClassType(_)) => {
            Ok(instance_of_host_class(obj, *id, vm))
        }
        Value::Ref(id) if let HeapReadOutput::Tuple(tuple) = vm.heap.read(*id) => {
            isinstance_check_tuple(obj, &tuple, vm)
        }
        Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::GenericAlias(_)) => {
            Err(ExcType::isinstance_parameterized_generic())
        }
        // `int | None`: true when any member matches, tested in order.
        Value::Ref(id) if let HeapData::Union(union) = vm.heap.get(*id) => {
            let args = union.args(vm.heap);
            defer_drop!(args, vm);
            let Some(HeapReadOutput::Tuple(members)) = args.read_heap(vm) else {
                unreachable!("Union::args is always a tuple")
            };
            isinstance_check_tuple(obj, &members, vm)
        }
        // `type` is a builtin function here rather than the class object, but
        // it is still the class every class is an instance of, which is what
        // `isinstance(x, type)` asks; see `limitations/builtins.md`.
        Value::Builtin(Builtins::Function(BuiltinsFunctions::Type)) => Ok(obj.py_type(vm).is_instance_of(Type::Type)),
        // An abstract base class of `collections.abc`, which is a marker here
        // rather than a class; see [`abc_instance`].
        Value::Marker(marker) => abc_instance(obj, marker.0, vm),
        _ => Err(ExcType::isinstance_arg2_error()),
    }
}

/// Whether `obj` is an instance of one of the abstract base classes
/// `collections.abc` and `typing` export.
///
/// CPython answers these through `__subclasshook__` and a registry, neither of
/// which Monty has, so each one is answered from what the value is. The seven
/// below are the ones a real type stands behind; every other marker is no class
/// to test against and raises as it did before. See `limitations/collections.md`.
fn abc_instance(obj: &Value, marker: StaticStrings, vm: &mut VM<'_>) -> RunResult<bool> {
    let held = obj.py_type(vm);
    Ok(match marker {
        // The same answer `callable()` gives, which leaves out an instance of
        // a class with a `__call__`: Monty dispatches none.
        StaticStrings::Callable => obj.is_callable(vm.heap),
        StaticStrings::CoroutineType => held == Type::Coroutine,
        StaticStrings::Generator => held == Type::Generator,
        StaticStrings::Iterable => obj.py_is_iterable(vm),
        StaticStrings::IteratorType => obj.py_is_iterator(vm),
        StaticStrings::Mapping => matches!(held, Type::Dict | Type::DefaultDict | Type::Counter),
        StaticStrings::Sequence => matches!(
            held,
            Type::Str | Type::Bytes | Type::List | Type::Tuple | Type::NamedTuple | Type::Range | Type::Deque
        ),
        _ => return Err(ExcType::isinstance_arg2_error()),
    })
}

/// Whether `obj` is a host instance whose class entry is `class_id` (exact
/// class only: the host never sends bases, so subclasses are unknown).
fn instance_of_host_class(obj: &Value, class_id: HeapId, vm: &VM<'_>) -> bool {
    matches!(obj, Value::Ref(obj_id) if matches!(vm.heap.get(*obj_id), HeapData::HostClass(hc) if hc.class_id() == class_id))
}

/// Whether `obj` is an instance of `class_id` or of one of its subclasses.
///
/// An instance is usually an [`Instance`](crate::types::Instance); an instance
/// of a class that inherits `str` is a string carrying its class instead, and
/// answers from the same chain.
fn instance_of_class(obj: &Value, class_id: HeapId, vm: &VM<'_>) -> bool {
    let Value::Ref(obj_id) = obj else { return false };
    let own_class = match vm.heap.get(*obj_id) {
        HeapData::Instance(inst) => inst.class(),
        HeapData::Str(value) => match value.class() {
            Some(own_class) => own_class,
            None => return false,
        },
        _ => return false,
    };
    class_chain(own_class, vm).contains(&class_id)
}

/// Whether `obj` is a namedtuple instance built from the class `class_id`.
///
/// Instances created by Monty internally (`sys.version_info`, host imports)
/// carry no `class_id`, so they never match a factory class.
fn instance_of_namedtuple_class(obj: &Value, class_id: HeapId, vm: &VM<'_>) -> bool {
    matches!(obj, Value::Ref(obj_id) if matches!(vm.heap.get(*obj_id), HeapData::NamedTuple(nt) if nt.class_id() == Some(class_id)))
}

/// Recursively walks a tuple of classinfo entries.
///
/// Each entry is answered by [`isinstance_check`], so the two can never drift:
/// an entry is read out of the tuple and owned for the length of its check,
/// because the check needs the VM and the tuple is in the heap it holds.
fn isinstance_check_tuple<'h>(obj: &Value, tuple: &HeapRead<'h, Tuple>, vm: &mut VM<'h>) -> RunResult<bool> {
    let len = tuple.get(vm.heap).as_slice().len();
    let mut guard = vm.recursion_guard()?;
    let vm = &mut *guard;
    for i in 0..len {
        let entry = tuple.get(vm.heap).as_slice()[i].clone_with_heap(vm.heap);
        defer_drop!(entry, vm);
        if isinstance_check(obj, entry, vm)? {
            return Ok(true);
        }
    }
    Ok(false)
}
