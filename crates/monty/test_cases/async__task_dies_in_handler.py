# run-async
# A task that dies while one of its `except` handlers is active leaves an entry
# on the exception stack. That stack belongs to the dying task, so it has to go
# with the task's frames; the one switched in next must not inherit it.
import asyncio


async def rethrows():
    try:
        raise ValueError('boom')
    except ValueError:
        raise


async def swallows():
    try:
        raise KeyError('inner')
    except KeyError:
        raise RuntimeError('from the handler')


async def waits():
    try:
        await asyncio.gather(rethrows())
        assert False, 'expected ValueError'
    except ValueError as exc:
        return str(exc)


assert await waits() == 'boom'  # pyright: ignore

try:
    await asyncio.gather(swallows())  # pyright: ignore
    assert False, 'expected RuntimeError'
except RuntimeError as exc:
    assert str(exc) == 'from the handler'

# The next raise sees no leftovers from either dead task.
try:
    raise IndexError('clean')
except IndexError as exc:
    assert str(exc) == 'clean'
