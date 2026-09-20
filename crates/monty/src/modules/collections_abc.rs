//! Implementation of the `collections.abc` module.
//!
//! The module is annotations only: every name is a [`Marker`], the same value
//! `typing` exports for the same name, so `Callable`, `Mapping` and the rest
//! resolve where a program writes them. None of them is callable or
//! subscriptable, and `isinstance(x, Sequence)` is not supported; see
//! `limitations/collections.md`.

use crate::{
    bytecode::VM,
    heap::HeapId,
    intern::StaticStrings,
    types::Module,
    value::{Marker, Value},
};

/// Creates the `collections.abc` module and allocates it on the heap.
pub fn create_module(vm: &mut VM<'_>) -> HeapId {
    let mut module = Module::new(StaticStrings::CollectionsAbc, vm.interns);
    for ss in ABC_ATTRS {
        module.set_attr(*ss, Value::Marker(Marker(*ss)), vm);
    }
    vm.heap.allocate_as(module).into_id()
}

/// The abstract base classes this module exports.
///
/// A subset of CPython's: the ones `typing` already re-exports as markers, so
/// the same name means the same value whichever module a program takes it
/// from. The rest raise `ImportError`.
const ABC_ATTRS: &[StaticStrings] = &[
    StaticStrings::Callable,
    StaticStrings::CoroutineType,
    StaticStrings::Generator,
    StaticStrings::Iterable,
    StaticStrings::IteratorType,
    StaticStrings::Mapping,
    StaticStrings::Sequence,
];
