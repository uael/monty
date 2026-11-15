# A program runs its own loop: once it sets it as the running loop, `create_task` and `gather` ask that loop for
# its tasks and futures, as CPython's do, and the program steps each task by hand.
import asyncio


class Future:
    _asyncio_future_blocking = False
    _log_destroy_pending = False
    _cancel_message = None

    def __init__(self, loop):
        self._loop = loop
        self.state = ''
        self.value = None
        self.callbacks = []

    def get_loop(self):
        return self._loop

    def done(self):
        return self.state != ''

    def cancelled(self):
        return self.state == 'cancelled'

    def _make_cancelled_error(self):
        return asyncio.CancelledError()

    def result(self):
        if self.exception() is not None:
            raise self.value
        return self.value

    def exception(self):
        if self.state == 'cancelled':
            raise self._make_cancelled_error()
        return self.value if self.state == 'raised' else None

    def add_done_callback(self, fn, context=None):
        if self.done():
            self._loop.call_soon(fn, self)
        else:
            self.callbacks.append(fn)

    def set_result(self, value):
        self.settle('result', value)

    def set_exception(self, raised):
        self.settle('raised', raised)

    def cancel(self, msg=None):
        if self.done():
            return False
        self.settle('cancelled', None)
        return True

    def settle(self, state, value):
        self.state = state
        self.value = value
        for fn in self.callbacks:
            self._loop.call_soon(fn, self)
        self.callbacks = []

    def __await__(self):
        if not self.done():
            self._asyncio_future_blocking = True
            yield self
        return self.result()


class Task(Future):
    def __init__(self, loop, awaitable):
        Future.__init__(self, loop)
        self.gave = None
        self.coro = self.ran(awaitable)
        loop.call_soon(self.step, None)

    async def ran(self, awaitable):
        # A StopIteration carries no value here, so the task keeps the value of what it awaited.
        self.gave = await awaitable

    def step(self, given):
        try:
            got = self.coro.throw(given) if isinstance(given, BaseException) else self.coro.send(given)
        except StopIteration:
            self.set_result(self.gave)
            return
        except Exception as raised:
            self.set_exception(raised)
            return
        if isinstance(got, str):
            # The answer that the program gives a plain yield, as a host answers a call.
            self._loop.call_soon(self.step, got.upper())
        else:
            got._asyncio_future_blocking = False
            got.add_done_callback(lambda done: self.step(None))


class Loop:
    def __init__(self):
        self.ready = []
        self.ran = []

    def get_debug(self):
        return False

    def call_soon(self, callback, *args, context=None):
        self.ready.append((callback, args))

    def create_future(self):
        return Future(self)

    def create_task(self, coro, name=None, context=None):
        return Task(self, coro)

    def run(self, coro):
        before = asyncio._get_running_loop()
        asyncio._set_running_loop(self)
        try:
            main = self.create_task(coro)
            while self.ready:
                callback, args = self.ready.pop(0)
                callback(*args)
            return main.result()
        finally:
            asyncio._set_running_loop(before)


class Asked:
    def __init__(self, word):
        self.word = word

    def __await__(self):
        return (yield self.word)


async def ask(word, order):
    order.append(word)
    got = await Asked(word)
    order.append(got)
    return got


async def fails():
    raise ValueError('a child failed')


# === no loop runs until a program sets one ===
assert asyncio._get_running_loop() is None

# === gather asks the running loop for a task of each coroutine, and the tasks interleave ===
order = []


async def both():
    return await asyncio.gather(ask('a', order), ask('b', order))


loop = Loop()
assert loop.run(both()) == ['A', 'B']
assert order == ['a', 'b', 'A', 'B']
assert asyncio._get_running_loop() is None

# === create_task makes a task of the running loop, which get_running_loop gives ===
order = []


async def spawns():
    assert asyncio.get_running_loop() is loop
    task = asyncio.create_task(ask('c', order))
    order.append('spawned')
    return await task


assert loop.run(spawns()) == 'C'
assert order == ['spawned', 'c', 'C']

# === gather takes an object that awaits, a future of the loop, and no awaitable at all ===
order = []


async def mixed():
    ready = loop.create_future()
    ready.set_result('ready')
    return await asyncio.gather(Asked('d'), ready)


assert loop.run(mixed()) == ['D', 'ready']


async def empty():
    return await asyncio.gather()


assert loop.run(empty()) == []

# === the first exception of a child reaches the awaiter of gather ===
order = []


async def first_fails():
    try:
        await asyncio.gather(ask('e', order), fails())
    except ValueError as exc:
        return str(exc)


assert loop.run(first_fails()) == 'a child failed'

# === a child that fails after a sibling ended still reaches the awaiter of gather ===
async def gives():
    return 1


async def second_fails():
    try:
        await asyncio.gather(gives(), fails())
    except ValueError as exc:
        return str(exc)


assert loop.run(second_fails()) == 'a child failed'

# === create_task with no running loop raises ===
try:
    asyncio.create_task(ask('f', order))
    assert False, 'expected RuntimeError'
except RuntimeError:
    pass
