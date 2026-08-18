# run-async
# What the loop still holds when the module ends, and what it should have let
# go of before then.
#
# A `gather` that fails early leaves its siblings running, by design and as
# CPython does, so its combinator has to give up the slots it registered on
# them: it owns those children and they owned it back, and the pair outlived
# every use of itself. The tasks themselves are a different matter, and not a
# leak: the loop still holds them, the way the module cache holds an import.
import asyncio

order = []


async def parks():
    await asyncio.sleep(100)
    order.append('woke')


async def boom():
    raise ValueError('x')


async def finishes():
    await asyncio.sleep(0)
    order.append('finished')
    return 'ok'


# The first failure settles the gather while the survivor runs on.
try:
    await asyncio.gather(parks(), boom())  # pyright: ignore
    assert False, 'expected ValueError'
except ValueError as exc:
    assert str(exc) == 'x'

# A sibling that does finish still reports, and the result is the caller's.
try:
    await asyncio.gather(finishes(), boom())  # pyright: ignore
    assert False, 'expected ValueError'
except ValueError:
    pass
await asyncio.sleep(0)  # pyright: ignore
assert order == ['finished'], order

# A task nobody awaits is parked on its own when the module returns.
asyncio.ensure_future(parks())
await asyncio.sleep(0)  # pyright: ignore
# ref-counts={'asyncio': 2, 'order': 1}
