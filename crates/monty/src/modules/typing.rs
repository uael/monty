//! Implementation of the `typing` module.
//!
//! Most names here are inert `Marker` values: they exist so annotated code can
//! import them, and nothing reads them at runtime. The exceptions are the forms
//! a *runtime* type expression is built from — `Union`, and the functions that
//! take one apart — which are real objects with real behaviour, because
//! `int | str` and `get_origin(...)` have answers.

use std::fmt;

use smallvec::SmallVec;

use crate::{
    args::ArgValues,
    builtins::Builtins,
    bytecode::VM,
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunResult},
    heap::{DropWithContext, HeapData, HeapId},
    intern::StaticStrings,
    modules::ModuleFunctions,
    types::{
        Module, NativeClass, Type, allocate_tuple,
        generic_alias::{origin_and_args, subscript_type_form},
        protocol::runtime_checkable,
    },
    value::{Marker, Value},
};

/// The `typing` functions Monty implements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(crate) enum TypingFunctions {
    /// `typing.get_origin(tp)`.
    GetOrigin,
    /// `typing.get_args(tp)`.
    GetArgs,
    /// `typing.overload(func)`.
    Overload,
    /// The function `overload` returns in place of the decorated stub. Calling
    /// it is the mistake CPython's `_overload_dummy` exists to report.
    OverloadDummy,
    /// `typing.dataclass_transform(**kwargs)`.
    DataclassTransform,
    /// The decorator `dataclass_transform` returns: it hands back whatever it
    /// is applied to, the whole point being that only a type checker reads it.
    Identity,
    /// `typing.runtime_checkable(cls)`.
    RuntimeCheckable,
    /// `typing.Generic.__class_getitem__`, which every PEP 695 generic class
    /// inherits: it is what makes `Spawned[T]` an alias rather than an error.
    ClassGetitem,
}

impl fmt::Display for TypingFunctions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::GetOrigin => "get_origin",
            Self::GetArgs => "get_args",
            Self::Overload => "overload",
            Self::OverloadDummy => "_overload_dummy",
            Self::DataclassTransform => "dataclass_transform",
            Self::Identity => "decorator",
            Self::RuntimeCheckable => "runtime_checkable",
            Self::ClassGetitem => "__class_getitem__",
        })
    }
}

/// Creates the `typing` module and allocates it on the heap.
///
/// # Panics
///
/// Panics if the required strings have not been pre-interned during prepare phase.
pub fn create_module(vm: &mut VM<'_>) -> HeapId {
    let mut module = Module::new(StaticStrings::Typing);

    // typing.TYPE_CHECKING - always False
    module.set_attr(StaticStrings::TypeChecking, Value::Bool(false), vm);

    for ss in MARKER_ATTRS {
        module.set_attr(*ss, Value::Marker(Marker(*ss)), vm);
    }
    for (name, func) in FUNCTION_ATTRS {
        module.set_attr(*name, Value::ModuleFunction(ModuleFunctions::Typing(*func)), vm);
    }
    // `Union` is a real type object, not a marker: it is what `type(int | str)`
    // reports and what `types.UnionType` names. `Protocol` and `Generic` are
    // real classes too, so a class statement can name one as a base.
    for (name, ty) in [
        (StaticStrings::UnionType, Type::Union),
        (StaticStrings::Protocol, Type::Native(NativeClass::Protocol)),
        (StaticStrings::Generic, Type::Native(NativeClass::Generic)),
    ] {
        module.set_attr(name, Value::Builtin(Builtins::Type(ty)), vm);
    }

    vm.heap.allocate(HeapData::Module(Box::new(module)))
}

/// Typing marker attributes exported by this module.
///
/// Each marker wraps its corresponding `StaticStrings` variant as both the
/// attribute name and the marker value.
const MARKER_ATTRS: &[StaticStrings] = &[
    StaticStrings::Any,
    StaticStrings::Optional,
    StaticStrings::ListType,
    StaticStrings::DictType,
    StaticStrings::TupleType,
    StaticStrings::SetType,
    StaticStrings::FrozenSet,
    StaticStrings::Callable,
    StaticStrings::Type,
    StaticStrings::Sequence,
    StaticStrings::Mapping,
    StaticStrings::Iterable,
    StaticStrings::IteratorType,
    StaticStrings::Generator,
    StaticStrings::ClassVar,
    StaticStrings::FinalType,
    StaticStrings::Literal,
    StaticStrings::TypeVar,
    StaticStrings::Annotated,
    StaticStrings::SelfType,
    StaticStrings::Never,
    StaticStrings::NoReturn,
];

/// Callable attributes exported by this module.
const FUNCTION_ATTRS: &[(StaticStrings, TypingFunctions)] = &[
    (StaticStrings::GetOrigin, TypingFunctions::GetOrigin),
    (StaticStrings::GetArgs, TypingFunctions::GetArgs),
    (StaticStrings::Overload, TypingFunctions::Overload),
    (StaticStrings::DataclassTransform, TypingFunctions::DataclassTransform),
    (StaticStrings::RuntimeCheckable, TypingFunctions::RuntimeCheckable),
];

/// Dispatches a `typing` module function call.
pub(super) fn call(vm: &mut VM<'_>, func: TypingFunctions, args: ArgValues) -> RunResult<Value> {
    match func {
        TypingFunctions::GetOrigin => {
            let tp = args.get_one_arg("get_origin", vm.heap)?;
            defer_drop!(tp, vm);
            Ok(match origin_and_args(tp, vm) {
                Some((origin, alias_args)) => {
                    alias_args.drop_with(vm);
                    origin
                }
                None => Value::None,
            })
        }
        TypingFunctions::GetArgs => {
            let tp = args.get_one_arg("get_args", vm.heap)?;
            defer_drop!(tp, vm);
            Ok(match origin_and_args(tp, vm) {
                Some((origin, alias_args)) => {
                    origin.drop_with(vm);
                    alias_args
                }
                None => allocate_tuple(SmallVec::new(), vm.heap),
            })
        }
        // The decorated stub is discarded: a later plain `def` of the same name
        // is the implementation, and CPython's rule is that it wins.
        TypingFunctions::Overload => {
            args.get_one_arg("overload", vm.heap)?.drop_with(vm);
            Ok(Value::ModuleFunction(ModuleFunctions::Typing(
                TypingFunctions::OverloadDummy,
            )))
        }
        TypingFunctions::OverloadDummy => {
            args.drop_with(vm);
            Err(ExcType::not_implemented(
                "You should not call an overloaded function. A series of @overload-decorated functions outside a stub module should always be followed by an implementation that is not @overload-ed.",
            )
            .into())
        }
        // Every keyword is a type-checker instruction, so they are accepted and
        // dropped rather than validated.
        TypingFunctions::DataclassTransform => {
            args.drop_with(vm);
            Ok(Value::ModuleFunction(ModuleFunctions::Typing(
                TypingFunctions::Identity,
            )))
        }
        TypingFunctions::Identity => args.get_one_arg("decorator", vm.heap),
        TypingFunctions::RuntimeCheckable => {
            let cls = args.get_one_arg("runtime_checkable", vm.heap)?;
            runtime_checkable(cls, vm)
        }
        // Reached as an implicit classmethod, so the class arrives first.
        TypingFunctions::ClassGetitem => {
            let (cls, key) = args.get_two_args("__class_getitem__", vm.heap)?;
            defer_drop!(key, vm);
            Ok(subscript_type_form(cls, key, vm))
        }
    }
}
