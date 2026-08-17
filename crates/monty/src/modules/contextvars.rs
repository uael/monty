//! Implementation of the `contextvars` module.
//!
//! Just `ContextVar`, exposed as the type object itself (like `collections.deque`)
//! rather than a factory function, so `type(v)` and `repr` name a class.
//!
//! `Token` is reachable only as the value `ContextVar.set()` returns, never as a
//! module attribute: CPython refuses to construct one anyway
//! (`cannot create '_contextvars.Token' instances`), so a name that could only
//! ever raise would buy nothing. `Context`, `copy_context` and `ContextVar`'s
//! per-context storage are absent — see `limitations/contextvars.md`.

use crate::{
    builtins::Builtins,
    bytecode::VM,
    heap::{HeapData, HeapId},
    intern::StaticStrings,
    types::{Module, Type},
    value::Value,
};

/// Creates the `contextvars` module on the heap.
///
/// # Panics
/// Panics if the required strings have not been pre-interned during prepare phase.
pub fn create_module(vm: &mut VM<'_>) -> HeapId {
    let mut module = Module::new(StaticStrings::Contextvars);
    module.set_attr(
        StaticStrings::ContextVarClass,
        Value::Builtin(Builtins::Type(Type::ContextVar)),
        vm,
    );
    vm.heap.allocate(HeapData::Module(Box::new(module)))
}
