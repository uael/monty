//! `typing.Protocol`: the flags a protocol class carries and the structural
//! `isinstance` a `@runtime_checkable` one answers.
//!
//! The flags live in the class namespace under the names CPython uses
//! (`_is_protocol`, `_is_runtime_protocol`, `__protocol_attrs__`), so nothing
//! new has to be stored on a class object and sandbox code sees what it would
//! see there.

use crate::{
    builtins::Builtins,
    bytecode::{CallResult, VM},
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunError, RunResult},
    heap::{DropGuard, DropWithContext, HeapData, HeapId, HeapReadOutput},
    intern::StaticStrings,
    types::{
        FrozenSet, LazyHeapSet, NativeClass, PyTrait, Set, Type,
        class::{MAX_MRO_DEPTH, class_base_id},
        instance::{class_has_member, class_member},
        str::allocate_string,
    },
    value::Value,
};

/// Namespace names that never count as protocol members.
///
/// CPython's `EXCLUDED_ATTRIBUTES`, restricted to the ones a Monty class body
/// can produce: `__doc__` is synthesized for every class, and `__init__`,
/// `__module__` and `__annotations__` are machinery rather than interface.
const EXCLUDED: &[&str] = &[
    "__doc__",
    "__init__",
    "__module__",
    "__annotations__",
    "__protocol_attrs__",
    "_is_protocol",
    "_is_runtime_protocol",
];

/// Whether the class's own namespace declares it a protocol, i.e. whether
/// `class C(Protocol)` was written rather than `class C(SomeProtocol)`.
///
/// Own namespace, not the chain: CPython writes `_is_protocol = False` onto a
/// concrete subclass, which is what makes that subclass instantiable.
pub(crate) fn is_protocol_class(class_id: HeapId, vm: &VM<'_>) -> bool {
    match vm.heap.get(class_id) {
        HeapData::Class(class) => matches!(
            class.namespace().get_by_str("_is_protocol", vm.heap, vm.interns),
            Some(Value::Bool(true))
        ),
        _ => false,
    }
}

/// Writes `_is_protocol` into a class namespace under construction, mirroring
/// CPython's `Protocol.__init_subclass__`.
///
/// True when `typing.Protocol` is literally among the bases, false when a base
/// merely inherits from one — that second write is what makes a concrete
/// subclass of a protocol instantiable. A class with no protocol anywhere in
/// its bases gains no attribute at all.
pub(crate) fn mark_protocol(pairs: &mut Vec<(Value, Value)>, bases: &[Value], vm: &VM<'_>) {
    let direct = bases.iter().any(|base| {
        matches!(
            base,
            Value::Builtin(Builtins::Type(Type::Native(NativeClass::Protocol)))
        )
    });
    let inherited = bases
        .iter()
        .any(|base| matches!(base, Value::Ref(id) if class_member_exists(*id, "_is_protocol", vm)));
    if direct || inherited {
        pairs.push((
            Value::InternString(StaticStrings::IsProtocol.into()),
            Value::Bool(direct),
        ));
    }
}

/// Whether `class_id` or one of its bases binds `name`; `false` for a non-class.
fn class_member_exists(class_id: HeapId, name: &str, vm: &VM<'_>) -> bool {
    matches!(vm.heap.get(class_id), HeapData::Class(_)) && class_has_member(class_id, name, vm)
}

/// `typing.runtime_checkable(cls)`: records the member names an `isinstance`
/// check will look for, and returns the class unchanged.
pub(crate) fn runtime_checkable(cls: Value, vm: &mut VM<'_>) -> RunResult<Value> {
    let mut guard = DropGuard::new(cls, vm);
    let (cls, vm) = guard.as_parts();
    // CPython reaches its own `issubclass(cls, Generic)` first, so a non-class
    // argument fails with that call's complaint rather than this one's.
    let Value::Ref(class_id) = cls else {
        return Err(ExcType::type_error("issubclass() arg 1 must be a class"));
    };
    let class_id = *class_id;
    if !matches!(vm.heap.get(class_id), HeapData::Class(_)) {
        return Err(ExcType::type_error("issubclass() arg 1 must be a class"));
    }
    if !is_protocol_class(class_id, vm) {
        return Err(runtime_checkable_error(cls, vm)?);
    }
    let members = protocol_attrs(class_id, vm)?;
    let HeapReadOutput::Class(mut class) = vm.heap.read(class_id) else {
        unreachable!("is_protocol_class matched a class");
    };
    let replaced = class.set_attr(
        Value::InternString(StaticStrings::IsRuntimeProtocol.into()),
        Value::Bool(true),
        vm,
    )?;
    replaced.drop_with(vm);
    let replaced = class.set_attr(
        Value::InternString(StaticStrings::DunderProtocolAttrs.into()),
        members,
        vm,
    )?;
    replaced.drop_with(vm);
    Ok(guard.into_inner())
}

/// The `TypeError` `runtime_checkable` raises for a non-protocol argument,
/// which names the argument by its `repr` exactly as CPython does.
fn runtime_checkable_error(got: &Value, vm: &mut VM<'_>) -> RunResult<RunError> {
    let mut rendered = String::new();
    let mut heap_ids = LazyHeapSet::default();
    got.py_repr_fmt(&mut rendered, vm, &mut heap_ids)?;
    Ok(ExcType::type_error(format!(
        "@runtime_checkable can be only applied to protocol classes, got {rendered}"
    )))
}

/// The member names a runtime-checkable protocol tests for: everything its
/// namespace and its bases' namespaces bind, plus everything they annotate,
/// minus the machinery names.
///
/// The annotations matter as much as the bindings, and are the only source for
/// the common shape: `class HasContent(Protocol): content: str` binds nothing
/// at all, so reading the namespace alone yields the empty set, and an empty
/// set is satisfied by every object. CPython's `_get_protocol_attrs` reads both
/// for the same reason.
fn protocol_attrs(class_id: HeapId, vm: &mut VM<'_>) -> RunResult<Value> {
    let mut names: Vec<String> = Vec::new();
    let mut current = Some(class_id);
    for _ in 0..MAX_MRO_DEPTH {
        let Some(id) = current else { break };
        if let HeapData::Class(class) = vm.heap.get(id) {
            let mut found: Vec<String> = class
                .namespace()
                .into_iter()
                .filter_map(|(key, _)| key.as_either_str(vm.heap))
                .map(|name| name.as_str(vm.interns).to_owned())
                .collect();
            found.extend(annotated_names(id, vm));
            for name in found {
                if !EXCLUDED.contains(&name.as_str()) && !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        current = class_base_id(id, vm);
    }
    // A `frozenset`, as CPython's is: the order the chain walk found them in
    // carries no meaning, and membership is the only question asked of it.
    let mut storage = Set::new();
    for name in names {
        let name = allocate_string(name, vm.heap);
        if let Err(e) = storage.add(name, vm) {
            Value::Ref(vm.heap.allocate(HeapData::Set(storage))).drop_with(vm);
            return Err(e);
        }
    }
    Ok(Value::Ref(
        vm.heap.allocate(HeapData::FrozenSet(FrozenSet::from_set(storage))),
    ))
}

/// The names `class_id`'s own `__annotations__` declares, which the parser
/// writes for every class body that annotates anything.
fn annotated_names(class_id: HeapId, vm: &VM<'_>) -> Vec<String> {
    let HeapData::Class(class) = vm.heap.get(class_id) else {
        return Vec::new();
    };
    let Some(Value::Ref(id)) = class.namespace().get_by_str("__annotations__", vm.heap, vm.interns) else {
        return Vec::new();
    };
    let HeapData::Dict(annotations) = vm.heap.get(*id) else {
        return Vec::new();
    };
    annotations
        .into_iter()
        .filter_map(|(key, _)| key.as_either_str(vm.heap))
        .map(|name| name.as_str(vm.interns).to_owned())
        .collect()
}

/// How a class answers an `isinstance`/`issubclass` question.
pub(crate) enum ProtocolCheck {
    /// Not a protocol: the ordinary base-chain walk is the whole answer.
    Ordinary,
    /// A `@runtime_checkable` protocol: the base-chain walk, or failing that a
    /// structural test against the recorded member names.
    Structural,
    /// A protocol that was never made runtime-checkable, which CPython refuses
    /// to answer at all — even for a class that really does derive from it.
    Refused,
}

/// Which of the three answers `class_id` gives.
pub(crate) fn protocol_check(class_id: HeapId, vm: &VM<'_>) -> ProtocolCheck {
    if !is_protocol_class(class_id, vm) {
        ProtocolCheck::Ordinary
    } else if class_has_member(class_id, "_is_runtime_protocol", vm) {
        ProtocolCheck::Structural
    } else {
        ProtocolCheck::Refused
    }
}

/// The `TypeError` a non-runtime protocol raises for either check.
pub(crate) fn protocol_check_refused() -> RunError {
    ExcType::type_error("Instance and class checks can only be used with @runtime_checkable protocols")
}

/// The member names `class_id` records for a structural check, or `None` when
/// it records none.
fn recorded_attrs(class_id: HeapId, vm: &mut VM<'_>) -> Option<Vec<Value>> {
    let members = class_member(class_id, "__protocol_attrs__", vm)?;
    defer_drop!(members, vm);
    match members {
        Value::Ref(id) => match vm.heap.get(*id) {
            HeapData::FrozenSet(set) => Some(set.storage().iter().map(|v| v.clone_with_heap(vm.heap)).collect()),
            _ => None,
        },
        _ => None,
    }
}

/// Whether `obj` structurally satisfies the runtime-checkable protocol
/// `class_id`: every recorded member name must resolve on the object.
pub(crate) fn protocol_instance_of(obj: &Value, class_id: HeapId, vm: &mut VM<'_>) -> RunResult<bool> {
    let Some(names) = recorded_attrs(class_id, vm) else {
        return Ok(false);
    };
    defer_drop!(names, vm);
    for name in names {
        let Some(attr) = name.as_either_str(vm.heap) else {
            return Ok(false);
        };
        // Read through `py_getattr`, as CPython's `__instancecheck__` does, so
        // an attribute an instance set at runtime counts. The value is only
        // wanted for its existence.
        match obj.py_getattr(&attr, vm) {
            Ok(CallResult::Value(value)) => value.drop_with(vm),
            // A property getter or method binding is a value the structural
            // check has no use for and cannot drive to completion here; its
            // presence is the answer.
            Ok(other) => other.drop_with(vm),
            Err(RunError::Exc(e)) if e.exc.exc_type() == ExcType::AttributeError => return Ok(false),
            Err(e) => return Err(e),
        }
    }
    Ok(true)
}

/// Whether the class `subject` structurally satisfies the runtime-checkable
/// protocol `class_id`: every recorded member name must be bound somewhere in
/// the subject's own chain.
pub(crate) fn protocol_subclass_of(subject: HeapId, class_id: HeapId, vm: &mut VM<'_>) -> bool {
    let Some(names) = recorded_attrs(class_id, vm) else {
        return false;
    };
    let answer = names.iter().all(|name| {
        name.as_either_str(vm.heap)
            .is_some_and(|attr| class_has_member(subject, attr.as_str(vm.interns), vm))
    });
    names.drop_with(vm);
    answer
}
