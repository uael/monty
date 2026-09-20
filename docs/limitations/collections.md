# `collections` module

`deque`, `namedtuple`, `defaultdict`, and `Counter` are implemented and
feature-complete against CPython 3.14 apart from the divergences below. The
exact supported surface is pinned by the custom typeshed stub
(`crates/monty-typeshed/custom/collections/__init__.pyi`) and
`crates/monty/tests/collections.rs`. `defaultdict` and `Counter` are exposed as
the type objects themselves (like `deque`), so `type(d) is defaultdict` and
`isinstance(c, Counter)` hold.

## Not implemented

`OrderedDict`, `ChainMap`, `UserDict`, `UserList` and `UserString`. Importing
one raises
`ImportError: cannot import name 'OrderedDict' from 'collections' (unknown location)` (or `AttributeError`
as an attribute). The narrowed typeshed stub makes
`from collections import OrderedDict` a type error too, rather than something
that type-checks and then fails at runtime.

## `collections.abc`

The submodule exists and exports `Callable`, `Coroutine`, `Generator`,
`Iterable`, `Iterator`, `Mapping` and `Sequence`. They answer `isinstance`, and
name a type in an annotation, and nothing else:

- **Each name is the same object `typing` exports under that name**, a
    `typing._SpecialForm`, not an abstract base class. `repr(Callable)` is
    `typing.Callable` where CPython writes `<class 'collections.abc.Callable'>`,
    and `collections.abc.Callable is typing.Callable` holds where CPython says
    they differ.
- **`isinstance` answers from what the value is**, because there is no
    `__subclasshook__` and no registry here. A sandbox class that defines
    `__iter__` is an `Iterable` and one that defines `__iter__` and `__next__`
    is an `Iterator`, as in CPython, but `Sequence` and `Mapping` are the
    built-in types alone: `Sequence` is `str`, `bytes`, `list`, `tuple`, a
    `namedtuple`, `range` and `deque`, and `Mapping` is `dict`, `defaultdict`
    and `Counter`. A class of your own is neither, where CPython lets one
    register. `issubclass(list, Sequence)` raises
    `TypeError: issubclass() arg 2 must be a class, or tuple of classes`.
- **`Callable` reads False for an instance of a class with a `__call__`**,
    because Monty dispatches no `__call__`; it is the answer `callable()`
    gives, see [classes.md](classes.md).
- **`Iterator` reads False for `map`, `filter`, `zip`, `enumerate` and
    `reversed`**, which are eager here and give a `list`, so they read as a
    `Sequence` instead; see [builtins.md](builtins.md).
- **None of them is callable or subscriptable.** `Callable[[int], int]` raises
    `TypeError: 'typing._SpecialForm' object is not subscriptable`, the same
    refusal the `typing` name gets (see [typing.md](typing.md)). An annotation
    is never evaluated, so writing one costs nothing.
- **Every other name CPython exports is missing**, including `Hashable`,
    `Awaitable`, `Container`, `Set`, `MutableMapping` and the rest:
    `from collections.abc import Hashable` raises
    `ImportError: cannot import name 'Hashable' from 'collections.abc' (unknown location)`.
- **`import collections.abc` needs an alias**, as every dotted module does:
    write `import collections.abc as abc`, `from collections import abc`, or
    `from collections.abc import ...`. See [modules.md](modules.md).

## `deque`

- **`d * 1.5`** raises `TypeError: unsupported operand type(s) for *: 'collections.deque' and 'float'`, where CPython
    says `can't multiply sequence by non-int of type 'float'`. Shared with `list`.
- **Repeat counts in `[2**63, 2**64)` are accepted** where CPython rejects them:
    Monty's repeat count is a `usize`, CPython's a C `ssize_t`. Only observable
    for a bounded deque, whose result truncates to `maxlen`: `deque([1, 2], maxlen=2) * 2**63` yields
    `deque([1, 2], maxlen=2)` in Monty but `OverflowError`
    in CPython. `2**64` or more raises `OverflowError` in both.
- **Returned to the host as a plain `list`**, so the `maxlen` bound and the
    deque-ness are lost; sending it back yields a `list`. (Same as
    defaultdict/Counter, which arrive as plain dicts.)
- **Mutation during iteration is not detected through `enumerate`/`zip`/`map`/
    `filter`/`reversed`**: those are eager (see [builtins.md](builtins.md)), so
    the deque is fully read before the loop body runs. `for x in d` and explicit
    `iter()`/`next()` detect it exactly as CPython does.
- `d += <any iterable>` works (it is `extend`), even though `list`'s `+=` still
    accepts only another list.
- **Extending from an eager builtin loses the partial result when it raises.**
    `extend`/`extendleft`/`+=` append each item as the source yields it, so an
    iterator raising part-way leaves the earlier items in place, as in CPython.
    But `map`/`filter`/`zip`/`enumerate` are eager (see
    [builtins.md](builtins.md)), so `d += map(f, xs)` with a raising `f` raises
    before the extend begins and appends nothing, where CPython appends whatever
    was yielded first.

`del d[i]` and subclassing (`class Q(deque)`) both fail at *compile* time: the
`del` statement and class inheritance are unimplemented Monty-wide (see
[language.md](language.md) / [classes.md](classes.md)), not deque limitations.

## `namedtuple`

Field-name validation matches CPython's messages exactly; see
[namedtuple.md](namedtuple.md) for the tuple-inherited surface and its
divergences. `repr(Point)` is `<class 'Point'>` where CPython gives
`<class '__main__.Point'>`, the repo-wide unqualified-class-name pattern.

## `defaultdict`

- **A `default_factory` cannot call an external or `os` function**; a
    missing-key access then raises `NotImplementedError`. Plain factories (`int`,
    `list`, `lambda`, ordinary functions) work. This applies to every callback
    Monty invokes mid-expression (the `key=` of `sorted`/`min`/`max`, `map`,
    `filter`, `__repr__`), not just defaultdict.
- **Crosses the host boundary as a plain `dict`**: the `default_factory` is a
    function and cannot cross, so sending the dict back yields a plain `dict`.

## `Counter`

- **`elements()` returns a list**, not CPython's lazy iterator. The values and
    order match, but the whole sequence is built up front, so a very large count
    can hit the memory limit where CPython would stream.
- **Crosses the host boundary as a plain `dict`.**

## Qualified vs bare type names

`deque` and `defaultdict` are C types, so CPython qualifies them
(`collections.deque`) in `repr(T)` and in every type-naming error message
(`unsupported operand type(s)`, `object is not callable`, `object has no attribute`) while `__name__` stays bare
(`'deque'`); Monty matches both
surfaces.

`Counter` is a Python-level class, so CPython gives the bare name everywhere
except `repr(Counter)`, which is `<class 'collections.Counter'>` where Monty
writes `<class 'Counter'>`. Everywhere else — `__name__`, the `cannot use ...`
unhashable clause, other type-naming errors — the bare name matches.
