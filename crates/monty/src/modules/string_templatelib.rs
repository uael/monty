//! Implementation of the `string.templatelib` module (PEP 750).
//!
//! The module is a namespace only: it exposes the `Template` and `Interpolation`
//! type objects so `isinstance(t, Template)` and annotations resolve, and has no
//! functions of its own. Neither type is constructible from Python; templates
//! are produced by `t"..."` literals. See `limitations/string_templatelib.md`.

use crate::{
    builtins::Builtins,
    bytecode::VM,
    heap::{HeapData, HeapId},
    intern::StaticStrings,
    types::{Module, Type},
    value::Value,
};

/// Allocates the `string.templatelib` module and returns its `HeapId`.
///
/// # Panics
/// If the required strings were not pre-interned during the prepare phase.
pub fn create_module(vm: &mut VM<'_>) -> HeapId {
    let mut module = Module::new(StaticStrings::StringTemplatelib);

    module.set_attr(
        StaticStrings::TemplateClass,
        Value::Builtin(Builtins::Type(Type::Template)),
        vm,
    );
    module.set_attr(
        StaticStrings::InterpolationClass,
        Value::Builtin(Builtins::Type(Type::Interpolation)),
        vm,
    );

    vm.heap.allocate(HeapData::Module(Box::new(module)))
}
