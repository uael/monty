# `functools` module

Monty implements `functools.partialmethod` and nothing else from `functools`.

```python
from functools import partialmethod

class Held:
    def ctl(self, kind: str) -> None: ...

    abort = partialmethod(ctl, 'abort')
    pause = partialmethod(ctl, 'pause')
```

## `partialmethod`

Matches CPython:

- The stored positional arguments go **after** the receiver and **before** the
  call's own, so `obj.abort(5)` calls `ctl(obj, 'abort', 5)`.
- Stored keywords apply to every call and a call may override them
  (`{**stored, **given}`). Overriding a stored keyword *positionally* is the
  ordinary duplicate-argument `TypeError`, as in CPython.
- Reached through the class (`Held.abort(obj)`), the receiver is simply the
  caller's first argument.
- `func`, `args` and `keywords` are readable.
- The first argument is checked for callability at construction:
  `partialmethod(1)` raises
  `TypeError: the first argument 1 must be a callable or a descriptor`.
- `repr` renders the function and every stored argument
  (`functools.partialmethod(<function ctl>, 'abort')`).

### Divergences

- **It binds through Monty's class-member mechanism, not `__get__`.** Monty has
  no descriptor protocol (see ./classes.md); a `partialmethod` in a class body
  binds its receiver because the class-member path treats it as a method, the
  same way it treats a plain function. The observable behaviour is CPython's,
  but `pm.__get__` raises `AttributeError` where CPython returns a
  `functools.partial`, and a `partialmethod` stored anywhere other than a class
  body is never bound. Nesting one inside another still behaves as CPython's
  does, even though CPython reaches that by flattening at construction and
  Monty by ordinary call chaining.
- **CPython also accepts a descriptor as `func`.** Monty has none, so only a
  callable is accepted; the rejection message is CPython's either way.
- **`__isabstractmethod__`, `__doc__` and `__name__` are absent** on the
  descriptor, as they are on every Monty function (see ./language.md).
- `repr(functools.partialmethod)` renders `<class 'partialmethod'>` where
  CPython qualifies it as `<class 'functools.partialmethod'>`. The bare name is
  CPython's `tp_name`, so error messages match.

## Not implemented

`partial`, `reduce`, `wraps`, `update_wrapper`, `cache`, `lru_cache`,
`cached_property`, `total_ordering`, `singledispatch`, `singledispatchmethod`,
`cmp_to_key` and `WRAPPER_ASSIGNMENTS`. The names are absent from the module
namespace rather than stubbed, so they fail type checking as well as raising
`AttributeError`.

`wraps` has no equivalent here in any case: a function exposes no attributes to
copy (see ./language.md).
