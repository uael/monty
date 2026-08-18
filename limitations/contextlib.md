# `contextlib` module

Monty implements `suppress` and `AbstractContextManager`. Nothing else from
`contextlib` exists.

```python
from contextlib import suppress

with suppress(ValueError, OverflowError):
    ...
```

## `suppress`

Behaves as CPython's does, including the parts that follow from its being a
plain Python class rather than a validating constructor:

- Variadic, and `suppress()` with no arguments suppresses nothing.
- Matching is by subclass, so `suppress(Exception)` swallows a `ValueError`.
- `__enter__` returns `None`, so `with suppress(...) as x` binds `None`.
- **Arguments are checked on exit, not on construction.** `suppress(1)` builds
  without complaint and raises `TypeError: issubclass() arg 2 must be a class,
  a tuple of classes, or a union` only if an exception actually reaches
  `__exit__`.
- **The check stops at the first match.** `suppress(ValueError, 1)` swallows a
  `ValueError` without ever looking at the `1`, while `suppress(1, ValueError)`
  raises. This differs from an `except` clause, which rejects a bad element even
  when an earlier one already matched.
- A manager is reusable and keeps no state between blocks.

### Divergences

- **`type(s).__name__` is `'suppress'`**, matching CPython's `tp_name` and so
  every error message, but `repr(contextlib.suppress)` renders
  `<class 'suppress'>` where CPython qualifies it as
  `<class 'contextlib.suppress'>`. The `repr` of an *instance* is qualified in
  both (`<contextlib.suppress object at 0x…>`).
- **`s._exceptions` is not readable.** CPython exposes the argument tuple under
  that private name; Monty stores it out of reach.
- **Methods cannot be extracted**, as for every builtin type
  (see ./language.md), though `s.__enter__()` and `s.__exit__(...)` can be
  called directly.
- No traceback object is passed to `__exit__`; the third argument is always
  `None` (see ./with.md).

## `AbstractContextManager`

A class the interpreter provides (see ./typing.md for that family), so it can
be named as a base, subscripted, and asked about:

- **It cannot be used as a base class.** A base must be a class defined in the
  sandbox or a builtin exception (see ./classes.md), and this is neither: it is
  a marker with no namespace of its own, so `class C(AbstractContextManager)`
  is refused. Nothing is lost by leaving it out — defining `__enter__` and
  `__exit__` on a plain class makes it a working context manager, and CPython's
  base contributes only a default `__enter__` returning `self` and an
  `__exit__` returning `None`.
- **Subscripting yields the base itself.** `AbstractContextManager[T]` is a
  `types.GenericAlias` in CPython; Monty has no alias object, so the parameter
  is dropped rather than recorded and `AbstractContextManager['X'] is
  AbstractContextManager` holds. The form is accepted so that annotations and
  base-class expressions parse and type-check.
- `isinstance(x, AbstractContextManager)` raises `TypeError: isinstance() arg 2
  must be a type, a tuple of types, or a union`. CPython answers it structurally
  through `__subclasshook__`, which needs the abc machinery Monty does not have.

## Not implemented

`contextmanager` and `asynccontextmanager` (both need generators, which Monty
does not have), `ExitStack`, `AsyncExitStack`, `closing`, `aclosing`,
`redirect_stdout`, `redirect_stderr`, `chdir`, `nullcontext`,
`AbstractAsyncContextManager`, and `ContextDecorator`.
