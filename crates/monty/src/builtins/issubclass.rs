//! The `issubclass()` builtin.

use crate::{
    args::ArgValues,
    builtins::Builtins,
    bytecode::VM,
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunResult},
    heap::{HeapData, HeapId},
    types::{Type, instance::class_chain},
    value::Value,
};

/// `issubclass(cls, classinfo)`: whether `cls` is `classinfo` or inherits from it.
///
/// Both arguments must be classes, unlike `isinstance()`, whose first argument
/// is any object: CPython raises `TypeError: issubclass() arg 1 must be a class`
/// for anything else, and so does this.
pub(crate) fn builtin_issubclass(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let (cls, classinfo) = args.get_two_args("issubclass", vm.heap)?;
    defer_drop!(cls, vm);
    defer_drop!(classinfo, vm);
    let Some(chain) = subject_chain(cls, vm) else {
        return Err(ExcType::type_error("issubclass() arg 1 must be a class"));
    };
    issubclass_check(cls, &chain, classinfo, vm).map(Value::Bool)
}

/// The class chain of `issubclass`'s first argument, or `None` when it is not a
/// class at all.
///
/// A builtin type has no chain to walk, so it answers an empty one and matches
/// only itself.
fn subject_chain(cls: &Value, vm: &VM<'_>) -> Option<Vec<HeapId>> {
    match cls {
        Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::Class(_)) => Some(class_chain(*id, vm).into_vec()),
        Value::Builtin(Builtins::Type(_) | Builtins::ExcType(_)) => Some(Vec::new()),
        _ => None,
    }
}

/// Tests one `classinfo` entry, or every entry of a tuple of them.
fn issubclass_check(cls: &Value, chain: &[HeapId], classinfo: &Value, vm: &mut VM<'_>) -> RunResult<bool> {
    match classinfo {
        Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::Class(_)) => Ok(chain.contains(id)),
        Value::Builtin(Builtins::ExcType(handler)) => {
            Ok(matches!(cls, Value::Builtin(Builtins::ExcType(exc)) if exc.is_subclass_of(*handler)))
        }
        Value::Builtin(Builtins::Type(wanted)) => {
            Ok(matches!(cls, Value::Builtin(Builtins::Type(t)) if is_type_subclass(*t, *wanted)))
        }
        Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::Tuple(_)) => issubclass_tuple(cls, chain, *id, vm),
        _ => Err(ExcType::type_error(
            "issubclass() arg 2 must be a class, or tuple of classes",
        )),
    }
}

/// Whether one builtin type is the other or a subclass of it.
///
/// `bool` is the only builtin subclass relationship Monty has, and it is the
/// one CPython also has.
fn is_type_subclass(cls: Type, wanted: Type) -> bool {
    cls == wanted || (cls == Type::Bool && wanted == Type::Int)
}

/// Walks a flat tuple of `classinfo` entries. CPython does not descend into a
/// nested tuple here, so neither does this.
///
/// The entries are cloned out first: each test re-reads the heap, which cannot
/// happen while the tuple still borrows it.
fn issubclass_tuple(cls: &Value, chain: &[HeapId], tuple_id: HeapId, vm: &mut VM<'_>) -> RunResult<bool> {
    let HeapData::Tuple(tuple) = vm.heap.get(tuple_id) else {
        unreachable!("caller matched a tuple");
    };
    let entries: Vec<Value> = tuple
        .as_slice()
        .iter()
        .map(|entry| entry.clone_with_heap(vm.heap))
        .collect();
    defer_drop!(entries, vm);
    for entry in entries {
        if issubclass_check(cls, chain, entry, vm)? {
            return Ok(true);
        }
    }
    Ok(false)
}
