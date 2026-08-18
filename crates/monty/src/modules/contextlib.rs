//! Implementation of the `contextlib` module.
//!
//! `suppress` (a real context manager, see [`crate::types::suppress`]) and
//! `AbstractContextManager` (a name to inherit from, no behaviour of its own).
//!
//! `contextmanager`, `ExitStack`, `closing`, `redirect_stdout` and the async
//! halves are absent — see `limitations/contextlib.md`.

use crate::{
    builtins::Builtins,
    bytecode::VM,
    heap::{HeapData, HeapId},
    intern::StaticStrings,
    types::{Module, Type},
    value::{Marker, Value},
};

/// Creates the `contextlib` module on the heap.
///
/// # Panics
/// Panics if the required strings have not been pre-interned during prepare phase.
pub fn create_module(vm: &mut VM<'_>) -> HeapId {
    let mut module = Module::new(StaticStrings::Contextlib);
    module.set_attr(
        StaticStrings::Suppress,
        Value::Builtin(Builtins::Type(Type::Suppress)),
        vm,
    );
    // A marker rather than a type object: nothing constructs it, and what a
    // program does with it is name it as a base class or subscript it.
    module.set_attr(
        StaticStrings::AbstractContextManager,
        Value::Marker(Marker(StaticStrings::AbstractContextManager)),
        vm,
    );
    vm.heap.allocate(HeapData::Module(Box::new(module)))
}
