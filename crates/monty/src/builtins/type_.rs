//! Implementation of the type() builtin function.

use super::Builtins;
use crate::{
    args::{ArgValues, KwargsValues},
    bytecode::VM,
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunResult},
    heap::{DropWithContext, HeapData, HeapId},
    intern::StaticStrings,
    types::{Class, Dict, PyTrait},
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
/// itself, so `type(x) is Foo` holds via reference identity; a host class
/// instance returns the `HostClassType` entry it owns (one per host class, so
/// identity holds there too); everything else returns the builtin `Type`
/// marker.
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
    } else if let Value::Ref(id) = &value
        && let HeapData::HostClass(hc) = vm.heap.get(*id)
    {
        let class_id = hc.class_id();
        vm.heap.inc_ref(class_id);
        Value::Ref(class_id)
    } else {
        Value::Builtin(Builtins::Type(value.py_type(vm)))
    }
}

/// The 3-arg `type(name, bases, dict)` form: dynamically creates a class.
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

    let (base_ids, builtin_exc) = resolve_bases(bases, vm)?;

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
        pairs.push((
            Value::InternString(vm.interns.intern_static(StaticStrings::DunderDoc)),
            Value::None,
        ));
    }
    let namespace_dict = Dict::from_pairs(pairs, vm)?;

    // The class takes a reference on each base, released by
    // `Class::py_dec_ref_ids`.
    for base in &base_ids {
        vm.heap.inc_ref(*base);
    }
    let class_id = vm.heap.allocate(HeapData::Class(Box::new(Class::new(
        class_name,
        namespace_dict,
        base_ids,
        builtin_exc,
    ))));
    Ok(Value::Ref(class_id))
}

/// Validates a `type()` bases tuple, answering the base classes it names and
/// the builtin exception the new class descends from, if any.
///
/// Inheritance is single: a second base would need a linearization Monty does
/// not have, so it is refused rather than silently resolved in the order
/// written. A builtin type other than an exception is refused too, for want of
/// anything to inherit: a `Class` holds a namespace, and `str` has none.
fn resolve_bases(bases: &Value, vm: &mut VM<'_>) -> RunResult<(Vec<HeapId>, Option<ExcType>)> {
    let Value::Ref(id) = bases else {
        let got = bases.py_type(vm).cpython_arg_name(vm.heap, vm.interns);
        return Err(ExcType::type_error_bad_arg_pos("type.__new__", 2, "tuple", got));
    };
    let HeapData::Tuple(tuple) = vm.heap.get(*id) else {
        let got = bases.py_type(vm).cpython_arg_name(vm.heap, vm.interns);
        return Err(ExcType::type_error_bad_arg_pos("type.__new__", 2, "tuple", got));
    };
    // Classified while the tuple still borrows the heap; the refusals below
    // need it again, so nothing here holds that borrow.
    let bases: Vec<BaseKind> = tuple
        .as_slice()
        .iter()
        .map(|base| match base {
            Value::Ref(base_id) if matches!(vm.heap.get(*base_id), HeapData::Class(_)) => {
                BaseKind::SandboxClass(*base_id)
            }
            Value::Builtin(Builtins::ExcType(exc)) => BaseKind::BuiltinException(*exc),
            _ => BaseKind::Other,
        })
        .collect();
    if bases.len() > 1 {
        return Err(ExcType::not_implemented(
            "a class with more than one base; Monty resolves a member by walking one chain, \
             so there is no linearization to resolve a second base against",
        )
        .into());
    }
    match bases.into_iter().next() {
        // A sandbox base contributes its own reference, and passes on whatever
        // builtin exception it descends from.
        Some(BaseKind::SandboxClass(id)) => {
            let inherited = match vm.heap.get(id) {
                HeapData::Class(class) => class.builtin_exc(),
                _ => None,
            };
            Ok((vec![id], inherited))
        }
        // A builtin exception is not a heap object, so there is no reference to
        // take: the class records which one it descends from instead.
        Some(BaseKind::BuiltinException(exc)) => Ok((Vec::new(), Some(exc))),
        Some(BaseKind::Other) => Err(ExcType::type_error(
            "a class can only inherit from a class defined in the sandbox or a builtin exception",
        )),
        None => Ok((Vec::new(), None)),
    }
}

/// What a value in a `type()` bases tuple turned out to be.
///
/// Classified in one pass while the tuple borrows the heap, so the refusals can
/// take it back.
enum BaseKind {
    /// A `class` statement's class object: the one base Monty accepts.
    SandboxClass(HeapId),
    /// A builtin exception type: the class descends from it, but there is no
    /// heap object to hold a reference on.
    BuiltinException(ExcType),
    /// Anything else.
    Other,
}
