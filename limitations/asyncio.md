# `asyncio` module and `async` / `await`

The event loop is inside the sandbox. `asyncio` names mean what they mean in
CPython: coroutines, futures and tasks are the same objects, `await` drives the
same protocol, and the coordination primitives hand out futures the loop
settles. The host is still what answers external calls, but it is no longer the
scheduler.

## Time is the scheduler's, never the host's

A run has to replay identically, so the clock `sleep`, `timeout` and
`timeout_at` measure is a counter the scheduler owns, not the host's. It moves
only when nothing else can happen: no task is ready, and no external call is
outstanding. Two consequences, both deliberate:

- **An external call costs zero scheduler time.** A `timeout` wrapped around a
  host call never fires however long the host takes, and a `sleep` racing one
  always loses. Pricing host time into the sandbox clock is the one thing that
  would make an interleaving depend on how fast the machine is.
- **A program with nothing left to wait for says so.** When every task is parked
  and no timer or external call can wake any of them, the run raises
  `RuntimeError: every task is waiting and nothing can wake them` instead of
  sitting there, which is what CPython's loop would do.

`loop.time()` is not reachable: there is no loop object (see below).

## Module surface

Implemented, with CPython's behaviour:

- `run`, `sleep`, `gather` (including `return_exceptions=`), `ensure_future`,
  `create_task`, `wait`, `as_completed`, `current_task`, `iscoroutine`,
  `isfuture`, `timeout`, `timeout_at`
- `Future`, `Task`, `Lock`, `Event`, `Semaphore`, `BoundedSemaphore`, `Barrier`,
  `Queue`, `TaskGroup`
- `CancelledError`, `InvalidStateError`, `QueueEmpty`, `QueueFull`, and
  `TimeoutError` (which is the builtin one, as it has been since 3.11)
- `FIRST_COMPLETED`, `FIRST_EXCEPTION`, `ALL_COMPLETED`

Not implemented (raise `AttributeError`):

`get_event_loop`, `get_running_loop`, `new_event_loop`, `set_event_loop`,
`Runner`, `wait_for`, `shield`, `to_thread`, `run_coroutine_threadsafe`,
`all_tasks`, `Condition`, `LifoQueue`, `PriorityQueue`, `BrokenBarrierError`,
`IncompleteReadError`, and the whole `asyncio.subprocess` / `streams` /
`protocols` / `transports` surface.

**There is no loop object.** The scheduler is the loop, and it has no Python
face, so anything reached through `loop.` is absent. What those calls are
normally used for is a module function instead: `create_task` for
`loop.create_task`, `Future()` for `loop.create_future`, `sleep` for
`loop.call_later`.

## `async def` / `await`

- `await` drives the full protocol: coroutines, futures, tasks, async
  generators, and any object with `__await__`. A `__await__` that returns a
  generator is driven step by step, so `yield from other.__await__()` inside one
  works, which is how a hand-written awaitable delegates.
- `coro.__await__()` returns the coroutine itself where CPython returns a
  `coroutine_wrapper`. The only visible difference is the type name.
- A `__await__` generator that bare-`yield`s a value hands that value to `await`,
  which waits on it. CPython's loop expects a future there and reports
  `RuntimeError: Task got bad yield` for anything else; here a non-awaitable
  raises `TypeError: 'X' object can't be awaited` from the same place.
- **Coroutines are single-shot.** Awaiting the same coroutine object twice
  raises `RuntimeError`. Store the *result*, not the coroutine.
- `asyncio.run(coro)` awaits the coroutine on the task that called it rather
  than starting a loop, since one is already running. Module-level code counts
  as a task: `current_task()` answers there, and `asyncio.timeout` can cancel it.
- Async comprehensions (`[x async for x in ...]`) are rejected at parse time.
  The `async for` *statement* is supported.

## Cancellation

- `CancelledError` derives from `BaseException`, so `except Exception:` inside a
  cancelled coroutine does not swallow it, and a `finally` still runs.
- `task.cancel()` returns `False` for a settled task and `True` otherwise. A
  parked task is cancelled through the future it waits on, exactly as CPython
  cancels `_fut_waiter`, so a shared future's other waiters see the
  cancellation too. A running task remembers the request and raises at its next
  suspension. A task cancelled before it starts never runs a line.
- A coroutine that catches `CancelledError` and returns normally completes
  normally: `cancelled()` is then `False`, as in CPython.
- `cancel(msg)` carries the message: `str(msg)` becomes the `CancelledError`'s
  own message.
- `cancelling()` and `uncancel()` count requests, but nothing consumes the
  count: they do not drive `TaskGroup`'s re-raise the way CPython's do.
- An exception nobody retrieves from a task is kept on its future and never
  reported. CPython logs "Task exception was never retrieved" at collection;
  there is no such hook here.

## Divergences in the primitives

- **`acquire()` and its relatives act at the call, not at the `await`.**
  CPython's `Lock.acquire()`, `Semaphore.acquire()`, `Event.wait()`,
  `Queue.get()` and `Queue.put()` return coroutines that do nothing until
  awaited; here they take effect immediately and return a future. Awaiting them
  behaves identically, which is what every ordinary use does; calling one and
  discarding the result does not.
- `Lock.release()` hands the lock straight to the next waiter rather than
  releasing it and letting the woken task re-take it. A queued acquirer
  therefore cannot be overtaken, which CPython also guarantees, but the
  intermediate state where `locked()` is briefly `False` does not exist.
- `Barrier` supports `wait`, `parties` and `n_waiting`. `abort`, `reset`,
  `broken` and `BrokenBarrierError` are absent.
- `Queue` supports `put`, `get`, `put_nowait`, `get_nowait`, `qsize`, `empty`,
  `full`, `maxsize`, `task_done` and `join`. `shutdown` is absent.
- `TaskGroup` cancels its remaining children as soon as one raises and re-raises
  **that first exception directly**. CPython collects them into a
  `BaseExceptionGroup`; Monty has no exception groups and no `except*`, so
  there is nothing to collect into.
- `asyncio.wait()` rejects a `timeout=` argument with `NotImplementedError`
  rather than ignoring it. Wrap the wait in `asyncio.timeout()` instead. It
  also accepts bare coroutines, which CPython has refused since 3.12; passing
  tasks is still what you want, since the set it hands back is of the futures
  it wrapped them in.
- `as_completed(fs)` supports the async-iterator form
  (`async for fut in as_completed(fs)`), and yields the futures it wrapped the
  awaitables in rather than the original awaitables. The legacy synchronous
  iterator form is absent.
- `repr` omits what CPython draws from an address or a frame: a future reads
  `<Future pending>` / `<Future finished result=1>` / `<Task cancelled>`, and
  the primitives read `<Lock>`, `<Queue>` and so on rather than CPython's
  `<asyncio.locks.Lock object at 0x... [unlocked]>`.
- `Task` answers `get_name`, `set_name`, `get_coro`, `cancelling` and
  `uncancel`; the default name is `Task-<n>` numbered by the scheduler, so it
  need not match CPython's numbering. `Future.get_loop()` is absent, there
  being no loop object.

## `async for` and `async with`

- `async for` drives `__aiter__` / `__anext__` and ends on
  `StopAsyncIteration`; `async with` drives `__aenter__` / `__aexit__` and
  awaits both. Both are ordinary attribute calls, so they work on any class
  that defines the methods, and on anything awaitable those methods return.
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

Concurrency is cooperative and single-threaded, with no preemption. Tasks run
in the order they became ready; a task runs until it awaits something that is
not already settled. When every task is parked on an external call, the
sandbox suspends and hands the pending calls to the host, resuming when it
answers. A suspended loop, timers included, survives a dump and reload.
