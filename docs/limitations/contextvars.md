# `contextvars`

The module exports `ContextVar` and `Token`, and nothing else.

`ContextVar(name, /, *, default=...)`, `var.get()`, `var.get(default)`, `var.set(value)`, `var.reset(token)` and
`var.name` behave as CPython's do, as does `with var.set(value):`, the context manager CPython 3.14 gave a token.

## There is one context

Monty has no `Context` object, so there is nothing to copy, run against, or iterate.
A variable's value is held by the variable itself, which is what a single context makes observable.

- **`Context` and `copy_context()` do not exist.** `from contextvars import Context` raises `ImportError`, and
    `contextvars.Context` raises `AttributeError`.
- **A coroutine does not get a context of its own.** `asyncio.gather()` runs its coroutines against the same
    variables, so a `set()` in one is seen by the others and outlives the `gather()`.
    CPython wraps each coroutine in a `Task` carrying a copy of the current context, where the `set()` is invisible
    outside it. Reset the variable yourself, or use `with var.set(value):` (see [asyncio.md](asyncio.md)).

## `Token`

- **`token.old_value` and `Token.MISSING` are absent** and raise `AttributeError`.
    A token exposes only `token.var`, the variable it belongs to.
- **Assigning to `var.name` or `token.var`** raises Monty's
    `AttributeError: '_contextvars.ContextVar' object has no attribute 'name' and no __dict__ for setting new attributes`,
    where CPython says `AttributeError: readonly attribute`.

## Errors

- **`var.get()` on an unset variable with no default** raises `LookupError` carrying the variable's repr as a string,
    where CPython passes the variable itself: `exc.args[0]` is a `str` here and a `ContextVar` there.
    `str(exc)` is the same text in both.
