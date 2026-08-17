//! PEP 695 type aliases: `typing.TypeAliasType`, the value of `type X = ...`.

use std::{cell::RefCell, fmt::Write};

use crate::{
    args::ArgValues,
    bytecode::{CallResult, VM},
    exception_private::RunResult,
    hash::{HashValue, identity_hash},
    heap::{HeapData, HeapId, HeapItem, HeapRead},
    intern::{StaticStrings, StringId},
    types::{LazyHeapSet, PyTrait, Type},
    value::{EitherStr, Value},
};

/// CPython's `typing.TypeAliasType`: what `type X = <value>` binds to.
///
/// PEP 695 defers the right-hand side, which is what makes `type Wire = ... |
/// list[Wire] | ...` definable at all, so the value arrives as a zero-arg
/// callable and is only run on the first `__value__` read. Both `thunk` and the
/// memoized value are owned references, released by [`HeapItem::py_dec_ref_ids`].
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct TypeAliasType {
    /// `__name__`.
    name: StringId,
    /// Owned zero-arg callable whose call yields `__value__`; PEP 695 defers
    /// evaluation so an alias may mention itself.
    thunk: Value,
    /// Memoized `__value__`; CPython evaluates once and caches, so
    /// `X.__value__ is X.__value__` holds. `RefCell` because the memo is written
    /// from [`PyTrait::py_getattr`], which only has `&self`; no borrow is ever
    /// held across the thunk call, so a self-referential alias cannot deadlock it.
    value: RefCell<Option<Value>>,
}

/// Allocates a `TypeAliasType`, taking ownership of `thunk`.
pub(crate) fn allocate_type_alias(name: StringId, thunk: Value, vm: &mut VM<'_>) -> Value {
    Value::Ref(vm.heap.allocate(HeapData::TypeAliasType(TypeAliasType {
        name,
        thunk,
        value: RefCell::new(None),
    })))
}

impl TypeAliasType {
    /// Runs `f` on every owned reference: the thunk, plus `__value__` once it has
    /// been memoized. Backs the heap's GC child walker, and MUST report the same
    /// references as [`HeapItem::py_dec_ref_ids`] below.
    pub(crate) fn for_each_owned_value(&self, mut f: impl FnMut(&Value)) {
        f(&self.thunk);
        if let Some(value) = self.value.borrow().as_ref() {
            f(value);
        }
    }
}

impl<'h> PyTrait<'h> for HeapRead<'h, TypeAliasType> {
    fn py_type(&self, _vm: &VM<'h>) -> Type {
        Type::TypeAliasType
    }

    fn py_len(&self, _vm: &VM<'h>) -> Option<usize> {
        None
    }

    fn py_eq_impl(&self, _other: &Value, _vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        // CPython's `TypeAliasType` defines no `__eq__`: two aliases with the same
        // name and value compare unequal, and identity is resolved before this.
        Ok(None)
    }

    fn py_hash(&self, self_id: HeapId, _vm: &mut VM<'h>) -> RunResult<Option<HashValue>> {
        Ok(Some(identity_hash(self_id)))
    }

    /// Just the alias name: CPython prints `Wire`, not `<... object>`.
    fn py_repr_fmt(&self, f: &mut impl Write, vm: &mut VM<'h>, _heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        Ok(f.write_str(vm.interns.get_str(self.get(vm.heap).name))?)
    }

    fn py_getattr(&self, attr: &EitherStr, vm: &mut VM<'h>) -> RunResult<Option<CallResult>> {
        // Fast path: interned names match by id, with no string comparison.
        if let Some(ss) = attr.static_string() {
            return match ss {
                StaticStrings::DunderName => Ok(Some(CallResult::Value(Value::InternString(self.get(vm.heap).name)))),
                StaticStrings::DunderValue => self.alias_value(vm).map(|v| Some(CallResult::Value(v))),
                _ => Ok(None),
            };
        }
        // Slow path: a name built at runtime is a heap str.
        match attr.as_str(vm.interns) {
            "__name__" => Ok(Some(CallResult::Value(Value::InternString(self.get(vm.heap).name)))),
            "__value__" => self.alias_value(vm).map(|v| Some(CallResult::Value(v))),
            _ => Ok(None),
        }
    }
}

impl<'h> HeapRead<'h, TypeAliasType> {
    /// `__value__`: the memoized value, or the thunk's first (and only) result.
    ///
    /// The thunk re-enters the VM, so nothing borrowed from the heap entry (the
    /// `RefCell` memo included) may be held across the call. The alias itself
    /// stays alive because every `py_getattr` caller owns a reference to it.
    fn alias_value(&self, vm: &mut VM<'h>) -> RunResult<Value> {
        let cached = self
            .get(vm.heap)
            .value
            .borrow()
            .as_ref()
            .map(|value| value.clone_with_heap(vm.heap));
        if let Some(cached) = cached {
            return Ok(cached);
        }

        let thunk = self.get(vm.heap).thunk.clone_with_heap(vm.heap);
        let evaluated = vm.evaluate_function("type alias value", &thunk, ArgValues::Empty);
        thunk.drop_with(vm);
        let evaluated = evaluated?;

        let result = evaluated.clone_with_heap(vm.heap);
        // A self-referential alias can have memoized during the call above; that
        // value is replaced (and released) so identity stays with the last write.
        let previous = self.get(vm.heap).value.replace(Some(evaluated));
        if let Some(previous) = previous {
            previous.drop_with(vm);
        }
        Ok(result)
    }
}

impl HeapItem for TypeAliasType {
    fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        // Mirrors `for_each_owned_value`: the thunk plus any memoized `__value__`.
        self.thunk.py_dec_ref_ids(stack);
        if let Some(value) = self.value.get_mut() {
            value.py_dec_ref_ids(stack);
        }
    }
}
