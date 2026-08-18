# `iter()` and iterators

- `iter(callable, sentinel)` runs `callable` synchronously, so one that calls an external/OS function cannot suspend and raises `NotImplementedError`. Same limitation as `map`/`filter`/`sorted(key=...)`.
- `iter(callable, sentinel)` compares `result == sentinel`, where CPython compares `sentinel == result`; only observable if the two sides have asymmetric `__eq__`.
- A user instance defining `__call__` works, `__call__` being one of the dunders dispatched for sandbox classes (see ./classes.md); like every callable Monty drives itself, it cannot suspend on an external or OS call.
- The vendored type stub is upstream typeshed's verbatim, so `-t` accepts `iter(obj)` for an object with only `__getitem__`; Monty has no `__getitem__` iteration fallback and raises `TypeError` at runtime.
- `-t` accepts `for x in obj` (though not `a, b = obj`) for a class that opts out of iteration with `__iter__ = None`, which raises `TypeError` at runtime as it does in CPython.
- Iterator `repr()` values omit CPython's process-local memory address: `<list_iterator object>` rather than `<list_iterator object at 0x...>`.
- Built-in iterators do not expose their dunders as attributes: `hasattr(iter([1]), '__iter__')` is `False`, where CPython reports `True`. Iteration itself works; only attribute lookup of the dunder differs. This covers every built-in iterator, including the `itertools` adaptors.
- Iterator-specific attributes such as `__length_hint__` are not exposed.

## Generators

`yield` and `yield from` work, and a `def` containing one returns a generator
supporting `__iter__` / `__next__` / `send` / `throw` / `close`. Generator
expressions are lazy and single-shot. Divergences:

- **A generator cannot suspend to the host while a Rust-side consumer is
  driving it.** `list(gen)`, `sum(gen)`, `next(gen)`, `''.join(gen)`, tuple
  unpacking and every other builtin that drains an iterator hold a Rust stack
  frame across the step, so an external or OS function call inside the
  generator body raises `NotImplementedError` there. Driven by a `for` loop,
  by `yield from`, or by `async for`, the same generator body can suspend
  freely, because those step it on the VM's own frame stack. Same shape as the
  `map` / `filter` / `sorted(key=...)` limitation above.
- **`StopIteration` carries the return value as its message, not as
  `.value`.** `str(exc)` renders the returned value, matching CPython's
  `str()`, but Monty exceptions hold a message rather than an argument tuple,
  so `exc.value` raises `AttributeError`.
- **A `StopIteration` raised in a generator body propagates as itself**, ending
  whatever drives the generator, where CPython (PEP 479) replaces it with
  `RuntimeError: generator raised StopIteration`. A `return` still ends a
  generator the same way in both.
- **`throw()` takes one argument.** CPython's legacy
  `throw(type, value, traceback)` form is not accepted.
- **`throw()` / `close()` only delegate into a generator.** When a generator
  is suspended in `yield from`, an exception thrown in is forwarded to the
  delegate and its handlers run, but only if that delegate is itself a
  generator. Delegating to a plain iterable (`yield from [1, 2]`) forwards
  nothing, because such an iterator has no `throw`.
- **An abandoned generator never runs its `finally`.** Monty has no
  finalizers, so a suspended generator that simply goes out of scope is
  discarded without being closed; CPython closes it when it is collected. Call
  `close()` explicitly to run cleanup.
- **The protocol methods are call-only.** `gen.send(1)` and `gen.close()`
  work, but reading one without calling it (`m = gen.send`, `hasattr(gen,
  '__next__')`) raises `AttributeError` and answers `False`, as it does for
  every builtin type in Monty (see ./language.md).
- Generator `repr()` omits CPython's process-local memory address:
  `<generator object counter>` rather than `<generator object counter at
  0x...>`.
- Introspection attributes are absent: `gi_frame`, `gi_running`, `gi_code`,
  `gi_yieldfrom`, `gi_suspended` all raise `AttributeError`. Only the protocol
  methods exist.
- `yield` inside a generator expression is a `SyntaxError`, as in CPython, but
  Monty also rejects it in the outermost iterable (`(x for x in (yield))`)
  which CPython accepts. See ./language.md.
- A walrus inside a generator expression binds in the expression's own scope
  rather than the enclosing one. See ./comprehensions.md.
