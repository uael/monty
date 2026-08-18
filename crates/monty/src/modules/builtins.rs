//! Implementation of the `builtins` module.
//!
//! The namespace a bare name resolves against, made reachable as an object so
//! `vars(builtins)` can enumerate it — which is what a program does to map a
//! value back to the name it is known by.
//!
//! It is assembled from the same three sources name resolution uses
//! ([`BuiltinsFunctions`], [`ExcType`], [`Type::from_builtin_name`]) rather
//! than from a written-out list, so a builtin added to any of them appears here
//! without a second edit. Each is enumerated exhaustively, so nothing can be
//! silently dropped.
//!
//! Attribute names are heap strings, not interned ids: the intern table is
//! frozen after prepare and most builtin names never appear in a program's
//! source, so there is nothing to intern them from.

use monty_types::ExcType;
use strum::IntoEnumIterator;

use crate::{
    builtins::{Builtins, BuiltinsFunctions},
    bytecode::VM,
    heap::{HeapData, HeapId},
    intern::StaticStrings,
    types::{Module, Type, str::allocate_string},
    value::Value,
};

/// The builtin type constructors, the inverse of [`Type::from_builtin_name`].
///
/// Written out because that function is a `&str` match with no iterator; keep
/// the two in step.
const BUILTIN_TYPES: &[Type] = &[
    Type::Bool,
    Type::Int,
    Type::Float,
    Type::Str,
    Type::Bytes,
    Type::List,
    Type::Tuple,
    Type::Dict,
    Type::Set,
    Type::FrozenSet,
    Type::Range,
    Type::Slice,
    Type::Iterator,
    Type::Type,
    Type::Property,
];

/// Creates the `builtins` module on the heap.
///
/// # Panics
/// Panics if the required strings have not been pre-interned during prepare
/// phase, or if a builtin type is missing the source-level name it is listed
/// under here.
pub fn create_module(vm: &mut VM<'_>) -> HeapId {
    let mut module = Module::new(StaticStrings::Builtins);

    for function in BuiltinsFunctions::iter() {
        let name: &'static str = function.into();
        set(&mut module, name, Value::Builtin(Builtins::Function(function)), vm);
    }
    for exc_type in ExcType::iter().filter(|exc| exc.is_builtin()) {
        let name: &'static str = exc_type.into();
        set(&mut module, name, Value::Builtin(Builtins::ExcType(exc_type)), vm);
    }
    for ty in BUILTIN_TYPES {
        let name = ty
            .builtin_name()
            .expect("BUILTIN_TYPES lists only types with a source-level name");
        set(&mut module, name, Value::Builtin(Builtins::Type(*ty)), vm);
    }
    // The names that resolve to a singleton rather than a callable. Built
    // inline because `Value` is deliberately not `Copy`.
    set(&mut module, "None", Value::None, vm);
    set(&mut module, "True", Value::Bool(true), vm);
    set(&mut module, "False", Value::Bool(false), vm);
    set(&mut module, "Ellipsis", Value::Ellipsis, vm);
    set(&mut module, "NotImplemented", Value::NotImplemented, vm);

    vm.heap.allocate(HeapData::Module(Box::new(module)))
}

/// Binds one name, allocating the key as a heap string.
fn set(module: &mut Module, name: &str, value: Value, vm: &mut VM<'_>) {
    let key = allocate_string(name.to_owned(), vm.heap);
    module.set_attr_value(key, value, vm);
}
