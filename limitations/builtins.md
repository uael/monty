# Built-in functions

Monty implements a subset of CPython's builtins. Referencing any name not
listed here raises `NameError` at runtime; there is no fallback to a host
Python.

## Implemented builtin functions

`abs`, `all`, `any`, `bin`, `chr`, `divmod`, `enumerate`, `filter`,
`getattr`, `hasattr`, `hash`, `hex`, `id`, `isinstance`, `issubclass`, `iter`,
`len`, `map`, `max`, `min`, `next`, `oct`, `open`, `ord`, `pow`, `print`,
`repr`, `reversed`, `round`, `setattr`, `sorted`, `sum`, `type`, `vars`, `zip`.

`classmethod`, `staticmethod`, `property` and `super` are also builtin names;
what they do is in ./classes.md, which covers the descriptors class-attribute
lookup invokes and the zero-argument `super()`.

## Implemented type constructors (also builtins)

`bool`, `bytes`, `dict`, `float`, `frozenset`, `int`, `list`, `range`,
`set`, `slice`, `str`, `tuple`. Exception classes (`ValueError`,
`TypeError`, etc.) are also names in the builtin namespace.

`object` is a name too, and answers `isinstance` / `issubclass` for every
value and every class, which is what makes it usable as the contract that
proves nothing. It builds no instances (`object()` raises `TypeError`) and takes no
parameters, because nothing in Monty needs a bare instance of it and
CPython refuses the subscript as well.

## Builtins that are NOT implemented

These raise `NameError`:

- **Code execution**: `eval`, `exec`, `compile`, `__import__`. Deliberate:
  sandboxed code must not be able to compile new code at runtime.
- **Namespace introspection**: `globals`, `locals`, `dir`. `vars` *is*
  implemented, but only for modules; see below.
- **Interactive**: `input`, `breakpoint`, `help`.
- **Construction / coercion**: `bytearray`, `complex`, `memoryview`,
  `format`, `ascii`.
- **Other**: `callable`, `delattr`, `aiter`, `anext`.

## One name only the type checker resolves

`UnicodeError` type-checks and then raises `NameError` when the code runs. It
is structural in the vendored stub rather than deliberate: it is the declared
base of `UnicodeDecodeError` / `UnicodeEncodeError`, so filtering it away
would leave the classes naming it describing nothing. Every other builtin name
agrees in both directions, which
`crates/monty-runtime/tests/typeshed_builtins.rs` pins against the interpreter's
own `vars(builtins)`.

The catch behaviour is unaffected: Monty's codec errors derive straight from
`ValueError`, so `except ValueError:` catches them exactly as the stub's
hierarchy implies.

## `vars()`

`vars(module)` returns the module's namespace as a `dict`, which is what makes
a value-to-name reverse lookup possible:

```python
import builtins

def name_of(value):
    for name, got in vars(builtins).items():
        if got is value:
            return name
```

`builtins` is importable for exactly this reason. Its namespace is assembled
from the same three sources a bare name resolves against, so it holds every
builtin function, type constructor, builtin exception class, and the
singletons `None`, `True`, `False`, `Ellipsis` and `NotImplemented`. The
exception classes that belong to a module rather than to builtins
(`json.JSONDecodeError`, `io.UnsupportedOperation`, `re.PatternError`,
`dataclasses.FrozenInstanceError`) are absent, as in CPython.

Divergences:

- **The result is a copy, not the live mapping.** A module owns its namespace
  inline, so there is no second reference to hand out. Writing to the returned
  dict therefore does not change the module, where in CPython
  `vars(m)['x'] = 1` sets `m.x`. Reading is identical.
- **Only modules have a `__dict__`.** Every other object raises
  `TypeError: vars() argument must have __dict__ attribute`, including class
  objects and instances — CPython returns a `mappingproxy` for a class and the
  instance dict for an instance (see ./classes.md, which lists `__dict__` as
  unavailable).
- **`vars()` with no argument raises** `NotImplementedError` rather than
  returning the calling frame's `locals()`, which Monty cannot build. Returning
  an empty dict would be worse: it would look like a frame with no names.

## Behavioural divergences

- **`repr` of a dict being mutated by its own elements** — Monty iterates the
  live entries like CPython, but deletion compacts Monty's dense entry storage
  where CPython leaves a tombstone in place: a key deleted from inside a user
  `__repr__` running *during that dict's repr* shifts later entries down, so
  the entry after the deleted one can be skipped from the output where CPython
  would still print it. Insertions during repr match CPython (appended and
  printed), as do list (live length, mid-repr pops truncate / appends extend),
  `set`, `collections.deque` and `collections.Counter` (all snapshot, like
  CPython).
- **Filling a dict from a sequence of pairs**: `dict(pairs)`, `d.update(pairs)`
  and `d |= pairs` all reject a malformed element with a `TypeError` reading
  `dictionary update sequence element has length 1; 2 is required`, where
  CPython raises a `ValueError` naming the offending index as well
  (`... element #0 has length 1; ...`); an element longer than two says `has
  length > 2` rather than its real length. An element that is not iterable at
  all names the type Monty tried to iterate (`'int' object is not iterable`)
  where CPython says `object is not iterable`. Only the wording and the
  exception type differ: the same inputs are accepted and rejected.
- **`dict` has no `__or__` / `__ior__` attribute**: the operators themselves
  work, but spelling one as a method call (`{'a': 1}.__or__({'b': 2})`,
  `d.__ior__(other)`) raises `AttributeError`, as do Monty's other builtin
  dunders (`{}.__eq__({})`, `(3).__add__(4)`).
- **`enumerate`, `zip`, `map`, `filter` and `reversed` are eager, not lazy** —
  each drains its source and returns a `list`, so `type(enumerate(x)).__name__`
  is `'list'` rather than `'enumerate'`. Observable several ways: a
  side-effecting callable runs for every item at the call itself rather than as
  the result is consumed; the whole result is held in memory at once, so an
  infinite iterator (e.g. `map(f, itertools.count())`) never returns and runs
  until a resource limit trips; the result can be indexed and re-iterated, which
  CPython forbids; and mutating the source from inside the loop body is never
  observed, so containers that detect mutation during iteration (`dict`, `set`,
  `collections.deque`) will not raise when looped over via one of these. `zip`
  and multi-iterable `map` stop at the shortest input, so pairing an infinite
  iterable with a finite or empty one stays bounded. A plain `for x in
  container` is lazy and does detect mutation. See ./itertools.md.
- **Arity-error wording for some str/bytes methods** — a handful of
  keyword-accepting methods (e.g. `str.split`, `str.rsplit` and the `bytes`
  equivalents) report too-many-arguments as `split expected at most 2
  arguments, got 3`, where CPython 3.14's Argument Clinic pre-counts
  positionals *plus* kwargs and says `split() takes at most 2 arguments (3
  given)`. Methods audited against CPython (`encode`, `decode`,
  `expandtabs`, `splitlines`, `replace`, …) already match; the remainder
  need a per-function `at_most_total` audit.
- **`getattr(obj, name)`** — if the resolved attribute would be an async
  coroutine, external function, or OS call, raises `TypeError:
  "getattr(): attribute is not a simple value"` rather than returning a
  bound method object. Use direct attribute access (`obj.name(...)`) for
  these.
- **`int(x, base=10)`** — string/bytes parsing accepts ASCII digits only;
  CPython also accepts non-ASCII Unicode decimal digits (`int('١٢')` == 12),
  which Monty rejects with `invalid literal for int() with base 10`.
- **`bytes(source)`** — an iterable of ints is not supported: CPython's
  `bytes([65, 66])` == `b'AB'`, Monty raises `TypeError: cannot convert
  'list' object to bytes`. The int / str-with-encoding / bytes source forms
  all work. A count above `i64` (`bytes(2**70)`) gives that same `TypeError`,
  not CPython's `OverflowError: cannot fit 'int' into an index-sized integer`.
- **`isinstance(obj, T)`** — `T` must be a built-in type (`int`, `str`,
  `list`, ...), a built-in exception class, a sandbox-defined class (see
  ./classes.md), an abstract class or union (see ./typing.md), `type` /
  `staticmethod` / `classmethod`, or a tuple of those. Passing a host-supplied
  dataclass / namedtuple as the second argument raises `TypeError`, as does
  `enumerate` or `super`, which are classes in CPython and builtin functions
  here.
- **`iter()`** — see ./iter.md for iterator and `iter(callable, sentinel)` divergences.
- **`pow(base, exp, mod)`** — the three-argument form requires all integers and
  rejects negative exponents with `ValueError` instead of computing a modular
  inverse. Non-modular exponents whose result cannot be materialized raise
  `OverflowError` (see ./resource_limits.md).
- **`sorted(iterable, *, key=None, reverse=False)`** — `key` and `reverse`
  must be passed by keyword; positional forms raise `TypeError`.
- **`round(n, ndigits)`** — `ndigits` values outside the i64 range are
  clamped by sign. For floats this matches CPython (which clamps to
  `Py_ssize_t`); for an int `n` with a hugely negative `ndigits`, CPython
  tries to materialise `10**-ndigits` and dies with `MemoryError` where
  Monty returns `0` immediately.
- **`print`** — writes via the host print callback. `file=`, `flush=` are
  not honoured; `sep=` and `end=` are.
- **Identity of host-supplied callables** — host functions passed in as inputs
  (`MontyObject::Function`) lose their host object identity at the sandbox
  boundary. Live external functions are identified by lookup name, so distinct
  host callables with the same name share `is`, equality, `id()`, and `hash()`
  results. Once the last sandbox reference is dropped, a later conversion of
  that name may create a new function object.
- **Type objects across the host boundary** — a `type` object (a class, not an
  instance) round-trips in both directions.
  - *Sandbox → host* (external/OS-call argument, or a `.run()` return value): the
    type is reconstructed as the corresponding host class. Genuine builtins
    (`int`, `str`, `type`, `bytes`, `list`, `dict`, `property`, …) resolve to the
    real builtin; Monty's modeled stdlib types map to their host stdlib class:
    `datetime`/`date`/`timedelta`/`timezone` → `datetime.*`,
    `re.Pattern`/`re.Match` → `re.*`, the binary/text file types → `io.*`. The
    `pathlib.Path` class maps to `pathlib.PurePosixPath`, consistent with how Path
    *instances* round-trip, and instantiable on every host OS. A type with no
    faithful host class (e.g. an internal function or cell type) cannot be
    reconstructed and surfaces as an `AttributeError` from the host call.
  - *Host → sandbox* (input, or an external-call return value): the same recognized
    builtins and modeled stdlib types are preserved as type objects, so
    `isinstance(x, the_type)` works inside the sandbox. Recognition is by
    type-object **identity**, not class name/module, so a class that forges
    `__name__`/`__module__` to impersonate a builtin is *not* treated as one. Every
    `pathlib` path class collapses to `PurePosixPath` (it re-emerges as
    `PurePosixPath`). A host class Monty does **not** model (e.g. a user-defined
    class) is not preserved as a type; it degrades to a callable, appearing inside
    the sandbox as a `function` rather than a `type`.
