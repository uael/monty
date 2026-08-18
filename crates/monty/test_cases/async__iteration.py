# run-async
# `async for`, `async with`, and async generators.


class Ticker:
    """Async iterator over 1..n, the `__aiter__`/`__anext__` pair by hand."""

    def __init__(self, n):
        self.n = n
        self.i = 0

    def __aiter__(self):
        return self

    async def __anext__(self):
        if self.i >= self.n:
            raise StopAsyncIteration
        self.i += 1
        return self.i


class Lock:
    """Async context manager recording whether it is held."""

    def __init__(self, suppress=False):
        self.held = False
        self.suppress = suppress
        self.seen = None

    async def __aenter__(self):
        self.held = True
        return self

    async def __aexit__(self, exc_type, exc, tb):
        self.held = False
        self.seen = exc_type
        return self.suppress


# === async for ===
seen = []
async for value in Ticker(3):
    seen.append(value)
assert seen == [1, 2, 3]

# An empty async iterator runs the body zero times.
seen = []
async for value in Ticker(0):
    seen.append(value)
assert seen == []

# `break` and `else` behave as in a sync `for`.
seen = []
async for value in Ticker(5):
    if value == 3:
        break
    seen.append(value)
else:
    seen.append('no-break')
assert seen == [1, 2]

seen = []
async for value in Ticker(2):
    seen.append(value)
else:
    seen.append('no-break')
assert seen == [1, 2, 'no-break']

# `continue` skips the rest of the body.
seen = []
async for value in Ticker(4):
    if value % 2 == 0:
        continue
    seen.append(value)
assert seen == [1, 3]

# An exception from the body escapes the loop rather than ending it.
try:
    async for value in Ticker(3):
        raise ValueError('body')
    assert False, 'expected ValueError'
except ValueError as exc:
    assert str(exc) == 'body'

# A `StopAsyncIteration` raised by the *body* is not the loop's own signal.
try:
    async for value in Ticker(3):
        raise StopAsyncIteration
    assert False, 'expected StopAsyncIteration'
except StopAsyncIteration:
    pass

# Nested loops each keep their own iterator.
pairs = []
async for outer in Ticker(2):
    async for inner in Ticker(2):
        pairs.append((outer, inner))
assert pairs == [(1, 1), (1, 2), (2, 1), (2, 2)]

# === async with ===
lock = Lock()
async with lock as held:
    assert held is lock
    assert lock.held
assert not lock.held
assert lock.seen is None

# Without a target the manager still enters and exits.
lock = Lock()
async with lock:
    assert lock.held
assert not lock.held

# An exception reaches `__aexit__` and, unsuppressed, keeps propagating.
lock = Lock()
try:
    async with lock:
        raise ValueError('inner')
    assert False, 'expected ValueError'
except ValueError as exc:
    assert str(exc) == 'inner'
assert not lock.held
assert lock.seen is ValueError

# A truthy `__aexit__` swallows it.
lock = Lock(suppress=True)
async with lock:
    raise ValueError('swallowed')
assert not lock.held
assert lock.seen is ValueError

# Nested managers exit inside-out.
order = []


class Tracked:
    def __init__(self, name):
        self.name = name

    async def __aenter__(self):
        order.append('enter ' + self.name)
        return self

    async def __aexit__(self, exc_type, exc, tb):
        order.append('exit ' + self.name)
        return False


async with Tracked('a'):
    async with Tracked('b'):
        order.append('body')
assert order == ['enter a', 'enter b', 'body', 'exit b', 'exit a']


# === async generators ===
async def ticks(n):
    i = 0
    while i < n:
        yield i
        i += 1


agen = ticks(3)
assert type(agen).__name__ == 'async_generator'

seen = []
async for value in ticks(3):
    seen.append(value)
assert seen == [0, 1, 2]

seen = []
async for value in ticks(0):
    seen.append(value)
assert seen == []


# `await` inside an async generator body works: the generator's frame is on
# the VM's own stack, so awaiting suspends it like any other frame.
async def double(x):
    return x * 2


async def doubled(n):
    i = 0
    while i < n:
        yield await double(i)
        i += 1


seen = []
async for value in doubled(3):
    seen.append(value)
assert seen == [0, 2, 4]


# `return` ends an async generator, as does running off the end.
async def early(n):
    for i in range(n):
        if i == 2:
            return
        yield i


seen = []
async for value in early(5):
    seen.append(value)
assert seen == [0, 1]


# An exception in the body reaches the consumer.
async def failing():
    yield 1
    raise ValueError('agen')


seen = []
try:
    async for value in failing():
        seen.append(value)
    assert False, 'expected ValueError'
except ValueError as exc:
    assert str(exc) == 'agen'
assert seen == [1]


# An async generator closes over its enclosing scope like any function.
async def scaled(factor):
    for i in [1, 2, 3]:
        yield i * factor


seen = []
async for value in scaled(10):
    seen.append(value)
assert seen == [10, 20, 30]


# A plain `for` over an async generator is a TypeError: it is not an iterator.
try:
    for value in ticks(3):
        pass
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "'async_generator' object is not iterable"
