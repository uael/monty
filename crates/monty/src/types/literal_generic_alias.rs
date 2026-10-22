//! `typing._LiteralGenericAlias` — the runtime value of `Literal['a', 'b']`.
//!
//! A literal alias records the values it was subscripted with, as `__args__`,
//! and keeps the `typing.Literal` form as `__origin__`. Unlike a
//! `types.GenericAlias` it holds values rather than types, so it compares as
//! a set of `(type, value)` pairs, flattens a nested `Literal` and drops a
//! repeated value, as CPython's `typing` module does.

use std::fmt::Write;

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::{
    args::ArgValues,
    bytecode::{CallResult, VM},
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunResult},
    hash::{HashValue, hash_one},
    heap::{ContainsHeap, DropGuard, DropWithContext, HeapData, HeapId, HeapItem, HeapObjectRead, HeapReadOutput},
    intern::StaticStrings,
    types::{
        LazyHeapSet, PyTrait, Type, Union, allocate_string, generic_alias::repr_type_arg, list::repr_check_time,
        tuple::allocate_tuple, union::tuple_items,
    },
    value::{EitherStr, Marker, Value},
};

/// A subscripted `typing.Literal`, such as `Literal['a', 'b']`.
///
/// `args` is an OWNED ref to the `__args__` tuple, so `py_dec_ref_ids` and
/// `for_each_child_id` must both release it. Its values are flattened and
/// deduplicated by type and value in first-seen order, as
/// [`LiteralGenericAlias::subscript`] builds them.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct LiteralGenericAlias {
    /// The `__args__` tuple, always a `Value::Ref` to a `HeapData::Tuple`.
    args: Value,
}

impl LiteralGenericAlias {
    /// Builds `typing.Literal[key]`, taking ownership of `key`.
    ///
    /// A tuple key gives the values, anything else is the one value. A value
    /// that is itself a literal alias gives its own values, and a repeated
    /// `(type, value)` pair is dropped, an unhashable one too.
    pub(crate) fn subscript(key: Value, vm: &mut VM<'_>) -> RunResult<Value> {
        // Guarded because the item clone can fail its allocation preflight,
        // and `key` must be released on that path too.
        let mut key_guard = DropGuard::new(key, vm);
        let items = {
            let (key, vm) = key_guard.as_parts_mut();
            tuple_items(key, vm)?
        };
        let params = match items {
            Some(items) => {
                drop(key_guard);
                items
            }
            None => vec![key_guard.into_inner()],
        };
        let flat = flatten(params, vm)?;
        let values = deduplicate(flat, vm)?;
        let args = allocate_tuple(SmallVec::from_vec(values), vm.heap);
        Ok(Value::Ref(
            vm.heap.allocate(HeapData::LiteralGenericAlias(Self { args })),
        ))
    }

    /// An owned ref to the `__args__` tuple.
    pub(crate) fn args(&self, heap: &impl ContainsHeap) -> Value {
        self.args.clone_with_heap(heap)
    }

    /// Invokes `on_child` for the heap id this alias owns (GC trace hook).
    pub(crate) fn for_each_child_id(&self, mut on_child: impl FnMut(HeapId)) {
        if let Value::Ref(id) = self.args {
            on_child(id);
        }
    }
}

/// The values of `params`, with the values of each literal alias among them in
/// its place, as CPython's `_flatten_literal_params`. Takes ownership of `params`.
///
/// One level is enough: a literal alias holds no literal alias, since its own
/// construction flattened it.
fn flatten(params: Vec<Value>, vm: &mut VM<'_>) -> RunResult<Vec<Value>> {
    defer_drop!(params, vm);
    let mut guard = DropGuard::new(Vec::with_capacity(params.len()), vm);
    let (flat, vm) = guard.as_parts_mut();
    for param in params {
        match literal_values(param, vm)? {
            Some(values) => flat.extend(values),
            None => flat.push(param.clone_with_heap(vm)),
        }
    }
    Ok(guard.into_inner())
}

/// Owned clones of the values of a literal alias, or `None` when `value` is
/// not one.
fn literal_values(value: &Value, vm: &mut VM<'_>) -> RunResult<Option<Vec<Value>>> {
    let Value::Ref(id) = value else { return Ok(None) };
    let HeapData::LiteralGenericAlias(alias) = vm.heap.get(*id) else {
        return Ok(None);
    };
    let args = alias.args(vm.heap);
    defer_drop!(args, vm);
    tuple_items(args, vm)
}

/// The first of each `(type, value)` pair of `values`, in order. Takes
/// ownership of `values`.
///
/// CPython deduplicates through a dict, and through a list scan when a value is
/// unhashable; both keep the same values as this scan, which is linear per
/// value and so polls the time limit.
fn deduplicate(values: Vec<Value>, vm: &mut VM<'_>) -> RunResult<Vec<Value>> {
    defer_drop!(values, vm);
    let mut guard = DropGuard::new(Vec::with_capacity(values.len()), vm);
    let (unique, vm) = guard.as_parts_mut();
    for value in values {
        if !contains_pair(unique, value, vm)? {
            unique.push(value.clone_with_heap(vm));
        }
    }
    Ok(guard.into_inner())
}

/// Whether `values` holds a value of the same type as `value` and equal to it,
/// which is how CPython compares the `(value, type)` pairs of a literal.
fn contains_pair(values: &[Value], value: &Value, vm: &mut VM<'_>) -> RunResult<bool> {
    let ty = value.py_type(vm);
    for (index, existing) in values.iter().enumerate() {
        vm.heap.tracker.check_memory_time_every(index)?;
        if existing.py_type(vm) == ty && existing.py_eq(value, vm)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The hash of each `(value, type)` pair of `values`, or the TypeError CPython
/// raises when it puts an unhashable pair in a set.
fn pair_hashes(values: &[Value], vm: &mut VM<'_>) -> RunResult<Vec<HashValue>> {
    let mut hashes = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        vm.heap.tracker.check_memory_time_every(index)?;
        let Some(hash) = value.py_hash(vm)? else {
            return Err(ExcType::type_error_unhashable_set_element(
                "tuple",
                &value.py_type_name(vm),
            ));
        };
        hashes.push(hash_one((hash.raw(), value.py_type(vm))));
    }
    Ok(hashes)
}

/// Releases the args tuple of an alias abandoned before it reaches the heap;
/// a heap-stored alias is freed through [`HeapItem::py_dec_ref_ids`].
impl<C: ContainsHeap> DropWithContext<C> for LiteralGenericAlias {
    fn drop_with(self, ctx: &mut C) {
        self.args.drop_with(ctx);
    }
}

impl HeapItem for LiteralGenericAlias {
    fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        self.args.py_dec_ref_ids(stack);
    }
}

impl<'h> HeapObjectRead<'h, LiteralGenericAlias> {
    /// Owned clones of the values of `__args__`.
    fn values(&self, vm: &mut VM<'h>) -> RunResult<Vec<Value>> {
        let args = self.get(vm.heap).args(vm.heap);
        defer_drop!(args, vm);
        let Some(values) = tuple_items(args, vm)? else {
            unreachable!("LiteralGenericAlias::args is always a tuple")
        };
        Ok(values)
    }

    /// The alias's repr as a plain string, for error messages that name it.
    fn repr_string(&self, vm: &mut VM<'h>) -> RunResult<String> {
        let mut repr = String::new();
        self.py_repr_fmt(&mut repr, vm, &mut LazyHeapSet::default())?;
        Ok(repr)
    }
}

impl<'h> PyTrait<'h> for HeapObjectRead<'h, LiteralGenericAlias> {
    /// A literal alias names values, not a class, so CPython's `_SpecialForm`
    /// refuses the call.
    fn py_call(&mut self, args: ArgValues, vm: &mut VM<'h>) -> RunResult<CallResult> {
        args.drop_with(vm);
        Err(ExcType::type_error_cannot_instantiate("typing.Literal"))
    }

    fn py_type(&self, _: &VM<'h>) -> Type {
        Type::LiteralGenericAlias
    }

    fn py_len(&self, _: &VM<'h>) -> Option<usize> {
        None
    }

    /// Two literal aliases are equal when they hold the same `(type, value)`
    /// pairs in any order, so `Literal[1, 2] == Literal[2, 1]` and
    /// `Literal[1] != Literal[True]`. An unhashable value raises, as the set
    /// that CPython builds does.
    fn py_eq_impl(&self, other: &Value, vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        let Some(HeapReadOutput::LiteralGenericAlias(other)) = other.read_heap(vm) else {
            return Ok(None);
        };
        let mine = self.values(vm)?;
        defer_drop!(mine, vm);
        let theirs = other.values(vm)?;
        defer_drop!(theirs, vm);
        pair_hashes(mine, vm)?;
        pair_hashes(theirs, vm)?;
        // Hashable values are deduplicated, so equal lengths plus containment
        // one way is set equality.
        if mine.len() != theirs.len() {
            return Ok(Some(false));
        }
        for value in mine {
            if !contains_pair(theirs, value, vm)? {
                return Ok(Some(false));
            }
        }
        Ok(Some(true))
    }

    /// Order-independent, so it agrees with `py_eq_impl`; an unhashable value
    /// (`Literal[[1]]`) raises the TypeError of CPython's frozenset.
    fn py_hash(&self, vm: &mut VM<'h>) -> RunResult<Option<HashValue>> {
        let values = self.values(vm)?;
        defer_drop!(values, vm);
        let combined = pair_hashes(values, vm)?
            .into_iter()
            .fold(values.len() as u64, |acc, hash| acc.wrapping_add(hash_one(hash).raw()));
        Ok(Some(HashValue::new(combined)))
    }

    /// `typing.Literal['a', 'b']`, `typing.Literal[None]`, `typing.Literal[()]`.
    ///
    /// Values render as the arguments of a generic alias do. Takes a recursion
    /// level like the container reprs: a value can be a list that holds this
    /// alias, and that cycle prints as `...`.
    fn py_repr_fmt(&self, f: &mut impl Write, vm: &mut VM<'h>, heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        let Ok(mut guard) = vm.recursion_guard() else {
            return Ok(f.write_str("...")?);
        };
        let vm = &mut *guard;
        let args = self.get(vm.heap).args(vm.heap);
        defer_drop!(args, vm);
        let Some(HeapReadOutput::Tuple(args)) = args.read_heap(vm) else {
            unreachable!("LiteralGenericAlias::args is always a tuple")
        };
        f.write_str("typing.Literal[")?;
        let count = args.get(vm.heap).as_slice().len();
        if count == 0 {
            f.write_str("()")?;
        }
        for index in 0..count {
            if index > 0 {
                if repr_check_time(index, vm) {
                    f.write_str(", ...[timeout]")?;
                    break;
                }
                f.write_str(", ")?;
            }
            let item = args.clone_item(index, vm);
            defer_drop!(item, vm);
            repr_type_arg(item, f, vm, heap_ids)?;
        }
        Ok(f.write_char(']')?)
    }

    /// `__origin__` (the `typing.Literal` form itself), `__args__`,
    /// `__parameters__` (always `()`) and `__name__` / `__qualname__`
    /// (`'Literal'`); any other name is an `AttributeError` on the alias.
    fn py_getattr(&self, attr: &EitherStr, vm: &mut VM<'h>) -> RunResult<Option<CallResult>> {
        let value = match attr.static_string(vm.interns) {
            Some(StaticStrings::DunderOrigin) => Value::Marker(Marker(StaticStrings::Literal)),
            Some(StaticStrings::DunderArgs) => self.get(vm.heap).args(vm.heap),
            Some(StaticStrings::DunderParameters) => vm.heap.get_empty_tuple(),
            Some(StaticStrings::DunderName | StaticStrings::DunderQualname) => allocate_string("Literal", vm.heap),
            _ => return Ok(None),
        };
        Ok(Some(CallResult::Value(value)))
    }

    /// CPython's `_GenericAlias.__or__` takes any operand, so
    /// `Literal['a'] | 1` is a union too.
    fn py_or_impl(&self, other: &Value, vm: &mut VM<'h>) -> RunResult<Option<Value>> {
        Union::heap_or(self, other, vm)
    }

    fn py_ror_impl(&self, other: &Value, vm: &mut VM<'h>) -> RunResult<Option<Value>> {
        Union::heap_ror(self, other, vm)
    }

    /// A literal alias has no type variables to fill, so `Literal['a']['b']`
    /// fails as a generic alias does.
    fn py_getitem(&self, _key: &Value, vm: &mut VM<'h>) -> RunResult<Value> {
        let repr = self.repr_string(vm)?;
        Err(ExcType::type_error_not_generic_class(&repr))
    }
}
