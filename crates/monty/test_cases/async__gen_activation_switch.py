# call-external
# run-async
# A generator step in flight across a task switch: the activation must travel
# with the task, not stay behind on the VM.
import asyncio


async def ticks(n):
    for i in range(n):
        yield await async_call(i)  # pyright: ignore


async def consume(n):
    out = []
    async for value in ticks(n):
        out.append(value)
    return out


got = await asyncio.gather(consume(3), async_call('other'), consume(2))  # pyright: ignore
assert got == [[0, 1, 2], 'other', [0, 1]], got
