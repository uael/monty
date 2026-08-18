# `contextvars` module

Monty implements `ContextVar` and the `Token` its `set()` returns. Both behave
as CPython's do for a program running in a single context, which is the only
context Monty has.

```python
from contextvars import ContextVar

depth: ContextVar[int] = ContextVar('depth', default=0)
token = depth.set(depth.get() + 1)
try:
    ...
finally:
    depth.reset(token)
```

## Implemented

- `ContextVar(name, *, default=...)` — `name` is positional-only, as in CPython.
- `ContextVar.name` — read-only.
- `ContextVar.get()` / `get(default)` — the current value, else the argument,
  else the construction default, else `LookupError` whose message is the
  variable's own `repr`, matching CPython.
- `ContextVar.set(value)` — returns a `Token`.
- `ContextVar.reset(token)` — restores what the token recorded, including
  returning the variable to *unset* when the token predates its first `set()`.
  A token is single-use and belongs to one variable; both rejections carry
  CPython's wording.
- `Token.var`, `Token.old_value`.
- `repr()` of both, including the `used` marker on a spent token and the
  `default=` half of a variable that has one.

## There is only one context

`Context`, `copy_context()`, `Context.run()` and `ContextVar`'s per-context
storage do not exist. A variable's value lives on the variable itself, so:

- **`asyncio` tasks share one set of values.** CPython copies the current
  context into each task, so a `set()` inside one task is invisible to its
  siblings and to the code that spawned it. In Monty every task reads and
  writes the same slot, and a `set()` that is never `reset()` outlives the task
  that made it. Code that relies on per-task isolation behaves differently;
  code that uses a variable as an ambient value for one run behaves the same.
- There is no way to snapshot or restore a whole set of variables at once.
  `reset(token)` is the only unwind, and it is per variable.

## Divergences from CPython

- **`Token.MISSING` does not exist**, so `token.old_value` reads as `None` for a
  token whose `set()` was the variable's first. CPython reports the
  `Token.MISSING` sentinel there and `None` only when `None` was genuinely the
  previous value; Monty cannot tell the two apart through this attribute.
  `reset()` itself is unaffected — it restores *unset*, not `None`.
- **Two arity messages differ in wording.** `ContextVar()` reports
  `takes at least 1 positional argument (0 given)` where CPython says
  `exactly`, and `ContextVar('a', 'b')` reports
  `takes at most 1 argument (2 given)` where CPython says
  `at most 1 positional argument (2 given)`. The exception type, the limit and
  the given count all match; only CPython's `PyArg_ParseTupleAndKeywords`
  phrasing for a positional-only parameter is not reproduced.
- **`ContextVar` is not subscriptable at runtime.** `ContextVar[int]` is
  accepted by the type checker (the stub is generic) but raises `TypeError:
  type '_contextvars.ContextVar' is not subscriptable` if evaluated, so it
  belongs in an annotation, not in an expression.
- **Assigning `v.name` reports the generic attribute error**
  (`'_contextvars.ContextVar' object has no attribute 'name' and no __dict__
  for setting new attributes`), where CPython says `readonly attribute`. Both
  raise `AttributeError`, and neither lets the assignment through.
- **Methods cannot be extracted.** `v.get` raises `AttributeError`; call the
  methods directly. This applies to every builtin type in Monty
  (see ./language.md).

## Not implemented

`Context`, `copy_context`, `ContextVar` in a `Context.run()`, and constructing a
`Token` directly. `contextvars.Token` is not a module attribute at all: CPython
refuses to construct one (`cannot create '_contextvars.Token' instances`), so a
name that could only ever raise would buy nothing. Tokens are still reachable
as the value `set()` returns.
