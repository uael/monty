# `typing` module

`typing` exists so type-annotated code can `import` it without
`ModuleNotFoundError`. **No runtime type checking happens.** The forms are
inert marker objects; subscripting them (`list[int]`, `Optional[str]`,
`Union[int, str]`) returns a placeholder value and validates nothing.

## Names defined

`Any`, `Optional`, `Union`, `List`, `Dict`, `Tuple`, `Set`, `FrozenSet`,
`Callable`, `Type`, `Sequence`, `Mapping`, `Iterable`, `Iterator`,
`Generator`, `ClassVar`, `Final`, `Literal`, `TypeVar`, `Generic`,
`Protocol`, `Annotated`, `Self`, `Never`, `NoReturn`, `TYPE_CHECKING`.

`TYPE_CHECKING` is `False`, as in CPython at runtime.

## Not implemented

- `get_type_hints`, `get_args`, `get_origin`, `cast`, `assert_type`,
  `assert_never`, `overload`, `final`, `runtime_checkable`, `NewType`,
  `NamedTuple`, `TypedDict`, `dataclass_transform`, `ParamSpec`,
  `Concatenate`, `Unpack`, `TypeAlias`, `LiteralString`.
- `typing.TypeAliasType` is **not** exported by this module even though the
  type exists: a PEP 695 `type X = ...` statement builds one (see below), but
  `from typing import TypeAliasType` raises `ImportError`, so the type object
  cannot be named and `isinstance(X, TypeAliasType)` has no equivalent.
- Annotation introspection on **functions and modules**: `__annotations__` is
  not populated there. Class `__annotations__` **is** populated; see below.

## PEP 695 type parameters

`def f[T](x: T) -> T:`, `class C[T]:` and `type X[T] = ...` all parse, but the
type parameters bind **nothing**. CPython puts each one in an implicit scope
around the construct, holding a `TypeVar` / `TypeVarTuple` / `ParamSpec`; Monty
drops them. Consequences:

- Reading a type parameter in a body raises `NameError`, where CPython returns
  the `TypeVar`: `def f[T](x): return T` fails.
- **A same-named outer binding shadows through instead.** If a module-level
  `T` exists, `def f[T](x): return T` returns *that* value in Monty and the
  `TypeVar` in CPython. This is the one case that gives a wrong answer rather
  than an error.
- Bounds and defaults (`def f[T: int, U = str]`) are parsed and discarded; the
  expressions are never evaluated, so an error in one is never raised.
- `__type_params__` is not exposed on functions, classes, or aliases; reading
  it raises `AttributeError` where CPython returns a (possibly empty) tuple.

None of this affects annotations, which are stringized and never evaluated.

## PEP 695 type aliases (`type X = ...`)

A `type` statement binds a real `TypeAliasType` object, at module scope, in a
function, and in a class body. `X.__name__`, `X.__value__`, `repr(X)` and
`str(X)` all match CPython, and `__value__` is evaluated lazily on first read
and then memoized, so an alias may mention itself
(`type Wire = str | list[Wire]` is only an error if `__value__` is read and the
operators involved are unsupported). Divergences:

- `type(X).__name__` is `'typing.TypeAliasType'`, not CPython's bare
  `'TypeAliasType'`. Monty names every non-builtin type by its
  module-qualified path (the same choice documented for `deque` in
  ./collections.md); `repr(type(X))` and error messages match CPython exactly.
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
- The blocker is that Monty has no generic types: `list[int]` and
  `dict[str, int]` raise `TypeError: 'type' object is not subscriptable`, and
  `int | None` raises `TypeError: unsupported operand type(s) for |: 'type'
  and 'NoneType'`, so evaluated annotations would fail on the most common
  forms. Runtime `types.GenericAlias` and `|` unions are the prerequisite for
  matching PEP 649.
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
