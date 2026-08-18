# `typing` module and runtime type forms

**No runtime type checking happens.** What does exist is the object an
annotation *evaluates to*: `list[int]` builds a real `types.GenericAlias`,
`int | str` a real `typing.Union`, and `typing.get_origin`/`get_args` take
either apart. Everything else in `typing` is an inert marker that exists so
annotated code can `import` it.

## Runtime type forms

`types.GenericAlias` — subscripting a class builds one, and it behaves as in
CPython: `__origin__`, `__args__`, `repr`, equality, hashing, calling through
to the origin (`list[int]() == []`) and attribute fall-through to the origin
(`list[int].__name__ == 'list'`). Only the classes CPython parameterizes are
subscriptable: `list`, `dict`, `tuple`, `set`, `frozenset`, `type`,
`enumerate`, `staticmethod`, `classmethod`, `collections.deque`,
`collections.defaultdict`, `collections.Counter`, `re.Pattern`, `re.Match`,
plus every `collections.abc` class. Anything else raises `TypeError: type 'X'
is not subscriptable`, as CPython does.

A class defines `__class_getitem__` to be subscriptable, and Monty treats it
as the implicit classmethod CPython does: a plain function in the body
receives the class before the subscript.

`typing.Union` — `int | str`. As in CPython 3.14, `types.UnionType` *is*
`typing.Union`, so `type(int | str) is types.UnionType` holds. Members are
flattened, deduplicated and order-preserving, `None` normalizes to `NoneType`,
and a one-member union collapses to the member (`int | int is int`).
`isinstance(x, int | str)` works, equality and hashing ignore member order,
and `typing.Union[...]` / `typing.Optional[X]` spell out the same objects.

`typing.get_origin` / `typing.get_args` read both forms and return `None` /
`()` for anything else, matching CPython.

Divergences:

- A **sandbox-defined class inside a type form prints its bare name**
  (`list[Foo]`), where CPython qualifies it with its module
  (`list[__main__.Foo]`). Monty gives sandbox classes no `__module__`. The
  same applies to a sandbox function used as an argument, which prints its
  `repr` rather than a qualified name; a *builtin* function prints bare
  (`list[len]`) exactly as CPython does.
- `__mro_entries__` is applied by class creation but not exposed as a method,
  so `list[int].__mro_entries__(())` raises `AttributeError` (CPython returns
  `(list,)`).
- `list[int][str]` (substituting into an alias) is not supported.
- `types.GenericAlias(list, (int,))` — the constructor — is not supported;
  build one by subscripting.
- `type(typing.Union)` is `type`, where CPython reports its metaclass.

## Names defined

`Any`, `Optional`, `Union`, `List`, `Dict`, `Tuple`, `Set`, `FrozenSet`,
`Callable`, `Type`, `Sequence`, `Mapping`, `Iterable`, `Iterator`,
`Generator`, `ClassVar`, `Final`, `Literal`, `TypeVar`, `Generic`,
`Protocol`, `Annotated`, `Self`, `Never`, `NoReturn`, `TYPE_CHECKING`,
`get_origin`, `get_args`, `overload`, `dataclass_transform`,
`runtime_checkable`.

`TYPE_CHECKING` is `False`, as in CPython at runtime.

`overload` returns the same refusing stub CPython's `_overload_dummy` is, so a
series of `@overload` definitions followed by a plain one leaves the plain one
bound and calling an unimplemented stub raises. `dataclass_transform` accepts
and ignores every keyword, returning an identity decorator: only a type checker
reads it.

`Literal`, `ClassVar`, `Final` and `Annotated` are subscriptable, and answer
`get_origin` with the form itself and `get_args` with the subscript, exactly as
CPython does. Every remaining name in the first list is an inert marker:
**subscripting one raises `TypeError`**. In particular the deprecated aliases
`typing.List[int]`, `typing.Dict[str, int]` and `typing.Callable[[], bool]` do
not work — CPython gives those the *class* as their origin while still printing
the `typing.` name, and one alias object cannot do both — so use the builtin
generics and `collections.abc` instead.

## `collections.abc` and `typing.Protocol`

The whole of `collections.abc.__all__` is implemented — `Hashable`, `Sized`,
`Container`, `Iterable`, `Iterator`, `Reversible`, `Collection`, `Callable`,
`Generator`, `Sequence`, `MutableSequence`, `ByteString`, `Set`, `MutableSet`,
`Mapping`, `MutableMapping`, `MappingView`, `KeysView`, `ItemsView`,
`ValuesView`, `Awaitable`, `Coroutine`, `AsyncIterable`, `AsyncIterator`,
`AsyncGenerator` and `Buffer` — alongside `typing.Protocol` and
`typing.Generic`. All of them are classes the interpreter provides rather than
sandbox `class` statements, and all behave the same way:

- **`isinstance` / `issubclass` give CPython's answers.** A builtin object
  matches through the same registration table CPython uses (`isinstance({},
  Mapping)`, `issubclass(str, Sequence)`), and a sandbox class matches
  structurally where CPython defines a `__subclasshook__` (`Iterable` wants
  `__iter__`, `Collection` wants `__len__`/`__iter__`/`__contains__`, and so
  on). `Sequence`, `Mapping` and `Set` have no hook in CPython either, so only
  a declared base counts for them.
- **Subscripting one builds a `types.GenericAlias`**: `Mapping[str, int]`,
  `Callable[[int], str]`.
- **One may stand in a base list.** It adds no link to the single-inheritance
  chain — the derived class still has at most one concrete base — but it is
  remembered, so `isinstance(Countdown(), Iterator)` is true and the base's
  default members arrive: `Iterator.__iter__` returns `self`, so a class
  defining only `__next__` iterates. A subscripted one (`class C(Iterator[int])`)
  resolves through `__mro_entries__` to the class it subscripted first.

`typing.Protocol` additionally behaves as it does in CPython: a class naming it
directly gets `_is_protocol = True` and cannot be instantiated (`TypeError:
Protocols cannot be instantiated`), a concrete subclass gets
`_is_protocol = False` and can be; `isinstance`/`issubclass` against a protocol
that is not `@runtime_checkable` raise `TypeError: Instance and class checks can
only be used with @runtime_checkable protocols`, even for a class that really
does derive from it; and `@runtime_checkable` records `__protocol_attrs__` (a
`frozenset`) and `_is_runtime_protocol = True`, after which `isinstance` and
`issubclass` test for those members structurally. The recorded members are the
names the protocol and its bases *bind* plus the names they *annotate*, which
is what makes a data protocol (`class HasContent(Protocol): content: str`)
mean anything: it binds nothing at all, and an empty member set is satisfied by
every object.

Divergences:

- **`type(Mapping)` is `type`**, where CPython reports `abc.ABCMeta`; likewise
  `type(Protocol)` rather than `typing._ProtocolMeta`.
- **`register()` is not implemented**: the registration table is fixed, so a
  sandbox class cannot declare itself a `Sequence` after the fact — name the
  base instead.
- **`Generator`, `AsyncIterable`, `AsyncIterator`, `AsyncGenerator` and
  `Buffer` match no builtin object**, since Monty has no generator, async
  iterator or buffer of its own; a sandbox class still matches them
  structurally.
- **Abstractness is not enforced**: a class naming an abstract base is
  instantiable even when it implements none of the abstract methods, where
  CPython raises. Only `Protocol` refuses instantiation.
- **`Generic` is not a distinct mechanism**: `class C(Generic[T])` is accepted
  and erased, exactly as PEP 695's `class C[T]` is.

## The `types` module

`types` exports the runtime type objects Monty can name exactly: `UnionType`,
`GenericAlias`, `NoneType`, `EllipsisType`, `NotImplementedType`, `ModuleType`
and `CellType`. Each *is* the type a value of that shape reports, so
`isinstance(None, types.NoneType)` holds.

`FunctionType`, `MethodType`, `BuiltinFunctionType`, `CoroutineType`,
`GeneratorType`, `SimpleNamespace`, `MappingProxyType`, `CodeType`,
`TracebackType` and `FrameType` are **absent** rather than wrong: Monty reports
one `function` type for plain functions, closures and bound methods alike, and
one `coroutine` type for coroutines and futures, so those names could not tell
apart what CPython tells apart.

## Not implemented

- `get_type_hints`, `cast`, `assert_type`,
  `assert_never`, `final`, `NewType`, `TypeVar` as a callable,
  `NamedTuple`, `TypedDict`, `ParamSpec`,
  `Concatenate`, `Unpack`, `TypeAlias`, `LiteralString`.
- `typing.TypeAliasType` is **not** exported by this module even though the
  type exists: a PEP 695 `type X = ...` statement builds one (see below), but
  `from typing import TypeAliasType` raises `ImportError`, so the type object
  cannot be named and `isinstance(X, TypeAliasType)` has no equivalent.
- Annotation introspection on **functions and modules**: `__annotations__` is
  not populated there. Class `__annotations__` **is** populated; see below.

## PEP 695 type parameters

**A class's type parameters bind.** `class C[T]` builds a real
`typing.TypeVar` per parameter when the class statement runs — one object, so
`T is T` inside the class — readable by the bases and by the class body, and
invisible outside them. That is what makes `class Held[T](Spawned[T])` work:
`Spawned[T]` is an ordinary subscript, and a generic class is subscriptable
because it gets the implicit `typing.Generic` base CPython gives it.

`repr(T)` is `T` and `T.__name__` is `'T'`, matching CPython, whose PEP 695
parameters infer their variance and so print without the `~` an explicitly
variant `TypeVar` carries.

The rest still bind **nothing**:

- **A function's and an alias's parameters are dropped.** `def f[T](x): return
  T` raises `NameError` where CPython returns the `TypeVar`, and so does
  `type X[T] = ...`. A class body runs exactly once, so a `TypeVar` created
  there has the identity CPython gives it; a function body runs many times, and
  binding one per call would not.
- **A same-named outer binding shadows through instead**, for those two
  constructs: if a module-level `T` exists, `def f[T](x): return T` returns
  *that* value in Monty and the `TypeVar` in CPython. This is the one case that
  gives a wrong answer rather than an error. A *class* is not affected — its
  parameter shadows the outer binding, as CPython's does.
- **`*Ts` and `**P` are dropped even on a class**: they would need a
  `TypeVarTuple` or a `ParamSpec`, which Monty has neither of, so reading one
  raises `NameError`.
- Bounds and defaults (`class C[T: int, U = str]`) are parsed and discarded;
  the expressions are never evaluated, so an error in one is never raised.
- `__type_params__` is not exposed on functions, classes, or aliases; reading
  it raises `AttributeError` where CPython returns a (possibly empty) tuple.
- A `TypeVar` is not constructible: `typing.TypeVar` is an inert marker, and
  only a `class C[T]` statement makes one.
- A method of a generic class cannot read the parameter. Monty's class-body
  locals are never cells, so a nested scope inside the body captures nothing
  from it; CPython resolves `T` there through the implicit type-parameter
  scope.
- A generic class evaluates its **bases inside the class body** rather than in
  the enclosing scope, since that is where its parameters live. Order is
  unchanged (parameters, then bases, then the body), but a base named the same
  as one of the class's own members now resolves to the member — an
  `UnboundLocalError`, where CPython reads the enclosing binding. A
  non-generic class is unaffected.

None of this affects annotations, which are stringized and never evaluated.

## PEP 695 type aliases (`type X = ...`)

A `type` statement binds a real `TypeAliasType` object, at module scope, in a
function, and in a class body. `X.__name__`, `X.__value__`, `repr(X)` and
`str(X)` all match CPython, and `__value__` is evaluated lazily on first read
and then memoized, so an alias may mention itself
(`type Wire = str | list[Wire]` is only an error if `__value__` is read and the
operators involved are unsupported). Divergences:

- `X.__type_params__` and `X.__module__` raise `AttributeError`.
- Every attribute is read-only: `X.__name__ = ...` raises `AttributeError:
  'typing.TypeAliasType' object has no attribute '__name__' and no __dict__ for
  setting new attributes`, where CPython says `readonly attribute`.
- `X[int]` (subscripting an alias) is not supported.
- The type object itself is not reachable from Python; see above.

## Class annotations are stringized

A class body's annotations **are** recorded, in order, on the class's
`__annotations__` dict, but in **stringized** form, unconditionally. The values
are the annotation expression rendered back to source, never evaluated. As in
CPython's PEP 563 stringizer the expression is *unparsed* rather than sliced out
of the file, so original spacing, line breaks and quote style are normalized
away (`x: dict[str,int]` gives `'dict[str, int]'`):

```python
class C:
    x: int
    y: list[int]
C.__annotations__        # {'x': 'int', 'y': 'list[int]'}  -- strings
```

This is a known temporary divergence; see `class__annotations.py`.

- **Divergence from CPython 3.14's default** (PEP 649), where these are the
  evaluated objects (`C.__annotations__['x'] is int`). CPython only agrees with
  Monty when the calling code uses `from __future__ import annotations`
  (PEP 563), which Monty's behaviour is otherwise equivalent to, except that
  Monty stringizes whether or not that import is present.
- The prerequisite for matching PEP 649 — runtime `types.GenericAlias` and `|`
  unions, so that `list[int]` and `int | None` have values — now exists; the
  migration itself does not. Annotations are still stringized and never
  evaluated.
- **Treat the values as provisional.** Code reading `__annotations__` sees
  strings today and would see type objects after a PEP 649 migration; the
  *keys* and their order are stable either way.
- Only **simple `name: T` targets** are recorded, as in CPython. A bare
  `obj.attr: T` contributes nothing to `__annotations__` on either, but CPython
  still *evaluates the target expression*: `undefined.attr: int` raises
  `NameError` there and is silently dropped by Monty. With a value
  (`obj.attr: T = v`) Monty raises `NotImplementedError`.
- Binding **`__annotations__` explicitly** in a class body that *also* has
  annotated names raises `NotImplementedError`. CPython instead stores the
  collected annotations into whatever the name holds, merging into an explicit
  `dict`, or raising `TypeError` if it holds something else. A class body that
  binds the name but annotates nothing is accepted, and its binding stands.
- **`from __future__ import annotations`** is accepted as a **no-op**, since it
  describes what Monty already does. See
  ./language.md for the other features.
- Consequences: `get_type_hints()` (which would evaluate the strings) is still
  not implemented, and code that reads `__annotations__` expecting type
  *objects* sees strings. CPython 3.14's `@dataclass` reads evaluated objects
  (`annotationlib.Format.FORWARDREF`), but keeps a string path for `ClassVar` /
  `InitVar` so PEP 563 code still works, which is what makes stringized
  annotations enough to build on.

If you need real type validation, do it on the *host* side around the
sandbox boundary.
