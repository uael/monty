//! Implementation of the type() builtin function.

use super::Builtins;
use crate::{
    args::{ArgValues, KwargsValues},
    bytecode::VM,
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunResult},
    heap::{DropGuard, DropWithContext, HeapData},
    intern::StaticStrings,
    types::{Class, Dict, PyTrait, Type, protocol::mark_protocol},
    value::Value,
};

/// Implementation of the type() builtin function.
///
/// The 1-arg form returns the type of an object; the 3-arg form
/// `type(name, bases, dict)` dynamically creates a new class, mirroring
/// CPython (except that `bases` must be empty — Monty classes cannot
/// inherit). Any other positional count is a `TypeError`.
///
/// This hand-rolls `args.into_parts()` rather than using `#[derive(FromArgs)]`
/// because the "exactly 1 *or* 3 positionals, same name" overload isn't
/// expressible by any of the binder families — CPython special-cases `type`'s
/// argument parsing in `type_new`/`type_init` for the same reason.
pub fn builtin_type(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let (mut pos, kwargs) = args.into_parts();
    match pos.len() {
        1 => {
            let value = pos.next().expect("length checked");
            if kwargs.is_empty() {
                Ok(type_of(vm, value))
            } else {
                value.drop_with(vm);
                kwargs.drop_with(vm);
                Err(ExcType::type_error_no_kwargs("type"))
            }
        }
        3 => {
            let name = pos.next().expect("length checked");
            let bases = pos.next().expect("length checked");
            let namespace = pos.next().expect("length checked");
            create_class(vm, name, bases, namespace, kwargs)
        }
        _ => {
            pos.drop_with(vm);
            kwargs.drop_with(vm);
            Err(ExcType::type_error("type() takes 1 or 3 arguments"))
        }
    }
}

/// The 1-arg `type(obj)` form.
///
/// For an instance of a user-defined class the type *is* the class object
/// itself, so `type(x) is Foo` holds via reference identity; for everything
/// else it returns the builtin `Type` marker.
///
/// An exception is the one builtin whose class also has a *name* in scope, and
/// that name evaluates to `Builtins::ExcType`. Returning the `Type` marker for
/// it would make `type(exc) is ValueError` false against the very object the
/// name produces, so exceptions return the same `ExcType` form.
fn type_of(vm: &mut VM<'_>, value: Value) -> Value {
    defer_drop!(value, vm);
    if let Value::Ref(id) = &value
        && let HeapData::Instance(inst) = vm.heap.get(*id)
    {
        let class_id = inst.class();
        vm.heap.inc_ref(class_id);
        Value::Ref(class_id)
    } else if let Value::Ref(id) = &value
        && let HeapData::NamedTuple(nt) = vm.heap.get(*id)
        && let Some(class_id) = nt.class_id()
    {
        // A factory-made namedtuple's type is its class object, so
        // `type(p) is Point` holds by identity (self-describing internal named
        // tuples like `sys.version_info` have no class and fall through).
        vm.heap.inc_ref(class_id);
        Value::Ref(class_id)
    } else if let Type::Exception(exc_type) = value.py_type(vm) {
        Value::Builtin(Builtins::ExcType(exc_type))
    } else {
        Value::Builtin(Builtins::Type(value.py_type(vm)))
    }
}

/// The 3-arg `type(name, bases, dict)` form: dynamically creates a class.
///
/// Also the runtime behind every compiled `class` statement, which the compiler
/// lowers to a call of this form, so this is the single place a class object is
/// built and the single place inheritance is validated.
///
/// Follows CPython's validation order (name, then bases, then dict, then
/// keyword rejection) and message wording (`type.__new__() argument N must
/// be ...`), except that non-string namespace keys raise a `TypeError`
/// where CPython merely warns. The namespace dict is *copied* into the
/// class — later mutation of the source dict must not affect the class —
/// and a `__doc__ = None` entry is synthesized when the dict omits it,
/// matching CPython's `type` descriptor default (compiled `class` bodies
/// get their `__doc__` from the parser instead).
fn create_class(
    vm: &mut VM<'_>,
    name: Value,
    bases: Value,
    namespace: Value,
    kwargs: KwargsValues,
) -> RunResult<Value> {
    defer_drop!(name, vm);
    defer_drop!(bases, vm);
    defer_drop!(namespace, vm);
    defer_drop!(kwargs, vm);

    let Some(class_name) = name.as_either_str(vm.heap) else {
        let got = name.py_type(vm).cpython_arg_name(vm.heap, vm.interns);
        return Err(ExcType::type_error_bad_arg_pos("type.__new__", 1, "str", got));
    };

    let base_slots = match bases {
        Value::Ref(id) if let HeapData::Tuple(t) = vm.heap.get(*id) => {
            // Cloned out so `resolve_bases` can take `&mut VM` without the
            // tuple's borrow of the heap still being live.
            t.as_slice()
                .iter()
                .map(|v| v.clone_with_heap(vm.heap))
                .collect::<Vec<_>>()
        }
        _ => {
            let got = bases.py_type(vm).cpython_arg_name(vm.heap, vm.interns);
            return Err(ExcType::type_error_bad_arg_pos("type.__new__", 2, "tuple", got));
        }
    };
    defer_drop!(base_slots, vm);
    let (base_values, exc_base) = resolve_bases(base_slots, vm)?;

    let Value::Ref(ns_id) = namespace else {
        let got = namespace.py_type(vm).cpython_arg_name(vm.heap, vm.interns);
        return Err(ExcType::type_error_bad_arg_pos("type.__new__", 3, "dict", got));
    };
    let HeapData::Dict(source) = vm.heap.get(*ns_id) else {
        let got = namespace.py_type(vm).cpython_arg_name(vm.heap, vm.interns);
        return Err(ExcType::type_error_bad_arg_pos("type.__new__", 3, "dict", got));
    };

    if !kwargs.is_empty() {
        // CPython forwards extra keywords to `__init_subclass__`, which
        // `object` rejects with this message — synthesize the equivalent.
        let name_str = class_name.as_str(vm.interns);
        return Err(ExcType::type_error_no_kwargs(&format!("{name_str}.__init_subclass__")));
    }

    // Monty divergence: CPython only emits a `RuntimeWarning` for non-string
    // namespace keys; Monty has no warnings machinery, so silently accepting
    // them would hide the mistake — raise instead. Validated before cloning
    // any pairs so the error path has nothing to clean up.
    if let Some((bad_key, _)) = source.iter().find(|(k, _)| !k.is_str(vm.heap)) {
        let name_str = class_name.as_str(vm.interns);
        let key_type = bad_key.py_type_heap(vm.heap).name(vm.heap, vm.interns);
        return Err(ExcType::type_error(format!(
            "non-string key ({key_type}) in the namespace of class '{name_str}'"
        )));
    }

    // Copy the namespace (CPython semantics: the class owns an independent
    // dict). `clone_with_heap` takes `&Heap`, so the pairs can be cloned
    // while `source` still borrows the heap immutably.
    let mut pairs: Vec<(Value, Value)> = source
        .iter()
        .map(|(k, v)| (k.clone_with_heap(vm.heap), v.clone_with_heap(vm.heap)))
        .collect();
    if source.get_by_str("__doc__", vm.heap, vm.interns).is_none() {
        pairs.push((Value::InternString(StaticStrings::DunderDoc.into()), Value::None));
    }
    if source.get_by_str("_is_protocol", vm.heap, vm.interns).is_none() {
        mark_protocol(&mut pairs, &base_values, vm);
    }
    let namespace_dict = Dict::from_pairs(pairs, vm)?;

    let class_id = vm.heap.allocate(HeapData::Class(Box::new(Class::new(
        class_name,
        namespace_dict,
        base_values,
        exc_base,
        vm.scope,
    ))));
    Ok(Value::Ref(class_id))
}

/// Validates a `bases` tuple and takes an owned reference to each entry,
/// returning them alongside the nearest builtin exception ancestor.
///
/// Each base is first put through the `__mro_entries__` protocol, which means
/// a subscripted class (`Spawned[T]`) stands for the class it subscripted.
///
/// What survives is of two kinds. A **concrete** base — a sandbox class or a
/// builtin exception type — joins the inheritance chain, and Monty implements
/// single inheritance, so a second one is rejected rather than linearized:
/// there is no MRO algorithm behind this, only a chain walk (see
/// `limitations/classes.md`). A **natively provided** base
/// ([`Type::Native`]: `typing.Protocol`, the `collections.abc` classes) adds
/// no link to that chain but is kept in `bases`, which is what later makes
/// `isinstance(Foo(), Iterator)` true and lets the base contribute default
/// members. Every other base — `object` and the remaining builtin types — is
/// rejected, since their instances have no `__dict__` to inherit into.
fn resolve_bases(bases: &[Value], vm: &mut VM<'_>) -> RunResult<(Vec<Value>, Option<ExcType>)> {
    let mut guard = DropGuard::new(Vec::<Value>::with_capacity(bases.len()), vm);
    let mut exc_base = None;
    let mut concrete = 0usize;
    for declared in bases {
        let base = mro_entry(declared, guard.ctx());
        let (kept, vm) = guard.as_parts_mut();
        match &base {
            Value::Builtin(Builtins::Type(Type::Native(_))) => {}
            Value::Builtin(Builtins::ExcType(exc_type)) => {
                concrete += 1;
                exc_base = Some(*exc_type);
            }
            Value::Ref(id) if let HeapData::Class(class) = vm.heap.get(*id) => {
                concrete += 1;
                exc_base = class.exc_base();
            }
            other => {
                // A builtin type names itself (`int`), not its own type (`type`),
                // which is what the reader wrote in the base list.
                let got = match other {
                    Value::Builtin(Builtins::Type(t)) => t.name(vm.heap, vm.interns).into_owned(),
                    _ => other.py_type(vm).cpython_arg_name(vm.heap, vm.interns).into_owned(),
                };
                base.drop_with(vm);
                return Err(ExcType::not_implemented(format!(
                    "inheriting from '{got}' is not supported; a base must be a class defined in the sandbox or a builtin exception"
                ))
                .into());
            }
        }
        kept.push(base);
    }
    let (kept, vm) = guard.into_parts();
    if concrete > 1 {
        kept.drop_with(vm);
        return Err(ExcType::not_implemented("multiple inheritance is not supported").into());
    }
    Ok((kept, exc_base))
}

/// The `__mro_entries__` of one base, as an owned reference.
///
/// Only `types.GenericAlias` defines the protocol here, and it resolves to its
/// origin — so `class Held(Spawned[T])` inherits from `Spawned`. Every other
/// base stands for itself.
fn mro_entry(base: &Value, vm: &mut VM<'_>) -> Value {
    match base {
        Value::Ref(id) if let HeapData::GenericAlias(alias) = vm.heap.get(*id) => {
            alias.origin().clone_with_heap(vm.heap)
        }
        other => other.clone_with_heap(vm.heap),
    }
}
