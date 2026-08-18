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

```python
class Session(AbstractContextManager['Session']):
    def __exit__(self, exc_type, exc, tb):
        return False
```

It contributes the two methods CPython's does: an `__enter__` returning the
receiver and an `__exit__` returning `None`, either of which a subclass may
override. `isinstance` answers structurally, as CPython's `__subclasshook__`
does: any object whose class defines both methods is an instance, `suppress`
included.

- **`__exit__` is not abstract.** CPython marks it `@abstractmethod`, so a
  subclass that defines neither method cannot be instantiated; Monty names an
  abstract base without enforcing abstractness (see ./typing.md), so such a
  subclass instantiates and inherits an
  `__exit__` that swallows nothing. A subclass that defines `__exit__`, which is
  what CPython requires anyway, behaves identically in both.

## Not implemented

`contextmanager`, `asynccontextmanager`, `ExitStack`, `AsyncExitStack`,
`closing`, `aclosing`, `redirect_stdout`, `redirect_stderr`, `chdir`,
`nullcontext`, `AbstractAsyncContextManager`, and `ContextDecorator`. The
names are absent from the module namespace rather than stubbed, so they fail
type checking as well as raising `AttributeError`.

Generators and async generators do exist (see ./iter.md and ./asyncio.md), so
`contextmanager` and `asynccontextmanager` are absent rather than blocked.
