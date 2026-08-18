//! Runtime type forms: `types.GenericAlias` (`list[int]`) and `typing.Union`
//! (`int | str`).
//!
//! The two live together because they are one another's operands: `|` on an
//! alias builds a union, a union's members are rendered by the same
//! type-expression printer, and both answer `typing.get_origin`/`get_args`.
//! Neither validates anything — they are the objects an annotation *evaluates
//! to*, which is what makes `class Held(Spawned[T])` and `isinstance(x, int |
//! str)` work.

use std::fmt::Write;

use smallvec::{SmallVec, smallvec};

use crate::{
    args::ArgValues,
    builtins::Builtins,
    bytecode::{CallResult, VM},
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunResult},
    hash::{HashValue, hash_one},
    heap::{DropGuard, DropWithContext, HeapData, HeapId, HeapItem, HeapRead},
    intern::StaticStrings,
    types::{
        LazyHeapSet, PyTrait, Type, allocate_tuple,
        instance::{class_member, class_name, descriptor_class_get},
    },
    value::{EitherStr, Value},
};

/// `types.GenericAlias`: what subscripting a class produces (`list[int]`).
///
/// `origin` is the subscripted class and `args` the (always tuple) subscript.
/// Both are owned references, released by [`HeapItem::py_dec_ref_ids`]. The
/// object is deliberately inert: it forwards attribute reads to its origin and
/// calls straight through to it, so `list[int]()` builds a plain list.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct GenericAlias {
    /// `__origin__`: the class that was subscripted.
    origin: Value,
    /// `__args__`: an owned reference to the subscript tuple.
    args: Value,
}

/// `typing.Union`, which CPython 3.14 also exports as `types.UnionType`: the
/// value of `int | str`.
///
/// `args` is an owned reference to a tuple that is already flattened and
/// deduplicated (a union is never a member of a union, and never repeats a
/// member) and always holds at least two members — a one-member union collapses
/// to the member itself, so this type never represents one.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct UnionType {
    /// `__args__`: an owned reference to the flattened member tuple.
    args: Value,
}

/// Allocates a `GenericAlias`, taking ownership of `origin` and `args`.
///
/// `args` must be a tuple reference; the subscript path is the only caller and
/// wraps a non-tuple subscript in a one-element tuple first, as CPython does.
pub(crate) fn allocate_generic_alias(origin: Value, args: Value, vm: &mut VM<'_>) -> Value {
    Value::Ref(vm.heap.allocate(HeapData::GenericAlias(GenericAlias { origin, args })))
}

impl GenericAlias {
    /// The class this alias subscripted, borrowed for the call-through path.
    pub(crate) fn origin(&self) -> &Value {
        &self.origin
    }

    /// Runs `f` on every owned reference. Backs the heap's GC child walker, and
    /// MUST report the same references as [`HeapItem::py_dec_ref_ids`].
    pub(crate) fn for_each_owned_value(&self, mut f: impl FnMut(&Value)) {
        f(&self.origin);
        f(&self.args);
    }
}

impl UnionType {
    /// Runs `f` on every owned reference. Backs the heap's GC child walker, and
    /// MUST report the same references as [`HeapItem::py_dec_ref_ids`].
    pub(crate) fn for_each_owned_value(&self, mut f: impl FnMut(&Value)) {
        f(&self.args);
    }
}

impl HeapItem for GenericAlias {
    fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        self.origin.py_dec_ref_ids(stack);
        self.args.py_dec_ref_ids(stack);
    }
}

impl HeapItem for UnionType {
    fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        self.args.py_dec_ref_ids(stack);
    }
}

/// Writes one argument of a type expression the way CPython's `ga_repr_item`
/// does: `...` for `Ellipsis`, `None` for the `None` singleton and `NoneType`
/// alike, the qualified type name for a class, and `repr()` for anything else
/// (so a forward reference keeps its quotes).
///
/// A sandbox-defined class prints its bare name where CPython prints
/// `module.Qualname`; Monty gives sandbox classes no `__module__`. See
/// `limitations/typing.md`.
pub(crate) fn write_type_arg(
    value: &Value,
    f: &mut impl Write,
    vm: &mut VM<'_>,
    heap_ids: &mut LazyHeapSet,
) -> RunResult<()> {
    match value {
        Value::Ellipsis => Ok(f.write_str("...")?),
        Value::None | Value::Builtin(Builtins::Type(Type::NoneType)) => Ok(f.write_str("None")?),
        Value::Builtin(Builtins::Type(t)) => Ok(f.write_str(&t.name(vm.heap, vm.interns))?),
        Value::Builtin(Builtins::ExcType(e)) => Ok(f.write_str(e.into())?),
        // A builtin function prints bare (`list[len]`), matching CPython, which
        // omits a `builtins` module prefix.
        Value::Builtin(Builtins::Function(func)) => Ok(write!(f, "{func}")?),
        Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::Class(_)) => {
            Ok(f.write_str(&class_name(*id, vm.heap, vm.interns))?)
        }
        // A list argument is itself a type expression, which is how
        // `Callable[[int], str]` prints its parameter list.
        Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::List(_)) => {
            let items = match vm.heap.get(*id) {
                HeapData::List(list) => list
                    .as_slice()
                    .iter()
                    .map(|v| v.clone_with_heap(vm.heap))
                    .collect::<Vec<_>>(),
                _ => unreachable!("matched a list"),
            };
            defer_drop!(items, vm);
            f.write_str("[")?;
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    f.write_str(", ")?;
                }
                write_type_arg(item, f, vm, heap_ids)?;
            }
            Ok(f.write_str("]")?)
        }
        other => other.py_repr_fmt(f, vm, heap_ids),
    }
}

/// Writes a comma-separated argument list, with CPython's `tuple[()]` spelling
/// for the empty one.
fn write_type_args(args: &Value, f: &mut impl Write, vm: &mut VM<'_>, heap_ids: &mut LazyHeapSet) -> RunResult<()> {
    let items = clone_args(args, vm);
    defer_drop!(items, vm);
    if items.is_empty() {
        return Ok(f.write_str("()")?);
    }
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            f.write_str(", ")?;
        }
        write_type_arg(item, f, vm, heap_ids)?;
    }
    Ok(())
}

/// The Python hash of a type-form component, raising `unhashable type` the way
/// `hash()` does — a type form is only as hashable as the members it holds.
fn hash_member(value: &Value, vm: &mut VM<'_>) -> RunResult<HashValue> {
    match value.py_hash(vm)? {
        Some(hash) => Ok(hash),
        None => Err(ExcType::type_error_unhashable(&value.py_type_name(vm))),
    }
}

/// Clones the members out of an `__args__` tuple so the heap read handle is
/// released before anything that can re-enter the VM (a `repr` on a member, an
/// equality test) runs.
fn clone_args(args: &Value, vm: &mut VM<'_>) -> Vec<Value> {
    match args {
        Value::Ref(id) => match vm.heap.get(*id) {
            HeapData::Tuple(t) => t.as_slice().iter().map(|v| v.clone_with_heap(vm.heap)).collect(),
            _ => unreachable!("__args__ is always a tuple"),
        },
        _ => unreachable!("__args__ is always a tuple"),
    }
}

impl<'h> PyTrait<'h> for HeapRead<'h, GenericAlias> {
    fn py_type(&self, _vm: &VM<'h>) -> Type {
        Type::GenericAlias
    }

    fn py_len(&self, _vm: &VM<'h>) -> Option<usize> {
        None
    }

    /// Equal to another alias with an equal origin and equal args, as CPython's
    /// `ga_richcompare` does; unequal (never an error) against anything else.
    fn py_eq_impl(&self, other: &Value, vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        let Value::Ref(other_id) = other else {
            return Ok(Some(false));
        };
        if !matches!(vm.heap.get(*other_id), HeapData::GenericAlias(_)) {
            return Ok(Some(false));
        }
        let (mine, theirs) = (self.get(vm.heap), vm.heap.get(*other_id));
        let HeapData::GenericAlias(theirs) = theirs else {
            unreachable!("checked above");
        };
        let (my_origin, my_args) = (mine.origin.clone_with_heap(vm.heap), mine.args.clone_with_heap(vm.heap));
        let (their_origin, their_args) = (
            theirs.origin.clone_with_heap(vm.heap),
            theirs.args.clone_with_heap(vm.heap),
        );
        defer_drop!(my_origin, vm);
        defer_drop!(my_args, vm);
        defer_drop!(their_origin, vm);
        defer_drop!(their_args, vm);
        Ok(Some(
            my_origin.py_eq(their_origin, vm)? && my_args.py_eq(their_args, vm)?,
        ))
    }

    /// `hash(origin) ^ hash(args)`, as CPython's `ga_hash` computes it, so equal
    /// aliases hash equal.
    fn py_hash(&self, _self_id: HeapId, vm: &mut VM<'h>) -> RunResult<Option<HashValue>> {
        let (origin, args) = {
            let alias = self.get(vm.heap);
            (
                alias.origin.clone_with_heap(vm.heap),
                alias.args.clone_with_heap(vm.heap),
            )
        };
        defer_drop!(origin, vm);
        defer_drop!(args, vm);
        Ok(Some(HashValue::new(
            hash_member(origin, vm)?.raw() ^ hash_member(args, vm)?.raw(),
        )))
    }

    fn py_repr_fmt(&self, f: &mut impl Write, vm: &mut VM<'h>, heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        let Ok(mut guard) = vm.recursion_guard() else {
            return Ok(f.write_str("...")?);
        };
        let vm = &mut *guard;
        let (origin, args) = {
            let alias = self.get(vm.heap);
            (
                alias.origin.clone_with_heap(vm.heap),
                alias.args.clone_with_heap(vm.heap),
            )
        };
        defer_drop!(origin, vm);
        defer_drop!(args, vm);
        write_type_arg(origin, f, vm, heap_ids)?;
        f.write_str("[")?;
        write_type_args(args, f, vm, heap_ids)?;
        Ok(f.write_str("]")?)
    }

    /// `__origin__`/`__args__`, and every other name forwarded to the origin —
    /// CPython's `ga_getattro` does the same, which is why `list[int].__name__`
    /// is `'list'`.
    fn py_getattr(&self, attr: &EitherStr, vm: &mut VM<'h>) -> RunResult<Option<CallResult>> {
        let (origin, args) = {
            let alias = self.get(vm.heap);
            (
                alias.origin.clone_with_heap(vm.heap),
                alias.args.clone_with_heap(vm.heap),
            )
        };
        defer_drop!(origin, vm);
        defer_drop!(args, vm);
        match attr.as_str(vm.interns) {
            "__origin__" => Ok(Some(CallResult::Value(origin.clone_with_heap(vm.heap)))),
            "__args__" => Ok(Some(CallResult::Value(args.clone_with_heap(vm.heap)))),
            _ => origin.py_getattr(attr, vm).map(Some),
        }
    }
}

impl<'h> PyTrait<'h> for HeapRead<'h, UnionType> {
    fn py_type(&self, _vm: &VM<'h>) -> Type {
        Type::Union
    }

    fn py_len(&self, _vm: &VM<'h>) -> Option<usize> {
        None
    }

    /// CPython compares unions as *sets* of members, so `int | str == str | int`.
    fn py_eq_impl(&self, other: &Value, vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        let Value::Ref(other_id) = other else {
            return Ok(Some(false));
        };
        let HeapData::UnionType(theirs) = vm.heap.get(*other_id) else {
            return Ok(Some(false));
        };
        let their_args = theirs.args.clone_with_heap(vm.heap);
        let my_args = self.get(vm.heap).args.clone_with_heap(vm.heap);
        defer_drop!(my_args, vm);
        defer_drop!(their_args, vm);
        let mine = clone_args(my_args, vm);
        defer_drop!(mine, vm);
        let theirs = clone_args(their_args, vm);
        defer_drop!(theirs, vm);
        if mine.len() != theirs.len() {
            return Ok(Some(false));
        }
        for member in mine {
            let mut found = false;
            for other in theirs {
                if member.py_eq(other, vm)? {
                    found = true;
                    break;
                }
            }
            if !found {
                return Ok(Some(false));
            }
        }
        Ok(Some(true))
    }

    /// The XOR of the member hashes, so an order-insensitive equality has an
    /// order-insensitive hash (CPython hashes the frozenset of members).
    fn py_hash(&self, _self_id: HeapId, vm: &mut VM<'h>) -> RunResult<Option<HashValue>> {
        let args = self.get(vm.heap).args.clone_with_heap(vm.heap);
        defer_drop!(args, vm);
        let members = clone_args(args, vm);
        defer_drop!(members, vm);
        let mut hash = hash_one("typing.Union").raw();
        for member in members {
            hash ^= hash_member(member, vm)?.raw();
        }
        Ok(Some(HashValue::new(hash)))
    }

    fn py_repr_fmt(&self, f: &mut impl Write, vm: &mut VM<'h>, heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        let Ok(mut guard) = vm.recursion_guard() else {
            return Ok(f.write_str("...")?);
        };
        let vm = &mut *guard;
        let args = self.get(vm.heap).args.clone_with_heap(vm.heap);
        defer_drop!(args, vm);
        let members = clone_args(args, vm);
        defer_drop!(members, vm);
        for (i, member) in members.iter().enumerate() {
            if i > 0 {
                f.write_str(" | ")?;
            }
            write_type_arg(member, f, vm, heap_ids)?;
        }
        Ok(())
    }

    /// `__args__` and `__name__` only: a union forwards nothing, having no origin.
    fn py_getattr(&self, attr: &EitherStr, vm: &mut VM<'h>) -> RunResult<Option<CallResult>> {
        match attr.as_str(vm.interns) {
            "__args__" => Ok(Some(CallResult::Value(self.get(vm.heap).args.clone_with_heap(vm.heap)))),
            "__name__" => Ok(Some(CallResult::Value(Value::InternString(
                StaticStrings::UnionType.into(),
            )))),
            _ => Ok(None),
        }
    }
}

/// Whether `value` may stand in a type expression, which is what `|` and a
/// class subscript accept: a class, `None`, another type form, or a string
/// forward reference. CPython's `is_unionable`, minus the parameter forms Monty
/// has no equivalent for.
pub(crate) fn is_type_form(value: &Value, vm: &VM<'_>) -> bool {
    match value {
        Value::None | Value::Builtin(Builtins::Type(_) | Builtins::ExcType(_)) => true,
        Value::Ref(id) => matches!(
            vm.heap.get(*id),
            HeapData::Class(_) | HeapData::GenericAlias(_) | HeapData::UnionType(_) | HeapData::TypeAliasType(_)
        ),
        _ => false,
    }
}

/// Builds `left | right`, consuming both.
///
/// Rejects a non-type operand with the ordinary binary-operator wording, which
/// is what the reader wrote; `typing.Union[...]` takes the other entry point.
pub(crate) fn union_of(left: Value, right: Value, vm: &mut VM<'_>) -> RunResult<Value> {
    let unionable = is_type_form(&left, vm) && is_type_form(&right, vm);
    if !unionable {
        defer_drop!(left, vm);
        defer_drop!(right, vm);
        let (lhs_type, lhs_name) = (left.py_type(vm), left.py_type_name(vm).into_owned());
        return Err(ExcType::binary_type_error(
            "|",
            lhs_type,
            lhs_name,
            right.py_type_name(vm),
        ));
    }
    union_from_members(vec![left, right], vm)
}

/// Builds the union of `members`, consuming them.
///
/// Members are flattened out of nested unions, `None` is normalized to
/// `NoneType` (so `int | None` and `int | type(None)` are one type), duplicates
/// are dropped, and a single surviving member is returned bare — all four are
/// CPython's `_Py_union_type_or` behaviour, and what makes `typing.Union[int]`
/// simply `int`.
pub(crate) fn union_from_members(members: Vec<Value>, vm: &mut VM<'_>) -> RunResult<Value> {
    let mut given = DropGuard::new(members, vm);
    let mut candidates = Vec::new();
    for i in 0..given.as_parts().0.len() {
        let (given, vm) = given.as_parts();
        if !is_type_form(&given[i], vm) {
            let mut got = String::new();
            let mut heap_ids = LazyHeapSet::default();
            given[i].py_repr_fmt(&mut got, vm, &mut heap_ids)?;
            candidates.drop_with(vm);
            return Err(ExcType::type_error(format!(
                "Union[arg, ...]: each arg must be a type. Got {got}."
            )));
        }
        candidates.extend(union_members(&given[i], vm));
    }
    let (given, vm) = given.into_parts();
    given.drop_with(vm);
    if candidates.is_empty() {
        return Err(ExcType::type_error("Cannot take a Union of no types."));
    }
    let mut guard = DropGuard::new(candidates, vm);
    // Marked rather than filtered in place: `py_eq` can run a user `__eq__` and
    // raise, and the guard can only release a vector it still wholly owns.
    let mut keep: Vec<bool> = Vec::new();
    for i in 0..guard.as_parts().0.len() {
        let mut duplicate = false;
        for j in 0..i {
            if !keep[j] {
                continue;
            }
            let (candidates, vm) = guard.as_parts();
            if candidates[j].py_eq(&candidates[i], vm)? {
                duplicate = true;
                break;
            }
        }
        keep.push(!duplicate);
    }
    let (candidates, vm) = guard.into_parts();
    let mut members = Vec::with_capacity(keep.iter().filter(|k| **k).count());
    for (candidate, keep) in candidates.into_iter().zip(keep) {
        if keep {
            members.push(candidate);
        } else {
            candidate.drop_with(vm);
        }
    }
    if members.len() == 1 {
        return Ok(members.into_iter().next().expect("length checked"));
    }
    let args = allocate_tuple(SmallVec::from_vec(members), vm.heap);
    Ok(Value::Ref(vm.heap.allocate(HeapData::UnionType(UnionType { args }))))
}

/// The members `value` contributes to a union: a nested union's own members,
/// `NoneType` for `None`, otherwise the value itself. Each is a fresh owned
/// reference.
fn union_members(value: &Value, vm: &mut VM<'_>) -> Vec<Value> {
    match value {
        Value::None => vec![Value::Builtin(Builtins::Type(Type::NoneType))],
        Value::Ref(id) => match vm.heap.get(*id) {
            HeapData::UnionType(union) => {
                let args = union.args.clone_with_heap(vm.heap);
                let members = clone_args(&args, vm);
                args.drop_with(vm);
                members
            }
            _ => vec![value.clone_with_heap(vm.heap)],
        },
        _ => vec![value.clone_with_heap(vm.heap)],
    }
}

/// The members of `union_id`, cloned out for `isinstance` to walk.
pub(crate) fn union_arg_values(union_id: HeapId, vm: &mut VM<'_>) -> Vec<Value> {
    let HeapData::UnionType(union) = vm.heap.get(union_id) else {
        unreachable!("caller matched a union");
    };
    let args = union.args.clone_with_heap(vm.heap);
    let members = clone_args(&args, vm);
    args.drop_with(vm);
    members
}

/// Builds `origin[key]`, taking ownership of `origin`.
///
/// A tuple subscript becomes `__args__` unchanged (so `list[int, str]` and
/// `list[(int, str)]` are one alias, and `tuple[()]` has no arguments);
/// anything else is wrapped in a one-element tuple, as CPython's
/// `Py_GenericAlias` does.
pub(crate) fn subscript_type_form(origin: Value, key: &Value, vm: &mut VM<'_>) -> Value {
    let args = match key {
        Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::Tuple(_)) => key.clone_with_heap(vm.heap),
        other => allocate_tuple(smallvec![other.clone_with_heap(vm.heap)], vm.heap),
    };
    allocate_generic_alias(origin, args, vm)
}

/// Subscripts a sandbox class: `Foo[int]`.
///
/// Dispatches the `__class_getitem__` the class or one of its bases defines,
/// which CPython treats as an implicit classmethod, so a plain function in the
/// body receives the class before the subscript. A class defining none is not
/// generic and says so.
pub(crate) fn class_subscript(class_id: HeapId, key: &Value, vm: &mut VM<'_>) -> RunResult<Value> {
    let Some(member) = class_member(class_id, "__class_getitem__", vm) else {
        let name = class_name(class_id, vm.heap, vm.interns).into_owned();
        return Err(ExcType::type_error_not_sub_class(name));
    };
    let implicit_classmethod =
        !matches!(member, Value::Ref(id) if matches!(vm.heap.get(id), HeapData::MethodDescriptor(_)));
    let callable = descriptor_class_get(member, class_id, vm);
    defer_drop!(callable, vm);
    let key = key.clone_with_heap(vm.heap);
    let args = if implicit_classmethod {
        vm.heap.inc_ref(class_id);
        ArgValues::Two(Value::Ref(class_id), key)
    } else {
        ArgValues::One(key)
    };
    vm.evaluate_function("__class_getitem__", callable, args)
}

/// The `__origin__`/`__args__` pair `typing.get_origin`/`get_args` report, or
/// `None` when the value is not a type form built by subscription or `|`.
pub(crate) fn origin_and_args(value: &Value, vm: &mut VM<'_>) -> Option<(Value, Value)> {
    let Value::Ref(id) = value else {
        return None;
    };
    match vm.heap.get(*id) {
        HeapData::GenericAlias(alias) => Some((
            alias.origin.clone_with_heap(vm.heap),
            alias.args.clone_with_heap(vm.heap),
        )),
        HeapData::UnionType(union) => Some((
            Value::Builtin(Builtins::Type(Type::Union)),
            union.args.clone_with_heap(vm.heap),
        )),
        _ => None,
    }
}
