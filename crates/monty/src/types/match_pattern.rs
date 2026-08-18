//! The runtime side of PEP 634 pattern matching: the questions a pattern asks
//! that ordinary bytecode cannot.
//!
//! Everything else a `match` does — comparisons, subscripts, attribute reads,
//! `isinstance` — the compiler emits from the existing instruction set. What is
//! left here is the shape tests (a sequence pattern deliberately refuses a
//! `str`, which `isinstance(x, Sequence)` would accept), the mapping key
//! lookup, whose "missing" answer has to be a value rather than an exception,
//! and the class pattern, which reads `__match_args__` and reports CPython's
//! errors for a misuse of it.

use smallvec::SmallVec;

use crate::{
    builtins::{Builtins, isinstance::isinstance_check},
    bytecode::{CallResult, VM},
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunError, RunResult},
    heap::{DropWithContext, HeapData, HeapId},
    types::{
        Dict, NativeClass, PyTrait, Type, allocate_tuple,
        instance::{class_member, class_name},
        instance_len,
        native_class::native_isinstance,
    },
    value::Value,
};

/// Whether a sequence pattern may match `subject`.
///
/// PEP 634 excludes `str`, `bytes` and `bytearray` from sequence patterns even
/// though they are sequences, so that `case [x, y]` never silently takes a
/// two-character string apart.
pub(crate) fn is_match_sequence(subject: &Value, vm: &mut VM<'_>) -> RunResult<bool> {
    if matches!(subject.py_type(vm), Type::Str | Type::Bytes) {
        return Ok(false);
    }
    native_isinstance(subject, NativeClass::Sequence, vm)
}

/// The length of a sequence a pattern already accepted, which is what its
/// element count is checked against.
pub(crate) fn match_len(subject: &Value, vm: &mut VM<'_>) -> RunResult<Value> {
    let len = match subject {
        Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::Instance(_)) => instance_len(*id, vm)?,
        other => other.py_len(vm),
    };
    match len {
        Some(len) => Ok(Value::Int(
            i64::try_from(len).map_err(|_| ExcType::overflow_c_ssize_t())?,
        )),
        // Unreachable behind the sequence test, which only accepts what has one.
        None => Err(ExcType::type_error(format!(
            "object of type '{}' has no len()",
            subject.py_type_name(vm)
        ))),
    }
}

/// Whether a mapping pattern may match `subject`.
pub(crate) fn is_match_mapping(subject: &Value, vm: &mut VM<'_>) -> RunResult<bool> {
    native_isinstance(subject, NativeClass::Mapping, vm)
}

/// The values `subject` holds for every key in `keys`, or `None` if it is
/// missing any — the answer a mapping pattern needs as a *value*, since a
/// missing key is a failed match rather than an error.
///
/// Takes ownership of `keys`.
pub(crate) fn match_keys(subject: &Value, keys: Value, vm: &mut VM<'_>) -> RunResult<Value> {
    defer_drop!(keys, vm);
    let wanted = tuple_items(keys, vm);
    defer_drop!(wanted, vm);
    let mut found: SmallVec<[Value; 2]> = SmallVec::with_capacity(wanted.len());
    for key in wanted {
        match subject_get(subject, key, vm) {
            Ok(Some(value)) => found.push(value),
            Ok(None) => {
                found.drop_with(vm);
                return Ok(Value::None);
            }
            Err(e) => {
                found.drop_with(vm);
                return Err(e);
            }
        }
    }
    Ok(allocate_tuple(found, vm.heap))
}

/// A new dict of everything in `subject` that `keys` did not name, which is
/// what `{**rest}` binds. Takes ownership of `keys`.
pub(crate) fn match_rest(subject: &Value, keys: Value, vm: &mut VM<'_>) -> RunResult<Value> {
    defer_drop!(keys, vm);
    let wanted = tuple_items(keys, vm);
    defer_drop!(wanted, vm);
    let Value::Ref(id) = subject else {
        return Err(ExcType::type_error("mapping pattern rest requires a mapping"));
    };
    // Cloned out before anything can re-enter the VM through `py_eq`.
    let pairs = match vm.heap.get(*id) {
        HeapData::Dict(dict) => dict
            .iter()
            .map(|(k, v)| (k.clone_with_heap(vm.heap), v.clone_with_heap(vm.heap)))
            .collect::<Vec<_>>(),
        _ => return Err(ExcType::type_error("mapping pattern rest requires a mapping")),
    };
    let mut kept = Vec::with_capacity(pairs.len());
    for (key, value) in pairs {
        let mut named = false;
        for wanted in wanted {
            if key.py_eq(wanted, vm)? {
                named = true;
                break;
            }
        }
        if named {
            key.drop_with(vm);
            value.drop_with(vm);
        } else {
            kept.push((key, value));
        }
    }
    let dict = Dict::from_pairs(kept, vm)?;
    Ok(Value::Ref(vm.heap.allocate(HeapData::Dict(dict))))
}

/// Matches a class pattern: `isinstance(subject, cls)`, then the attributes the
/// sub-patterns name.
///
/// Returns the tuple of attribute values in sub-pattern order (positional
/// first), or `None` when the subject is not an instance or is missing one of
/// the attributes. Takes ownership of `cls` and `keywords`.
pub(crate) fn match_class(
    subject: &Value,
    cls: Value,
    keywords: Value,
    positional: usize,
    vm: &mut VM<'_>,
) -> RunResult<Value> {
    defer_drop!(cls, vm);
    defer_drop!(keywords, vm);
    if !is_class_value(cls, vm) {
        return Err(ExcType::type_error("called match pattern must be a class"));
    }
    if !isinstance_check(subject, cls, vm)? {
        return Ok(Value::None);
    }
    let names = positional_attr_names(cls, positional, vm)?;
    defer_drop!(names, vm);
    let keyword_names = tuple_items(keywords, vm);
    defer_drop!(keyword_names, vm);
    // A name given both positionally and by keyword would bind the same
    // attribute twice, which CPython reports rather than resolving.
    for keyword in keyword_names {
        for name in names {
            if keyword.py_eq(name, vm)? {
                let attr = attr_display(keyword, vm);
                return Err(ExcType::type_error(format!(
                    "{}() got multiple sub-patterns for attribute '{attr}'",
                    match_class_name(cls, vm)
                )));
            }
        }
    }
    let mut found: SmallVec<[Value; 2]> = SmallVec::with_capacity(names.len() + keyword_names.len());
    for name in names.iter().chain(keyword_names.iter()) {
        // The self-matching marker: `case int(x)` binds the whole subject.
        if matches!(name, Value::Ellipsis) {
            found.push(subject.clone_with_heap(vm.heap));
            continue;
        }
        let Some(attr) = name.as_either_str(vm.heap) else {
            found.drop_with(vm);
            return Err(ExcType::type_error(format!(
                "{}.__match_args__ must be a tuple of strings",
                match_class_name(cls, vm)
            )));
        };
        // A missing attribute is a failed match, not an error, exactly as
        // CPython's `match_class_attr` treats it.
        match subject.py_getattr(&attr, vm) {
            Ok(CallResult::Value(value)) => found.push(value),
            Ok(other) => {
                other.drop_with(vm);
                found.drop_with(vm);
                return Err(ExcType::not_implemented(
                    "a match pattern reading an attribute that suspends (a property doing host work)",
                )
                .into());
            }
            Err(_) => {
                found.drop_with(vm);
                return Ok(Value::None);
            }
        }
    }
    Ok(allocate_tuple(found, vm.heap))
}

/// The attribute names a class pattern's positional sub-patterns bind against.
///
/// A builtin whose single positional sub-pattern matches the whole subject
/// (`case int(x)`) has no attribute at all, and is reported by an empty name
/// list plus the caller's own handling — CPython calls these the "special"
/// classes. Anything else reads `__match_args__`.
fn positional_attr_names(cls: &Value, positional: usize, vm: &mut VM<'_>) -> RunResult<Vec<Value>> {
    if positional == 0 {
        return Ok(Vec::new());
    }
    if let Value::Builtin(Builtins::Type(t)) = cls
        && SELF_MATCHING.contains(t)
    {
        if positional > 1 {
            return Err(ExcType::type_error(format!(
                "{}() accepts 1 positional sub-pattern ({positional} given)",
                match_class_name(cls, vm)
            )));
        }
        // Reported to the caller as "no attribute names"; `match_class` reads
        // the marker back and hands the subject itself to the sub-pattern.
        return Ok(vec![Value::Ellipsis]);
    }
    let args = match cls {
        Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::Class(_)) => class_member(*id, "__match_args__", vm),
        _ => None,
    };
    let Some(args) = args else {
        return Err(ExcType::type_error(format!(
            "{}() accepts 0 positional sub-patterns ({positional} given)",
            match_class_name(cls, vm)
        )));
    };
    defer_drop!(args, vm);
    let Value::Ref(id) = args else {
        return Err(match_args_type_error(cls, args, vm));
    };
    if !matches!(vm.heap.get(*id), HeapData::Tuple(_)) {
        return Err(match_args_type_error(cls, args, vm));
    }
    let names = tuple_items(args, vm);
    if names.len() < positional {
        let available = names.len();
        names.drop_with(vm);
        return Err(ExcType::type_error(format!(
            "{}() accepts {available} positional sub-patterns ({positional} given)",
            match_class_name(cls, vm)
        )));
    }
    let mut names = names;
    let extra = names.split_off(positional);
    extra.drop_with(vm);
    Ok(names)
}

/// The `TypeError` for a `__match_args__` that is not a tuple.
fn match_args_type_error(cls: &Value, args: &Value, vm: &mut VM<'_>) -> RunError {
    let got = args.py_type_name(vm).into_owned();
    ExcType::type_error(format!(
        "{}.__match_args__ must be a tuple (got {got})",
        match_class_name(cls, vm)
    ))
}

/// The class name a class-pattern error message names.
fn match_class_name(cls: &Value, vm: &mut VM<'_>) -> String {
    match cls {
        Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::Class(_)) => {
            class_name(*id, vm.heap, vm.interns).into_owned()
        }
        Value::Builtin(Builtins::Type(t)) => t.name(vm.heap, vm.interns).into_owned(),
        Value::Builtin(Builtins::ExcType(e)) => <&str>::from(*e).to_owned(),
        other => other.py_type_name(vm).into_owned(),
    }
}

/// Renders an attribute name for the duplicate-sub-pattern message.
fn attr_display(name: &Value, vm: &mut VM<'_>) -> String {
    name.as_either_str(vm.heap)
        .map_or_else(|| "?".to_owned(), |s| s.as_str(vm.interns).to_owned())
}

/// The builtin classes whose single positional sub-pattern matches the whole
/// subject rather than an attribute, as PEP 634 lists them. `bytearray` and
/// `complex` would belong here, and Monty has neither.
const SELF_MATCHING: &[Type] = &[
    Type::Bool,
    Type::Bytes,
    Type::Dict,
    Type::Float,
    Type::FrozenSet,
    Type::Int,
    Type::List,
    Type::Set,
    Type::Str,
    Type::Tuple,
];

/// Whether `value` is something a class pattern may test against.
fn is_class_value(value: &Value, vm: &VM<'_>) -> bool {
    match value {
        Value::Builtin(Builtins::Type(_) | Builtins::ExcType(_)) => true,
        Value::Ref(id) => matches!(vm.heap.get(*id), HeapData::Class(_) | HeapData::NamedTupleClass(_)),
        _ => false,
    }
}

/// Clones the items out of a tuple value; the compiler only ever passes one.
fn tuple_items(value: &Value, vm: &mut VM<'_>) -> Vec<Value> {
    match value {
        Value::Ref(id) => match vm.heap.get(*id) {
            HeapData::Tuple(t) => t.as_slice().iter().map(|v| v.clone_with_heap(vm.heap)).collect(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// Reads one key out of the subject of a mapping pattern, answering `None` for
/// a key it does not hold.
fn subject_get(subject: &Value, key: &Value, vm: &mut VM<'_>) -> RunResult<Option<Value>> {
    let Value::Ref(id) = subject else {
        return Ok(None);
    };
    let id: HeapId = *id;
    match vm.heap.get(id) {
        HeapData::Dict(_) => {
            let HeapData::Dict(dict) = vm.heap.get(id) else {
                unreachable!("matched a dict");
            };
            // Cloned out of the read before `py_eq` inside the lookup can
            // re-enter the VM.
            let pairs = dict
                .iter()
                .map(|(k, v)| (k.clone_with_heap(vm.heap), v.clone_with_heap(vm.heap)))
                .collect::<Vec<_>>();
            defer_drop!(pairs, vm);
            for (candidate, value) in pairs {
                if candidate.py_eq(key, vm)? {
                    return Ok(Some(value.clone_with_heap(vm.heap)));
                }
            }
            Ok(None)
        }
        // Anything else that answered the mapping test is a sandbox class with
        // a `__getitem__`; a missing key raises there, which is a failed match.
        _ => match subject.py_getitem(key, vm) {
            Ok(value) => Ok(Some(value)),
            Err(_) => Ok(None),
        },
    }
}
