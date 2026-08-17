//! Implementation of the `operator` module.
//!
//! Only `attrgetter` so far — see `limitations/operator.md`. The comparison and
//! arithmetic helpers (`lt`, `add`, `itemgetter`, `methodcaller`, ...) are
//! absent from the namespace rather than stubbed, so they raise `AttributeError`
//! up front and fail type checking too.

use crate::{
    builtins::Builtins,
    bytecode::VM,
    heap::{HeapData, HeapId},
    intern::StaticStrings,
    types::{Module, Type},
    value::Value,
};

/// Creates the `operator` module on the heap.
///
/// # Panics
/// Panics if the required strings have not been pre-interned during prepare phase.
pub fn create_module(vm: &mut VM<'_>) -> HeapId {
    let mut module = Module::new(StaticStrings::Operator);
    module.set_attr(
        StaticStrings::Attrgetter,
        Value::Builtin(Builtins::Type(Type::AttrGetter)),
        vm,
    );
    vm.heap.allocate(HeapData::Module(Box::new(module)))
}
