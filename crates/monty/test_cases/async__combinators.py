# run-async
# `wait`, `as_completed`, `TaskGroup` and `timeout`.
import asyncio


async def after(delay, value):
    await asyncio.sleep(delay)
    return value


# === wait: (done, pending) rather than results ===
quick = asyncio.ensure_future(after(0.01, 'quick'))
slow = asyncio.ensure_future(after(0.05, 'slow'))
done, pending = await asyncio.wait({quick, slow}, return_when=asyncio.FIRST_COMPLETED)  # pyright: ignore
assert done == {quick}, done
assert pending == {slow}, pending
done, pending = await asyncio.wait({quick, slow})  # pyright: ignore
assert done == {quick, slow}
assert pending == set()
assert quick.result() == 'quick'
assert slow.result() == 'slow'

# === as_completed: each awaitable back in settling order ===
order = []
async for finished in asyncio.as_completed([after(0.03, 'c'), after(0.01, 'a'), after(0.02, 'b')]):
    order.append(await finished)
assert order == ['a', 'b', 'c'], order

# === TaskGroup: the body's children all finish before the block exits ===
collected = []


async def collect(delay, value):
    await asyncio.sleep(delay)
    collected.append(value)
    return value


async with asyncio.TaskGroup() as group:
    first = group.create_task(collect(0.02, 'second'))
    second = group.create_task(collect(0.01, 'first'))
assert collected == ['first', 'second'], collected
assert first.result() == 'second'
assert second.result() == 'first'

# === timeout: a body that overruns is cancelled and raises TimeoutError ===
reached = []
try:
    async with asyncio.timeout(0.01):
        await asyncio.sleep(1)
        reached.append('body finished')
    assert False, 'expected TimeoutError'
except TimeoutError:
    pass
assert reached == [], reached

# A body that finishes in time is untouched.
async with asyncio.timeout(1):
    inside = await after(0.01, 'in time')
assert inside == 'in time'

# `timeout(None)` never fires.
async with asyncio.timeout(None):
    assert await after(0.01, 'unbounded') == 'unbounded'
