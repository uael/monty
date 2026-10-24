//! Implementation of the `string.templatelib` module (PEP 750).
//!
//! The module is a namespace only: it exposes the `Template` and `Interpolation`
//! type objects, which `isinstance(t, Template)` and annotations resolve, and
//! which build a template as CPython does; it has no functions of its own. See
//! `limitations/string_templatelib.md`. The `string` package that holds it
//! binds nothing else.

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
    let mut module = Module::new(StaticStrings::StringTemplatelib, vm.interns);

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

/// Allocates the `string` package and returns its `HeapId`.
///
/// It binds `templatelib` alone, which is what `import string.templatelib`
/// and `from string.templatelib import ...` walk through; the rest of
/// CPython's `string` module is not there. See `limitations/string_templatelib.md`.
pub fn create_package(vm: &mut VM<'_>) -> HeapId {
    let mut package = Module::new(StaticStrings::StringAttr, vm.interns);
    let templatelib = create_module(vm);
    package.set_attr(StaticStrings::Templatelib, Value::Ref(templatelib), vm);
    vm.heap.allocate(HeapData::Module(Box::new(package)))
}
