# `asyncio` module and `async` / `await`

`async def` functions can suspend on `await`, and the host drives long-running
external calls. There is no event loop inside the sandbox; the host is the
loop.

## Module surface

The `asyncio` module exposes exactly two functions:

- `asyncio.run(coro)` — runs a coroutine to completion. Returns the value
  the coroutine `return`s, or re-raises an exception from it.
- `asyncio.gather(*awaitables)` — runs awaitables concurrently and returns
  a list of results. Always behaves as `return_exceptions=False`.
  Any keyword argument is rejected with
  `NotImplementedError: gather() does not yet support keyword arguments`,
  where CPython raises
  `TypeError: gather() got an unexpected keyword argument 'X'` because
  `return_exceptions` is a real kwarg there.

Not implemented (raise `AttributeError`):

`create_task`, `sleep`, `wait`, `wait_for`, `shield`, `to_thread`,
`new_event_loop`, `get_event_loop`, `get_running_loop`, `Queue`, `Lock`,
`Semaphore`, `Event`, `Future`, `Task`, `TaskGroup`, `timeout`,
`timeout_at`, `Timeout`, `as_completed`, `iscoroutine`, `ensure_future`,
the whole `asyncio.subprocess` / `asyncio.streams` / `asyncio.protocols`
surface.

`async with` works, so an async context manager written in Python is usable;
what is missing above is the `asyncio` objects themselves, `Lock` included.

## `async def` / `await`

- `async def` functions and `await` work; coroutines can call each other.
- **Coroutines are single-shot.** Awaiting the same coroutine object twice
  raises `RuntimeError`. Store the *result*, not the coroutine, if you need
  it again.
- `await` on a non-awaitable raises `TypeError`.
- Async comprehensions (`[x async for x in ...]`) are rejected at parse
  time. The `async for` *statement* is supported.
- There is no `__await__` protocol. Awaitables are only the things Monty
  knows internally: coroutines from `async def`, async generators, gather
  futures, and external function call futures returned by host bindings.

## `async for` and `async with`

- `async for` drives `__aiter__` / `__anext__` and ends on
  `StopAsyncIteration`; `async with` drives `__aenter__` / `__aexit__` and
  awaits both. Both are ordinary attribute calls, so they work on any class
  that defines the methods.
- **`async for` over a non-async-iterable reports the wrong exception.**
  Monty raises `AttributeError: 'list' object has no attribute '__aiter__'`
  where CPython raises `TypeError: 'async for' requires an object with
  __aiter__ method, got list`. The type is different, not only the wording,
  so `except TypeError` does not catch it.
- `__aexit__` receives `(type(exc), exc, None)`: Monty has no traceback
  objects, so the third argument is always `None`.

## Async generators

An `async def` whose body contains `yield` is an async generator, driven by
`async for`. `await` inside the body works. Beyond that:

- **No `asend()` / `athrow()` / `aclose()`.** Only `__aiter__` / `__anext__`
  exist, so an async generator cannot be resumed with a value, have an
  exception thrown in, or be closed early. A `finally` in its body runs only
  when the body itself finishes or raises.
- `__anext__()` returns the async generator itself rather than CPython's
  `async_generator_asend` object. Awaiting the result of `__anext__()` twice
  therefore advances the generator twice instead of replaying one step.
- `agen.__anext__` is the only attribute; `ag_running`, `ag_frame`,
  `ag_code` and the rest are absent.
- `type(agen).__name__` is `'async_generator'`, and `repr()` omits CPython's
  memory address: `<async_generator object ticks>`.

## Concurrency model

Concurrency is cooperative and host-driven. `gather` suspends Monty whenever
every branch is blocked on an external call, hands the pending calls to the
host, and resumes when the host returns results. There is no preemption, no
threads, and no in-sandbox scheduler.

### A failing `gather` cancels its siblings

When one child of a `gather` raises, every sibling still running is cancelled where it is blocked and never resumes.
That includes the tasks of any gather a sibling was itself awaiting.
CPython leaves those siblings running as tasks on the loop, so:

```python
async def worker():
    for _ in range(3):
        await asyncio.gather(step())
    done.append('finished')
```

appends `'finished'` under CPython after a sibling of `worker()` raises, but not under Monty.

External calls already passed to the host are not cancelled.
The host still resolves them and the results are discarded.
