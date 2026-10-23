//! Implementation of the `contextvars` module.
//!
//! The module exports the two type objects and nothing else: Monty runs one
//! implicit context, so `Context` and `copy_context()` would have nothing to
//! copy. See `limitations/contextvars.md` and [`crate::types::contextvar`].

use crate::{
    builtins::Builtins,
    bytecode::VM,
    heap::HeapId,
    intern::StaticStrings,
    types::{Module, Type},
    value::Value,
};

/// Creates the `contextvars` module and allocates it on the heap.
pub fn create_module(vm: &mut VM<'_>) -> HeapId {
    let mut module = Module::new(StaticStrings::Contextvars, vm.interns);
    module.set_attr(
        StaticStrings::ContextVar,
        Value::Builtin(Builtins::Type(Type::ContextVar)),
        vm,
    );
    module.set_attr(
        StaticStrings::Token,
        Value::Builtin(Builtins::Type(Type::ContextVarToken)),
        vm,
    );
    vm.heap.allocate_as(module).into_id()
}
