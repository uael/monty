//! Implementation of the `collections.abc` module.
//!
//! Every name is a [`NativeClass`] value: the abstract classes answer
//! `isinstance`/`issubclass` from the interpreter's own knowledge of a type
//! (a `dict` is a `Mapping` whether or not any sandbox code says so), and a
//! class statement may name one as a base to inherit both the answer and the
//! members below.

use std::fmt;

use crate::{
    args::ArgValues,
    builtins::Builtins,
    bytecode::VM,
    exception_private::RunResult,
    heap::{HeapData, HeapId},
    intern::StaticStrings,
    types::{Module, NativeClass, Type},
    value::Value,
};

/// The methods `collections.abc` classes hand to their subclasses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(crate) enum CollectionsAbcFunctions {
    /// `Iterator.__iter__`, which returns the iterator itself so a class
    /// defining only `__next__` still works in a `for`.
    IteratorIter,
}

impl fmt::Display for CollectionsAbcFunctions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::IteratorIter => "__iter__",
        })
    }
}

/// Creates the `collections.abc` module and allocates it on the heap.
///
/// # Panics
///
/// Panics if the required strings have not been pre-interned during prepare phase.
pub fn create_module(vm: &mut VM<'_>) -> HeapId {
    let mut module = Module::new(StaticStrings::CollectionsAbc);
    for (name, class) in ABC_ATTRS {
        module.set_attr(*name, Value::Builtin(Builtins::Type(Type::Native(*class))), vm);
    }
    vm.heap.allocate(HeapData::Module(Box::new(module)))
}

/// Dispatches a `collections.abc` default method.
pub(super) fn call(vm: &mut VM<'_>, func: CollectionsAbcFunctions, args: ArgValues) -> RunResult<Value> {
    match func {
        // Bound like any other method, so the receiver arrives as the only
        // argument and is what `iter()` must hand back.
        CollectionsAbcFunctions::IteratorIter => args.get_one_arg("__iter__", vm.heap),
    }
}

/// The whole of `collections.abc.__all__`, each bound to its native class.
const ABC_ATTRS: &[(StaticStrings, NativeClass)] = &[
    (StaticStrings::Hashable, NativeClass::Hashable),
    (StaticStrings::Sized, NativeClass::Sized),
    (StaticStrings::Container, NativeClass::Container),
    (StaticStrings::Iterable, NativeClass::Iterable),
    (StaticStrings::IteratorType, NativeClass::Iterator),
    (StaticStrings::Reversible, NativeClass::Reversible),
    (StaticStrings::Collection, NativeClass::Collection),
    (StaticStrings::Callable, NativeClass::Callable),
    (StaticStrings::Generator, NativeClass::Generator),
    (StaticStrings::Sequence, NativeClass::Sequence),
    (StaticStrings::MutableSequence, NativeClass::MutableSequence),
    (StaticStrings::ByteString, NativeClass::ByteString),
    (StaticStrings::SetType, NativeClass::Set),
    (StaticStrings::MutableSet, NativeClass::MutableSet),
    (StaticStrings::Mapping, NativeClass::Mapping),
    (StaticStrings::MutableMapping, NativeClass::MutableMapping),
    (StaticStrings::MappingView, NativeClass::MappingView),
    (StaticStrings::KeysView, NativeClass::KeysView),
    (StaticStrings::ItemsView, NativeClass::ItemsView),
    (StaticStrings::ValuesView, NativeClass::ValuesView),
    (StaticStrings::Awaitable, NativeClass::Awaitable),
    (StaticStrings::CoroutineClass, NativeClass::Coroutine),
    (StaticStrings::AsyncIterable, NativeClass::AsyncIterable),
    (StaticStrings::AsyncIterator, NativeClass::AsyncIterator),
    (StaticStrings::AsyncGenerator, NativeClass::AsyncGenerator),
    (StaticStrings::Buffer, NativeClass::Buffer),
];
