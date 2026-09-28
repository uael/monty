# Generators

A function whose body contains `yield` returns a generator, and so does a
generator expression: the call binds the arguments, and the body runs only when
something resumes it.
`next()`, `send()`, `close()`, `throw()`, `for`, unpacking, and every builtin
that walks an iterator drive it.
A generator is its own iterator, `finally` blocks run where they stand, and a
body that raises leaves the generator exhausted exactly as a return does.
`yield from` delegates to another generator or to any iterable, and a `throw()`
or a `close()` reaches the receiver first, at every depth.

While it runs, a generator's frame is an ordinary frame on the VM's own frame
stack; `yield` lifts it back off into the generator object. Its saved state is
the frame and nothing else, so what is written below follows from what a frame
here can hold. A coroutine is the same saved frame, stopping at an `await`
rather than at a `yield`, so `send()`, `throw()` and `close()` drive one the
same way; see [asyncio.md](asyncio.md) for where the two part.

## Not implemented

- **Async generators.** A `yield` inside an `async def` is refused at compile
    time with `'yield' inside an async function is not supported`, rather than
    building something that does not answer `__aiter__` / `__anext__`.
- **`gi_frame`, `gi_running`, `gi_code`, `gi_yieldfrom`** and the rest of the
    introspection attributes are absent. `__iter__`, `__next__`, `send`,
    `close` and `throw` are answered when they are called on a generator, and
    any other name raises `AttributeError`. Reading one of the five rather than
    calling it raises `AttributeError` too, as reading any builtin method
    does.
- **A generator expression has a limit on its clauses.** Each `for` and each
    `if` of one is a level of nesting, and they are counted against the same
    budget as nesting written out in the source: 200 levels in a release build,
    where `SyntaxError: Source is too deeply nested` is the refusal. CPython
    has no such limit. The budget bounds compiler recursion on
    attacker-controlled source.
- **`throw()` takes an exception instance only.** CPython also accepts the
    older `throw(type, value, traceback)` form, deprecated there since 3.12.
- **A `yield` that can never run does not make a generator.**
    `def f(): return; yield` is an ordinary function returning `None` here,
    where CPython's symtable still marks it a generator that stops at once:
    dead code is dropped before the body is read for a `yield`, so only a
    `yield` the compiler emits counts. A `yield` under `if False:` is reachable
    code to the reader and does make one.

## Divergences

- **An unfinished generator that is collected does not run its `finally`
    blocks.** CPython closes a generator when it is finalized, which runs them.
    Destruction here is an iterative heap walk that cannot re-enter the VM to run
    Python, so the saved frame's values are released and its `finally` blocks are
    not. An explicit `close()` runs them, and a generator driven to exhaustion
    runs them normally, where they stand.

- **A generator that something walks for you cannot suspend to the host.**
    `gen.send(...)` and `gen.__next__()` put the generator's frame on the VM's
    own stack, so a host call inside the body suspends the session as it would
    anywhere. Everything else that walks an iterator does it through a nested
    run of the VM loop: `for`, the `next()` builtin, `list()`, `tuple()`,
    `sum()`, `sorted()` and a comprehension. A host suspension cannot cross that
    boundary, so it is reported inside the generator as a `NotImplementedError`
    that names the external function. This is the restriction every synchronous
    re-entry in Monty carries, not one generators add.

- **A returned value reaches a `yield from` but not a `StopIteration`.**
    `return value` inside a generator body gives that value to a `yield from`
    waiting on it, which is where CPython reads it from too. A resumer that
    catches the `StopIteration` instead finds nothing on it: `e.args` is empty
    and `e.value` raises `AttributeError`, because an exception here carries a
    message rather than a Python object. The value travels inside the
    interpreter, so only the delegation can read it.

- **A receiver of a `yield from` that is no generator is never closed.** PEP 380
    closes the receiver before the exception of a `throw()`, and before the exit
    of a `close()`, reaches the generator that waits on it. That happens here for
    a receiver that is a generator, at every depth, innermost first. A receiver
    that is any other iterator is dropped instead, so a `close` method on a
    hand-written iterator class does not run.

- **`yield` outside a function** raises `SyntaxError: 'yield' outside function`
    at compile time, as in CPython. A class body counts as outside a function.
