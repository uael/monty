# run-async
# Tasks, futures and the loop that runs them.
import asyncio

order = []


async def step(name, times):
    for i in range(times):
        order.append((name, i))
        await asyncio.sleep(0)
    return name


# === ensure_future runs a coroutine concurrently with the awaiting task ===
a = asyncio.ensure_future(step('a', 3))
b = asyncio.ensure_future(step('b', 3))
assert await asyncio.gather(a, b) == ['a', 'b']  # pyright: ignore
assert order == [('a', 0), ('b', 0), ('a', 1), ('b', 1), ('a', 2), ('b', 2)], order

# === a settled task replays its result to every later await ===
assert a.done()
assert not a.cancelled()
assert a.result() == 'a'
assert a.exception() is None
assert await a == 'a'  # pyright: ignore

# === create_task is the same thing under its other name ===
order.clear()
c = asyncio.create_task(step('c', 1))
assert await c == 'c'  # pyright: ignore

# === a bare Future is settled by whoever holds it ===
fut = asyncio.Future()
assert not fut.done()
fut.set_result(5)
assert fut.done()
assert await fut == 5  # pyright: ignore
try:
    fut.set_result(6)
    assert False, 'expected InvalidStateError'
except asyncio.InvalidStateError as exc:
    assert str(exc) == 'invalid state'


async def settle_later(f, value):
    await asyncio.sleep(0)
    f.set_result(value)


pending = asyncio.Future()
asyncio.ensure_future(settle_later(pending, 'late'))
assert await pending == 'late'  # pyright: ignore

# === several tasks can await the same future ===
shared = asyncio.Future()


async def reader():
    return await shared


readers = [asyncio.ensure_future(reader()) for _ in range(3)]
asyncio.ensure_future(settle_later(shared, 'broadcast'))
assert await asyncio.gather(*readers) == ['broadcast'] * 3  # pyright: ignore

# === an exception from a task reaches the awaiting site ===


async def boom():
    raise ValueError('boom')


failing = asyncio.ensure_future(boom())
try:
    await failing  # pyright: ignore
    assert False, 'expected ValueError'
except ValueError as exc:
    assert str(exc) == 'boom'
assert failing.done()
assert isinstance(failing.exception(), ValueError)

# === gather(return_exceptions=True) collects them instead ===
collected = await asyncio.gather(boom(), step('d', 1), return_exceptions=True)  # pyright: ignore
assert isinstance(collected[0], ValueError)
assert collected[1] == 'd'

# === gather over the same awaitable twice runs it once ===
runs = []


async def once():
    runs.append(1)
    return 'once'


shared_coro = once()
assert await asyncio.gather(shared_coro, shared_coro) == ['once', 'once']  # pyright: ignore
assert runs == [1]

# === done callbacks run after the future settles ===
seen = []
notified = asyncio.ensure_future(step('e', 1))
notified.add_done_callback(lambda f: seen.append(f.result()))
await notified  # pyright: ignore
await asyncio.sleep(0)  # pyright: ignore
assert seen == ['e'], seen

# === sleep orders tasks by their deadlines, not by their start ===
timed = []


async def after(delay, name):
    await asyncio.sleep(delay)
    timed.append(name)


await asyncio.gather(after(0.03, 'slow'), after(0.01, 'quick'), after(0.02, 'middle'))  # pyright: ignore
assert timed == ['quick', 'middle', 'slow'], timed
