//! Implementation of the `functools` module.
//!
//! Only `partialmethod` so far — see `limitations/functools.md`. `partial`,
//! `reduce`, `wraps`, `cache`/`lru_cache`, `cached_property` and
//! `total_ordering` are absent from the namespace rather than stubbed, so they
//! raise `AttributeError` up front and fail type checking too.

use crate::{
    builtins::Builtins,
    bytecode::VM,
    heap::{HeapData, HeapId},
    intern::StaticStrings,
    types::{Module, Type},
    value::Value,
};

/// Creates the `functools` module on the heap.
///
/// # Panics
/// Panics if the required strings have not been pre-interned during prepare phase.
pub fn create_module(vm: &mut VM<'_>) -> HeapId {
    let mut module = Module::new(StaticStrings::Functools);
    module.set_attr(
        StaticStrings::Partialmethod,
        Value::Builtin(Builtins::Type(Type::PartialMethod)),
        vm,
    );
    vm.heap.allocate(HeapData::Module(Box::new(module)))
}
