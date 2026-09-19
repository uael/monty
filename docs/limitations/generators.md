# Generators

A function whose body contains `yield` returns a generator: the call binds the
arguments, and the body runs only when something resumes it. `next()`, `send()`,
`for`, unpacking, and every builtin that walks an iterator drive it. A generator
is its own iterator, `finally` blocks run where they stand, and a body that
raises leaves the generator exhausted exactly as a return does.

While it runs, a generator's frame is an ordinary frame on the VM's own frame
stack; `yield` lifts it back off into the generator object. Its saved state is
the frame and nothing else, so what is written below follows from what a frame
here can hold.

## Not implemented

- **`yield from`** raises `NotImplementedError: yield from expressions` when the
    code is parsed.
- **`close()` and `throw()`** are absent: a generator answers `__iter__`,
    `__next__` and `send` only, and any other attribute raises `AttributeError`.
- **Async generators.** A `yield` inside an `async def` is refused at compile
    time with `'yield' inside an async function is not supported`, rather than
    building something that does not answer `__aiter__` / `__anext__`.
- **Returning a value.** `return value` inside a generator body is refused at
    compile time with `returning a value from a generator is not supported`.
    CPython puts that value on the `StopIteration` it raises, and an exception
    here carries a message rather than a Python object, so it is refused instead
    of dropped silently. Bare `return`, and falling off the end, work and raise a
    plain `StopIteration`.
- **`gi_frame`, `gi_running`, `gi_code`, `gi_yieldfrom`** and the rest of the
    introspection attributes are absent.

## Divergences

- **An unfinished generator that is collected does not run its `finally`
    blocks.** CPython closes a generator when it is finalized, which runs them.
    Destruction here is an iterative heap walk that cannot re-enter the VM to run
    Python, so the saved frame's values are released and its `finally` blocks are
    not. A generator driven to exhaustion runs them normally, where they stand.

- **A generator driven from inside a builtin cannot suspend to the host.**
    `for x in gen` and an explicit `next(gen)` put the generator's frame on the
    VM's own stack, so a host call inside the body suspends the session as it
    would anywhere. The builtins that walk an iterator themselves — `list()`,
    `tuple()`, `sum()`, `sorted()`, a comprehension — drive the generator through
    a nested run of the VM loop, and a host suspension cannot cross that boundary;
    it is reported inside the generator instead. This is the restriction every
    synchronous re-entry in Monty carries, not one generators add.

- **A generator expression is not lazy.** `(x for x in xs)` still compiles as a
    list comprehension, so it is built eagerly and is a `list`, not a generator.
    `type((x for x in xs))` reads `list`, and an infinite source does not
    terminate. Only `def`-with-`yield` builds a generator today.

- **A `yield` that can never run does not make a generator.** In
    `def f(): return 1` followed by an unreachable `yield`, CPython's symtable
    still marks the function a generator; here the unreachable statement is
    dropped before the body is compiled, so `f()` is an ordinary call. Only a
    `yield` the compiler emits counts.

- **`yield` outside a function** raises `SyntaxError: 'yield' outside function`
    at compile time, as in CPython. A class body counts as outside a function.
