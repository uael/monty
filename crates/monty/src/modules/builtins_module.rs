//! The `builtins` module.
//!
//! Every name a bare identifier resolves to without an import, reachable as an
//! attribute. A program that looks a builtin up by its name, rather than
//! writing it out, needs this: `getattr(builtins, name)` and
//! `vars(builtins)[name]` are how that is written in Python.

use monty_types::ExcType;
use strum::VariantNames;

use crate::{
    builtins::{Builtins, BuiltinsFunctions},
    bytecode::VM,
    heap::HeapId,
    intern::StaticStrings,
    types::{Module, Type},
    value::Value,
};

/// Builds the `builtins` module.
///
/// The names come from the three enums a bare name resolves through, and each
/// one is set through [`Builtins::from_str`], so the module cannot hold a name
/// that does not resolve or miss one that does.
pub fn create_module(vm: &mut VM<'_>) -> HeapId {
    let mut module = Module::new(StaticStrings::Builtins, vm.interns);
    let named = BuiltinsFunctions::VARIANTS
        .iter()
        .chain(ExcType::VARIANTS)
        .chain(Type::BUILTIN_NAMES);
    for name in named {
        if let Ok(one) = name.parse::<Builtins>() {
            module.set_named(name, Value::Builtin(one), vm);
        }
    }
    // The three constants are keywords, so no bare name resolves through
    // `Builtins`, but CPython carries them here and a lookup by name finds them.
    module.set_named("None", Value::None, vm);
    module.set_named("True", Value::Bool(true), vm);
    module.set_named("False", Value::Bool(false), vm);
    vm.heap.allocate_as(module).into_id()
}
