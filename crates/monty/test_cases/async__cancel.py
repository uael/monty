# run-async
# Cancellation: `CancelledError` derives from `BaseException`, a `finally` in a
# cancelled coroutine still runs, and the error surfaces at the awaiting site.
import asyncio

trace = []


async def patient(name):
    try:
        await asyncio.sleep(10)
        trace.append((name, 'returned'))
        return 'done'
    except asyncio.CancelledError:
        trace.append((name, 'caught'))
        raise
    finally:
        trace.append((name, 'finally'))


# === cancelling a parked task raises at its suspension point ===
task = asyncio.ensure_future(patient('one'))
await asyncio.sleep(0)  # pyright: ignore
assert task.cancel()
try:
    await task  # pyright: ignore
    assert False, 'expected CancelledError'
except asyncio.CancelledError:
    pass
assert trace == [('one', 'caught'), ('one', 'finally')], trace
assert task.cancelled()
assert task.done()

# === cancel on a settled task is refused ===
assert not task.cancel()

# === CancelledError is not an Exception ===
# `issubclass` on a builtin exception type is unsupported in Monty (it rejects
# every one of them, not only these), so the hierarchy is checked by catching.
caught_by_except_exception = False
try:
    raise asyncio.CancelledError
except Exception:
    caught_by_except_exception = True
except asyncio.CancelledError:
    pass
assert not caught_by_except_exception

caught_by_except_base = False
try:
    raise asyncio.CancelledError
except BaseException:
    caught_by_except_base = True
assert caught_by_except_base

# === a coroutine that swallows the cancellation still finishes normally ===
trace.clear()


async def stubborn():
    try:
        await asyncio.sleep(10)
    except asyncio.CancelledError:
        trace.append('swallowed')
    return 'survived'


survivor = asyncio.ensure_future(stubborn())
await asyncio.sleep(0)  # pyright: ignore
assert survivor.cancel()
assert await survivor == 'survived'  # pyright: ignore
assert trace == ['swallowed'], trace
assert not survivor.cancelled()

# === a task cancelled before it starts never runs a line ===
started = []


async def never():
    started.append('ran')
    return 1


unstarted = asyncio.ensure_future(never())
assert unstarted.cancel()
try:
    await unstarted  # pyright: ignore
    assert False, 'expected CancelledError'
except asyncio.CancelledError:
    pass
assert started == [], started
assert unstarted.cancelled()

# === cancelling a plain future settles it ===
fut = asyncio.Future()
assert fut.cancel()
assert fut.cancelled()
try:
    fut.result()
    assert False, 'expected CancelledError'
except asyncio.CancelledError:
    pass

# === gather(return_exceptions=True) reports a cancelled child ===


async def fine():
    return 'fine'


victim = asyncio.ensure_future(patient('two'))
await asyncio.sleep(0)  # pyright: ignore
victim.cancel()
mixed = await asyncio.gather(victim, fine(), return_exceptions=True)  # pyright: ignore
assert isinstance(mixed[0], asyncio.CancelledError)
assert mixed[1] == 'fine'

# === cancelling the task a `gather` is waiting on stops the wait ===
trace.clear()


async def waits_on_gather():
    await asyncio.gather(patient('a'), patient('b'))


outer = asyncio.ensure_future(waits_on_gather())
await asyncio.sleep(0)  # pyright: ignore
outer.cancel()
try:
    await outer  # pyright: ignore
    assert False, 'expected CancelledError'
except asyncio.CancelledError:
    pass
assert outer.cancelled()

# === cancelling() counts the requests, uncancel() takes one back ===
counted = asyncio.ensure_future(patient('three'))
await asyncio.sleep(0)  # pyright: ignore
assert counted.cancelling() == 0
counted.cancel()
assert counted.cancelling() == 1
assert counted.uncancel() == 0
try:
    await counted  # pyright: ignore
except asyncio.CancelledError:
    pass
