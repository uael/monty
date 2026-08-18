//! Classes Monty implements natively rather than as a sandbox `class`
//! statement: `typing.Protocol`, `typing.Generic`, and the `collections.abc`
//! family.
//!
//! They are values of [`Type::Native`], not heap [`Class`](super::Class)
//! objects, because their answers come from the interpreter: `isinstance({},
//! Mapping)` is true of a `dict` no sandbox class ever touched. A class
//! statement may still name one as a base — the base contributes no entry to
//! the single-inheritance chain, and is instead remembered on the derived
//! class, which is what makes `isinstance(Foo(), Iterator)` and the default
//! members below work.

use crate::{
    builtins::Builtins,
    bytecode::VM,
    exception_private::{ExcType, ExcTypeExt, RunResult},
    heap::{HeapData, HeapId},
    modules::{
        ModuleFunctions, collections_abc::CollectionsAbcFunctions, contextlib::ContextlibFunctions,
        typing::TypingFunctions,
    },
    types::{
        PyTrait, Type,
        class::{MAX_MRO_DEPTH, class_base_id},
        instance::class_has_member,
    },
    value::Value,
};

/// A class the interpreter provides: a `typing` form or a `collections.abc`
/// abstract class.
///
/// The strum name is the fully qualified one, so `repr` and a type expression
/// (`collections.abc.Mapping[str, int]`) both read as they do in CPython. The
/// cost is that `__name__` is qualified too, the same divergence `deque`
/// documents in ./collections.md.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, strum::IntoStaticStr, strum::EnumIter,
)]
pub enum NativeClass {
    #[strum(serialize = "typing.Protocol")]
    Protocol,
    #[strum(serialize = "typing.Generic")]
    Generic,
    #[strum(serialize = "collections.abc.Hashable")]
    Hashable,
    #[strum(serialize = "collections.abc.Sized")]
    Sized,
    #[strum(serialize = "collections.abc.Container")]
    Container,
    #[strum(serialize = "collections.abc.Iterable")]
    Iterable,
    #[strum(serialize = "collections.abc.Iterator")]
    Iterator,
    #[strum(serialize = "collections.abc.Reversible")]
    Reversible,
    #[strum(serialize = "collections.abc.Collection")]
    Collection,
    #[strum(serialize = "collections.abc.Callable")]
    Callable,
    #[strum(serialize = "collections.abc.Generator")]
    Generator,
    #[strum(serialize = "collections.abc.Sequence")]
    Sequence,
    #[strum(serialize = "collections.abc.MutableSequence")]
    MutableSequence,
    #[strum(serialize = "collections.abc.ByteString")]
    ByteString,
    #[strum(serialize = "collections.abc.Set")]
    Set,
    #[strum(serialize = "collections.abc.MutableSet")]
    MutableSet,
    #[strum(serialize = "collections.abc.Mapping")]
    Mapping,
    #[strum(serialize = "collections.abc.MutableMapping")]
    MutableMapping,
    #[strum(serialize = "collections.abc.MappingView")]
    MappingView,
    #[strum(serialize = "collections.abc.KeysView")]
    KeysView,
    #[strum(serialize = "collections.abc.ItemsView")]
    ItemsView,
    #[strum(serialize = "collections.abc.ValuesView")]
    ValuesView,
    #[strum(serialize = "collections.abc.Awaitable")]
    Awaitable,
    #[strum(serialize = "collections.abc.Coroutine")]
    Coroutine,
    #[strum(serialize = "collections.abc.AsyncIterable")]
    AsyncIterable,
    #[strum(serialize = "collections.abc.AsyncIterator")]
    AsyncIterator,
    #[strum(serialize = "collections.abc.AsyncGenerator")]
    AsyncGenerator,
    #[strum(serialize = "collections.abc.Buffer")]
    Buffer,
    /// `contextlib.AbstractContextManager`, the one native base outside
    /// `typing` and `collections.abc`: CPython defines it in `contextlib` with
    /// the same `__subclasshook__` shape as the abstract classes above.
    #[strum(serialize = "contextlib.AbstractContextManager")]
    AbstractContextManager,
    /// `object`, the root every value is an instance of.
    ///
    /// A native class rather than a [`Type`] of its own because that is what
    /// the rest of this file already means by "a class whose answers the
    /// interpreter computes": every check against it is true, so it needs no
    /// registration table and no base chain. It is the last variant because
    /// the enum is serialized by discriminant.
    #[strum(serialize = "object")]
    Object,
}

/// Every builtin type with a length, which is also every builtin type with a
/// `__contains__`: the two abstract classes select the same set here.
const SIZED: &[Type] = &[
    Type::Str,
    Type::Bytes,
    Type::List,
    Type::Deque,
    Type::Tuple,
    Type::NamedTuple,
    Type::Dict,
    Type::DefaultDict,
    Type::Counter,
    Type::DictKeys,
    Type::DictItems,
    Type::DictValues,
    Type::Set,
    Type::FrozenSet,
    Type::Range,
];

/// Builtin types that iterate backwards. A set has no order to reverse, which
/// is the only difference from [`SIZED`].
const REVERSIBLE: &[Type] = &[
    Type::Str,
    Type::Bytes,
    Type::List,
    Type::Deque,
    Type::Tuple,
    Type::NamedTuple,
    Type::Dict,
    Type::DefaultDict,
    Type::Counter,
    Type::DictKeys,
    Type::DictItems,
    Type::DictValues,
    Type::Range,
];

/// The builtin sequence types, in CPython's registration: `bytearray` and
/// `memoryview` would belong here, and Monty has neither.
const SEQUENCE: &[Type] = &[
    Type::Str,
    Type::Bytes,
    Type::Tuple,
    Type::NamedTuple,
    Type::Range,
    Type::List,
    Type::Deque,
];

/// The builtin types that are not hashable: the mutable containers, plus the
/// two dict views that are sets (and so define `__eq__`). `dict_values` is not
/// a set and stays hashable, as in CPython.
const UNHASHABLE: &[Type] = &[
    Type::List,
    Type::Deque,
    Type::Dict,
    Type::DefaultDict,
    Type::Counter,
    Type::Set,
    Type::DictKeys,
    Type::DictItems,
];

/// The builtin mapping types: `dict` and its two `DictKind` specializations.
const MAPPING: &[Type] = &[Type::Dict, Type::DefaultDict, Type::Counter];

/// Everything callable that is not a sandbox instance, which answers through
/// its class's `__call__` instead.
const CALLABLE: &[Type] = &[
    Type::Function,
    Type::BuiltinFunction,
    Type::Type,
    Type::GenericAlias,
    Type::StaticMethod,
    Type::ClassMethod,
];

impl NativeClass {
    /// The methods a class inheriting from this one gets for free.
    ///
    /// `Iterator.__iter__` returning `self` is the one CPython defines that
    /// Monty can supply today; the hook exists so a natively provided base can
    /// contribute behaviour rather than only an `isinstance` answer.
    #[must_use]
    pub(crate) fn default_member(self, name: &str) -> Option<Value> {
        match (self, name) {
            (Self::Iterator, "__iter__") => Some(Value::ModuleFunction(ModuleFunctions::CollectionsAbc(
                CollectionsAbcFunctions::IteratorIter,
            ))),
            // What makes a PEP 695 generic class subscriptable: the compiler
            // gives one an implicit `Generic` base, exactly as CPython does.
            (Self::Generic | Self::Protocol, "__class_getitem__") => Some(Value::ModuleFunction(
                ModuleFunctions::Typing(TypingFunctions::ClassGetitem),
            )),
            // CPython's `AbstractContextManager` defines exactly these two: an
            // `__enter__` returning `self` and an `__exit__` returning `None`,
            // which is what makes naming it as a base worth anything.
            (Self::AbstractContextManager, "__enter__") => Some(Value::ModuleFunction(ModuleFunctions::Contextlib(
                ContextlibFunctions::ContextManagerEnter,
            ))),
            (Self::AbstractContextManager, "__exit__") => Some(Value::ModuleFunction(ModuleFunctions::Contextlib(
                ContextlibFunctions::ContextManagerExit,
            ))),
            _ => None,
        }
    }

    /// The dunders an object's class must define for it to count as an
    /// instance without declaring the base, i.e. CPython's
    /// `__subclasshook__`. `None` where CPython defines no hook, so only a
    /// declared base (or, there, a registration) can match.
    #[must_use]
    const fn structural_members(self) -> Option<&'static [&'static str]> {
        match self {
            Self::Sized => Some(&["__len__"]),
            Self::Container => Some(&["__contains__"]),
            Self::Iterable => Some(&["__iter__"]),
            Self::Iterator => Some(&["__iter__", "__next__"]),
            Self::Reversible => Some(&["__reversed__", "__iter__"]),
            Self::Collection => Some(&["__len__", "__iter__", "__contains__"]),
            Self::Callable => Some(&["__call__"]),
            Self::Generator => Some(&["__iter__", "__next__", "send", "throw", "close"]),
            Self::Awaitable => Some(&["__await__"]),
            Self::Coroutine => Some(&["__await__", "send", "throw", "close"]),
            Self::AsyncIterable => Some(&["__aiter__"]),
            Self::AsyncIterator => Some(&["__aiter__", "__anext__"]),
            Self::AsyncGenerator => Some(&["__aiter__", "__anext__", "asend", "athrow", "aclose"]),
            Self::Buffer => Some(&["__buffer__"]),
            Self::AbstractContextManager => Some(&["__enter__", "__exit__"]),
            // No member is required of `object`, which is what makes every
            // class satisfy it without naming it as a base.
            Self::Object => Some(&[]),
            // Hashable's answer is computed, not looked up; see `native_isinstance`.
            Self::Hashable
            | Self::Protocol
            | Self::Generic
            | Self::Sequence
            | Self::MutableSequence
            | Self::ByteString
            | Self::Set
            | Self::MutableSet
            | Self::Mapping
            | Self::MutableMapping
            | Self::MappingView
            | Self::KeysView
            | Self::ItemsView
            | Self::ValuesView => None,
        }
    }

    /// CPython's registration table: the builtin types declared instances of
    /// this abstract class. `Iterable`, `Iterator` and `Callable` are absent
    /// because their answer is a property the interpreter computes rather than
    /// a list; see the two callers below.
    #[must_use]
    const fn registered_types(self) -> &'static [Type] {
        match self {
            Self::Sized | Self::Container | Self::Collection => SIZED,
            Self::Reversible => REVERSIBLE,
            Self::Sequence => SEQUENCE,
            Self::MutableSequence => &[Type::List, Type::Deque],
            Self::ByteString => &[Type::Bytes],
            Self::Set => &[Type::Set, Type::FrozenSet, Type::DictKeys, Type::DictItems],
            Self::MutableSet => &[Type::Set],
            Self::Mapping | Self::MutableMapping => MAPPING,
            Self::MappingView => &[Type::DictKeys, Type::DictItems, Type::DictValues],
            Self::KeysView => &[Type::DictKeys],
            Self::ItemsView => &[Type::DictItems],
            Self::ValuesView => &[Type::DictValues],
            Self::Awaitable | Self::Coroutine => &[Type::Coroutine],
            Self::AbstractContextManager => &[Type::Suppress],
            Self::Callable => CALLABLE,
            // Monty has no generator, async iterator or buffer object, so
            // nothing builtin can match; a sandbox class still can, through the
            // structural members above. `Hashable` is computed, and `Protocol`
            // and `Generic` are not instance checks at all.
            Self::Iterable
            | Self::Iterator
            | Self::Hashable
            | Self::Generator
            | Self::AsyncIterable
            | Self::AsyncIterator
            | Self::AsyncGenerator
            | Self::Buffer
            | Self::Protocol
            | Self::Generic
            // `object` matches everything, which the two callers answer before
            // consulting a table.
            | Self::Object => &[],
        }
    }

    /// Whether a builtin *type* is a subclass of this abstract class, for
    /// `issubclass(dict, Mapping)`.
    ///
    /// A bare type cannot be asked whether it iterates, so the two iteration
    /// classes read the same sets the value-level check would arrive at.
    #[must_use]
    pub(crate) fn registers_type(self, ty: Type) -> bool {
        match self {
            Self::Object => true,
            Self::Iterable => SIZED.contains(&ty) || ty.is_iterator(),
            Self::Iterator => ty.is_iterator(),
            Self::Hashable => !UNHASHABLE.contains(&ty),
            _ => self.registered_types().contains(&ty),
        }
    }

    /// Whether a builtin object is an instance of this abstract class.
    ///
    /// Iterability is a property the interpreter already answers for every
    /// value, so it is asked rather than tabulated; the rest come from the
    /// registration table.
    #[must_use]
    fn matches_builtin(self, obj: &Value, ty: Type, vm: &VM<'_>) -> bool {
        match self {
            Self::Object => true,
            Self::Iterable => obj.py_is_iterable(vm),
            Self::Iterator => obj.py_is_iterator(vm),
            _ => self.registered_types().contains(&ty),
        }
    }
}

/// Whether `obj` is an instance of the natively provided class `native`.
///
/// Three routes, in CPython's own order: a declared base on the object's class,
/// then the structural hook, then the builtin registration table.
pub(crate) fn native_isinstance(obj: &Value, native: NativeClass, vm: &mut VM<'_>) -> RunResult<bool> {
    if matches!(native, NativeClass::Protocol | NativeClass::Generic) {
        return Err(ExcType::type_error(
            "Instance and class checks can only be used with @runtime_checkable protocols",
        ));
    }
    // Hashability is not a lookup in Monty: a class defining `__eq__` without
    // `__hash__` is unhashable, which is exactly the answer the abc gives.
    if native == NativeClass::Hashable {
        return Ok(obj.py_hash(vm)?.is_some());
    }
    let ty = obj.py_type(vm);
    if let Type::Instance(class_id) = ty {
        Ok(class_derives_from(class_id, native, vm) || class_has_structural_members(class_id, native, vm))
    } else {
        Ok(native.matches_builtin(obj, ty, vm))
    }
}

/// Whether the sandbox class `class_id` names `native` as a base, anywhere up
/// its chain.
pub(crate) fn class_derives_from(class_id: HeapId, native: NativeClass, vm: &VM<'_>) -> bool {
    let mut current = Some(class_id);
    // The chain is finite (a base must exist to be named); the same bound
    // `class_is_subclass` uses guards a corrupted snapshot.
    for _ in 0..MAX_MRO_DEPTH {
        let Some(id) = current else { return false };
        if let HeapData::Class(class) = vm.heap.get(id)
            && class
                .bases()
                .iter()
                .any(|base| matches!(base, Value::Builtin(Builtins::Type(Type::Native(n))) if *n == native))
        {
            return true;
        }
        current = class_base_id(id, vm);
    }
    false
}

/// Whether `class_id` defines every member `native`'s subclass hook requires.
pub(crate) fn class_has_structural_members(class_id: HeapId, native: NativeClass, vm: &VM<'_>) -> bool {
    native
        .structural_members()
        .is_some_and(|names| names.iter().all(|name| class_has_member(class_id, name, vm)))
}

/// The default member `class_id` inherits from a natively provided base, or
/// `None` when no base up its chain supplies `name`.
pub(crate) fn native_default_member(class_id: HeapId, name: &str, vm: &VM<'_>) -> Option<Value> {
    let mut current = Some(class_id);
    for _ in 0..MAX_MRO_DEPTH {
        let id = current?;
        if let HeapData::Class(class) = vm.heap.get(id) {
            for base in class.bases() {
                if let Value::Builtin(Builtins::Type(Type::Native(native))) = base
                    && let Some(member) = native.default_member(name)
                {
                    return Some(member);
                }
            }
        }
        current = class_base_id(id, vm);
    }
    None
}
