//! PEP 695 type parameters: `typing.TypeVar`, the value `T` holds inside
//! `class Held[T](Spawned[T])`.
//!
//! One object per execution of the construct that declares it, so `T is T`
//! within its scope, and nothing more: Monty evaluates no bound, no default and
//! no constraint, because none of them can change what a type expression built
//! from `T` does at runtime.

use std::fmt::Write;

use crate::{
    bytecode::{CallResult, VM},
    exception_private::RunResult,
    hash::{HashValue, identity_hash},
    heap::{HeapData, HeapId, HeapItem, HeapRead},
    intern::{StaticStrings, StringId},
    types::{LazyHeapSet, PyTrait, Type},
    value::{EitherStr, Value},
};

/// A PEP 695 type parameter object.
///
/// Leaf type: the name is interned, so a `TypeVar` owns no heap reference and
/// cannot take part in a cycle.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct TypeVar {
    /// `__name__`, which is also the whole of `repr`.
    name: StringId,
}

/// Allocates the `TypeVar` a `class C[T]` statement binds to `T`.
pub(crate) fn allocate_type_var(name: StringId, vm: &mut VM<'_>) -> Value {
    Value::Ref(vm.heap.allocate(HeapData::TypeVar(TypeVar { name })))
}

impl HeapItem for TypeVar {
    fn py_dec_ref_ids(&mut self, _stack: &mut Vec<HeapId>) {}
}

impl<'h> PyTrait<'h> for HeapRead<'h, TypeVar> {
    fn py_type(&self, _vm: &VM<'h>) -> Type {
        Type::TypeVar
    }

    fn py_len(&self, _vm: &VM<'h>) -> Option<usize> {
        None
    }

    /// CPython's `TypeVar` defines no `__eq__`, so two same-named parameters
    /// compare unequal and identity is resolved before this.
    fn py_eq_impl(&self, _other: &Value, _vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        Ok(None)
    }

    fn py_hash(&self, self_id: HeapId, _vm: &mut VM<'h>) -> RunResult<Option<HashValue>> {
        Ok(Some(identity_hash(self_id)))
    }

    /// Just the name. CPython prefixes an explicitly-variant `TypeVar` with
    /// `~`, `+` or `-`, but a PEP 695 parameter infers its variance and prints
    /// bare, and PEP 695 is the only way to get one here.
    fn py_repr_fmt(&self, f: &mut impl Write, vm: &mut VM<'h>, _heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        Ok(f.write_str(vm.interns.get_str(self.get(vm.heap).name))?)
    }

    fn py_getattr(&self, attr: &EitherStr, vm: &mut VM<'h>) -> RunResult<Option<CallResult>> {
        let name = Value::InternString(self.get(vm.heap).name);
        Ok(match attr.static_string() {
            Some(StaticStrings::DunderName) => Some(CallResult::Value(name)),
            Some(_) | None if attr.as_str(vm.interns) == "__name__" => Some(CallResult::Value(name)),
            _ => None,
        })
    }
}
