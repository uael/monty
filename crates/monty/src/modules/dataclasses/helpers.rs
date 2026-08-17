//! The `dataclasses` module functions that read a decorated class back:
//! `fields`, `asdict`, `astuple`, `replace` and `is_dataclass`.

use smallvec::SmallVec;

use super::{class_fields_dict_id, field_specs, is_dataclass_class};
use crate::{
    args::{ArgValues, FromArgs, KwargsValues},
    bytecode::VM,
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunResult},
    heap::{DropGuard, HeapData, HeapId},
    types::{
        Dict, List,
        instance::{class_name, instance_attr},
        str::allocate_string,
        tuple::allocate_tuple,
    },
    value::Value,
};

/// `is_dataclass(obj)`: true when `obj` is a dataclass **class** or an
/// **instance** of one (matching CPython, which accepts both).
pub(super) fn is_dataclass(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let arg = args.get_one_arg("is_dataclass", vm.heap)?;
    let result = dataclass_class_of(&arg, vm).is_some();
    arg.drop_with(vm);
    Ok(Value::Bool(result))
}

/// The decorated class behind `value`, whether it *is* one or is an instance of
/// one; `None` when neither.
fn dataclass_class_of(value: &Value, vm: &VM<'_>) -> Option<HeapId> {
    let &Value::Ref(id) = value else { return None };
    let class_id = match vm.heap.get(id) {
        HeapData::Class(_) => id,
        HeapData::Instance(instance) => instance.class(),
        _ => return None,
    };
    is_dataclass_class(class_id, vm).then_some(class_id)
}

/// The decorated class behind an *instance* only, which is what the three
/// conversion helpers require; `None` for a class or anything else.
fn dataclass_instance_class(value: &Value, vm: &VM<'_>) -> Option<HeapId> {
    let &Value::Ref(id) = value else { return None };
    let HeapData::Instance(instance) = vm.heap.get(id) else {
        return None;
    };
    let class_id = instance.class();
    is_dataclass_class(class_id, vm).then_some(class_id)
}

/// `fields(class_or_instance)`: the class's `Field` objects in definition
/// order, as a tuple.
///
/// The very objects `__dataclass_fields__` holds, as in CPython, so identity
/// and any later mutation are shared.
pub(super) fn fields(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let arg = args.get_one_arg("fields", vm.heap)?;
    defer_drop!(arg, vm);
    let Some(class_id) = dataclass_class_of(arg, vm) else {
        return Err(ExcType::type_error("must be called with a dataclass type or instance"));
    };
    let fields_id = class_fields_dict_id(class_id, vm).expect("a dataclass advertises its fields");
    let HeapData::Dict(fields) = vm.heap.get(fields_id) else {
        unreachable!("__dataclass_fields__ is a dict")
    };
    let items: SmallVec<[Value; 2]> = fields.iter().map(|(_, field)| field.clone_with_heap(vm.heap)).collect();
    Ok(allocate_tuple(items, vm.heap))
}

/// `asdict(instance, *, dict_factory=dict)`: the instance as a nested dict.
pub(super) fn asdict(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let AsdictArgs { obj, dict_factory } = AsdictArgs::from_args(args, vm)?;
    defer_drop!(obj, vm);
    defer_drop!(dict_factory, vm);
    if dataclass_instance_class(obj, vm).is_none() {
        return Err(ExcType::type_error("asdict() should be called on dataclass instances"));
    }
    convert(obj, &Shape::Dict(dict_factory), vm)
}

/// `astuple(instance, *, tuple_factory=tuple)`: the instance as a nested tuple.
pub(super) fn astuple(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let AstupleArgs { obj, tuple_factory } = AstupleArgs::from_args(args, vm)?;
    defer_drop!(obj, vm);
    defer_drop!(tuple_factory, vm);
    if dataclass_instance_class(obj, vm).is_none() {
        return Err(ExcType::type_error("astuple() should be called on dataclass instances"));
    }
    convert(obj, &Shape::Tuple(tuple_factory), vm)
}

/// Which shape a dataclass collapses into, and the factory building it.
///
/// The factory is `Value::None` when the caller did not pass one, which keeps
/// the builtin `dict` / `tuple` result CPython's defaults produce.
enum Shape<'v> {
    /// `asdict`: name/value pairs handed to `dict_factory`.
    Dict(&'v Value),
    /// `astuple`: values handed to `tuple_factory`.
    Tuple(&'v Value),
}

/// CPython's `_asdict_inner` / `_astuple_inner`: dataclasses collapse, the
/// three builtin containers are rebuilt around their converted items, and
/// anything else is passed through.
///
/// CPython deep-copies that last case; Monty has no `copy.deepcopy`, so the
/// value is shared with the original (see `limitations/dataclasses.md`).
fn convert(value: &Value, shape: &Shape<'_>, vm: &mut VM<'_>) -> RunResult<Value> {
    // Bound like every other recursive value walk, so a self-referential
    // dataclass raises rather than overflowing the host stack.
    let mut recursion = vm.recursion_guard()?;
    let vm = &mut *recursion;
    if let Some(class_id) = dataclass_instance_class(value, vm) {
        let &Value::Ref(self_id) = value else {
            unreachable!("a dataclass instance is a heap value")
        };
        return convert_dataclass(self_id, class_id, shape, vm);
    }
    let &Value::Ref(id) = value else {
        return Ok(value.clone_with_heap(vm.heap));
    };
    // Cloned out before converting: the conversion needs the heap mutably.
    let cloned = match vm.heap.get(id) {
        HeapData::List(list) => Cloned::List(clone_values(list.as_slice(), vm)),
        HeapData::Tuple(tuple) => Cloned::Tuple(clone_values(tuple.as_slice(), vm)),
        HeapData::Dict(dict) => Cloned::Dict(
            dict.iter()
                .map(|(k, v)| (k.clone_with_heap(vm.heap), v.clone_with_heap(vm.heap)))
                .collect(),
        ),
        _ => return Ok(value.clone_with_heap(vm.heap)),
    };
    match cloned {
        Cloned::List(items) => {
            let converted = convert_values(items, shape, vm)?;
            Ok(Value::Ref(vm.heap.allocate(HeapData::List(List::new(converted)))))
        }
        Cloned::Tuple(items) => {
            let converted = convert_values(items, shape, vm)?;
            Ok(allocate_tuple(SmallVec::from_vec(converted), vm.heap))
        }
        Cloned::Dict(pairs) => {
            let converted = convert_pairs(pairs, shape, vm)?;
            let dict = Dict::from_pairs(converted, vm)?;
            Ok(Value::Ref(vm.heap.allocate(HeapData::Dict(dict))))
        }
    }
}

/// A container's contents, lifted out of the heap so the walk can recurse.
enum Cloned {
    List(Vec<Value>),
    Tuple(Vec<Value>),
    Dict(Vec<(Value, Value)>),
}

/// Fresh references to every item of a borrowed slice.
fn clone_values(items: &[Value], vm: &VM<'_>) -> Vec<Value> {
    items.iter().map(|v| v.clone_with_heap(vm.heap)).collect()
}

/// Converts each item, taking ownership of `items` and releasing everything on
/// the way out if one conversion fails.
fn convert_values(items: Vec<Value>, shape: &Shape<'_>, vm: &mut VM<'_>) -> RunResult<Vec<Value>> {
    defer_drop!(items, vm);
    let out: Vec<Value> = Vec::with_capacity(items.len());
    let mut guard = DropGuard::new(out, vm);
    let (out, vm) = guard.as_parts_mut();
    for item in items {
        let converted = convert(item, shape, vm)?;
        out.push(converted);
    }
    Ok(guard.into_inner())
}

/// [`convert_values`] for a dict's entries: CPython converts keys as well.
fn convert_pairs(pairs: Vec<(Value, Value)>, shape: &Shape<'_>, vm: &mut VM<'_>) -> RunResult<Vec<(Value, Value)>> {
    defer_drop!(pairs, vm);
    let out: Vec<(Value, Value)> = Vec::with_capacity(pairs.len());
    let mut guard = DropGuard::new(out, vm);
    let (out, vm) = guard.as_parts_mut();
    for (key, value) in pairs {
        let key = convert(key, shape, vm)?;
        // Guarded before the second conversion, which can raise.
        let mut converted_key = DropGuard::new(key, vm);
        let (_, vm) = converted_key.as_parts();
        let value = convert(value, shape, vm)?;
        out.push((converted_key.into_inner(), value));
    }
    Ok(guard.into_inner())
}

/// Collapses one dataclass instance into the requested shape.
fn convert_dataclass(self_id: HeapId, class_id: HeapId, shape: &Shape<'_>, vm: &mut VM<'_>) -> RunResult<Value> {
    let names: Vec<_> = field_specs(class_id, vm).iter().map(|spec| spec.name).collect();
    let values: Vec<Value> = Vec::with_capacity(names.len());
    let mut guard = DropGuard::new(values, vm);
    let (values, vm) = guard.as_parts_mut();
    for &name in &names {
        let field_name = vm.interns.get_str(name).to_owned();
        // A field left unset (`init=False` with no default) has no value to
        // convert, so the attribute read raises exactly as CPython's does.
        let Some(value) = instance_attr(self_id, &field_name, vm) else {
            let owner = class_name(class_id, vm.heap, vm.interns).into_owned();
            return Err(ExcType::attribute_error(&owner, &field_name));
        };
        defer_drop!(value, vm);
        let converted = convert(value, shape, vm)?;
        values.push(converted);
    }
    let (values, vm) = guard.into_parts();
    match shape {
        Shape::Tuple(factory) => build_tuple(factory, values, vm),
        Shape::Dict(factory) => {
            let pairs = names.into_iter().map(Value::InternString).zip(values).collect();
            build_dict(factory, pairs, vm)
        }
    }
}

/// The `astuple` result: a plain tuple, or whatever `tuple_factory` makes of
/// the list of values CPython hands it.
fn build_tuple(factory: &Value, values: Vec<Value>, vm: &mut VM<'_>) -> RunResult<Value> {
    if matches!(factory, Value::None) {
        return Ok(allocate_tuple(SmallVec::from_vec(values), vm.heap));
    }
    let list = Value::Ref(vm.heap.allocate(HeapData::List(List::new(values))));
    vm.evaluate_function("dataclasses tuple_factory", factory, ArgValues::One(list))
}

/// The `asdict` result: a plain dict, or whatever `dict_factory` makes of the
/// list of `(name, value)` pairs CPython hands it.
fn build_dict(factory: &Value, pairs: Vec<(Value, Value)>, vm: &mut VM<'_>) -> RunResult<Value> {
    if matches!(factory, Value::None) {
        let dict = Dict::from_pairs(pairs, vm)?;
        return Ok(Value::Ref(vm.heap.allocate(HeapData::Dict(dict))));
    }
    let items: Vec<Value> = Vec::with_capacity(pairs.len());
    let mut guard = DropGuard::new(items, vm);
    let (items, vm) = guard.as_parts_mut();
    for (key, value) in pairs {
        items.push(allocate_tuple(SmallVec::from_vec(vec![key, value]), vm.heap));
    }
    let (items, vm) = guard.into_parts();
    let list = Value::Ref(vm.heap.allocate(HeapData::List(List::new(items))));
    vm.evaluate_function("dataclasses dict_factory", factory, ArgValues::One(list))
}

/// `replace(instance, /, **changes)`: constructs a new instance of the same
/// class, taking every unchanged field from `instance`.
///
/// Runs the construction to completion, so a user `__init__` or a
/// `__post_init__` that suspends on an external call raises here (see
/// `limitations/dataclasses.md`).
pub(super) fn replace(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let ReplaceArgs { obj, changes } = ReplaceArgs::from_args(args, vm)?;
    defer_drop!(obj, vm);
    let pairs: Vec<(Value, Value)> = changes.into_iter().collect();
    let mut guard = DropGuard::new(pairs, vm);
    let (pairs, vm) = guard.as_parts_mut();

    let Some(class_id) = dataclass_instance_class(obj, vm) else {
        return Err(ExcType::type_error("replace() should be called on dataclass instances"));
    };
    let &Value::Ref(self_id) = obj else {
        unreachable!("a dataclass instance is a heap value")
    };

    for spec in field_specs(class_id, vm) {
        let field_name = vm.interns.get_str(spec.name).to_owned();
        let supplied = pairs.iter().any(|(key, _)| is_named(key, &field_name, vm));
        if !spec.init {
            if supplied {
                return Err(ExcType::type_error(format!(
                    "field {field_name} is declared with init=False, it cannot be specified with replace()"
                )));
            }
            // CPython leaves an init=False field to the new instance's own
            // construction, so its current value is not carried over.
            continue;
        }
        if supplied {
            continue;
        }
        let Some(value) = instance_attr(self_id, &field_name, vm) else {
            let owner = class_name(class_id, vm.heap, vm.interns).into_owned();
            return Err(ExcType::attribute_error(&owner, &field_name));
        };
        // Guarded by the pairs guard from here on.
        let key = allocate_string(field_name, vm.heap);
        pairs.push((key, value));
    }

    let (pairs, vm) = guard.into_parts();
    vm.heap.inc_ref(class_id);
    let class = Value::Ref(class_id);
    defer_drop!(class, vm);
    let args = ArgValues::Kwargs(KwargsValues::Pairs(pairs));
    vm.evaluate_function("replace", class, args)
}

/// Whether a keyword key names `field_name`. Keys are strings: `**` unpacking
/// is the only way a non-string one could arrive, and `DictMerge` rejects those
/// before the call.
fn is_named(key: &Value, field_name: &str, vm: &VM<'_>) -> bool {
    key.as_either_str(vm.heap)
        .is_some_and(|s| s.as_str(vm.interns) == field_name)
}

#[derive(FromArgs)]
#[from_args(name = "asdict", style = def)]
struct AsdictArgs {
    #[from_args(pos_only)]
    obj: Value,
    #[from_args(kw_only, default = Value::None)]
    dict_factory: Value,
}

#[derive(FromArgs)]
#[from_args(name = "astuple", style = def)]
struct AstupleArgs {
    #[from_args(pos_only)]
    obj: Value,
    #[from_args(kw_only, default = Value::None)]
    tuple_factory: Value,
}

#[derive(FromArgs)]
#[from_args(name = "replace", style = def)]
struct ReplaceArgs {
    #[from_args(pos_only)]
    obj: Value,
    #[from_args(varkwargs)]
    changes: KwargsValues,
}
