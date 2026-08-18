//! Implementation of the `types` module.
//!
//! Only the type objects Monty can name exactly: each attribute here *is* the
//! runtime type a value of that shape reports, so `isinstance(None,
//! types.NoneType)` and `type(int | str) is types.UnionType` both hold. Shapes
//! Monty conflates (every callable reports `function`, so `FunctionType` and
//! `MethodType` could not be told apart) are deliberately absent rather than
//! wrong; see `limitations/typing.md`.

use crate::{
    builtins::Builtins,
    bytecode::VM,
    heap::{HeapData, HeapId},
    intern::StaticStrings,
    types::{Module, Type},
    value::Value,
};

/// Creates the `types` module and allocates it on the heap.
///
/// # Panics
///
/// Panics if the required strings have not been pre-interned during prepare phase.
pub fn create_module(vm: &mut VM<'_>) -> HeapId {
    let mut module = Module::new(StaticStrings::TypesModule);
    for (name, ty) in TYPE_ATTRS {
        module.set_attr(*name, Value::Builtin(Builtins::Type(*ty)), vm);
    }
    vm.heap.allocate(HeapData::Module(Box::new(module)))
}

/// The `types` attributes, each bound to the runtime type it names.
///
/// `UnionType` is `typing.Union`: CPython 3.14 merged the two, so both module
/// attributes hold one object and `get_origin(int | str) is UnionType`.
const TYPE_ATTRS: &[(StaticStrings, Type)] = &[
    (StaticStrings::UnionTypeClass, Type::Union),
    (StaticStrings::GenericAliasClass, Type::GenericAlias),
    (StaticStrings::NoneTypeClass, Type::NoneType),
    (StaticStrings::EllipsisTypeClass, Type::Ellipsis),
    (StaticStrings::NotImplementedTypeClass, Type::NotImplementedType),
    (StaticStrings::ModuleTypeClass, Type::Module),
    (StaticStrings::CellTypeClass, Type::Cell),
];
