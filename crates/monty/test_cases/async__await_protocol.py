# run-async
# The `__await__` protocol: any object with `__await__` is awaitable, and a
# generator returned from it can delegate to a coroutine with `yield from`.


async def double(n):
    return n * 2


class Plain:
    """`__await__` returning a generator that never suspends."""

    def __init__(self, n):
        self.n = n

    def __await__(self):
        yield from ()
        return self.n + 1


class Delegating:
    """`__await__` delegating to a coroutine, the shape `asyncio` primitives use."""

    def __init__(self, n):
        self.n = n

    def __await__(self):
        got = yield from double(self.n).__await__()
        return got + 1


class Reraising:
    def __await__(self):
        raise ValueError('from __await__')
        yield  # pyright: ignore


# === a bare __await__ generator ===
assert await Plain(5) == 6  # pyright: ignore

# === delegation to a coroutine ===
assert await Delegating(5) == 11  # pyright: ignore

# === the same object is awaitable more than once ===
twice = Delegating(1)
assert await twice == 3  # pyright: ignore
assert await twice == 3  # pyright: ignore

# === an exception raised inside __await__ reaches the await site ===
try:
    await Reraising()  # pyright: ignore
    assert False, 'expected ValueError'
except ValueError as exc:
    assert str(exc) == 'from __await__'

# === __await__ inside a coroutine, and awaited coroutines nest ===


async def uses(n):
    return await Delegating(n)


assert await uses(3) == 7  # pyright: ignore

# === a coroutine is its own __await__ iterator ===


async def drive():
    return await double(4)


assert await drive() == 8  # pyright: ignore

# === objects with no __await__ are rejected ===
try:
    await 3  # pyright: ignore
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "'int' object can't be awaited"


class NoAwait:
    pass


try:
    await NoAwait()  # pyright: ignore
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "'NoAwait' object can't be awaited"


class BadAwait:
    def __await__(self):
        return 42


try:
    await BadAwait()  # pyright: ignore
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "__await__() returned non-iterator of type 'int'"
