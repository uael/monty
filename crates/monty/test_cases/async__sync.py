# run-async
# The coordination primitives: Lock, Event, Semaphore, Barrier and Queue.
import asyncio

# === Lock: one holder, the rest queued in arrival order ===
lock = asyncio.Lock()
assert not lock.locked()
held = []


async def guarded(name):
    async with lock:
        held.append((name, 'in'))
        await asyncio.sleep(0)
        held.append((name, 'out'))


await asyncio.gather(guarded('a'), guarded('b'))  # pyright: ignore
assert held == [('a', 'in'), ('a', 'out'), ('b', 'in'), ('b', 'out')], held
assert not lock.locked()

await lock.acquire()  # pyright: ignore
assert lock.locked()
lock.release()
assert not lock.locked()
try:
    lock.release()
    assert False, 'expected RuntimeError'
except RuntimeError as exc:
    assert str(exc) == 'Lock is not acquired.'

# === Event: every waiter is released by one set ===
event = asyncio.Event()
assert not event.is_set()
woke = []


async def waiter(name):
    await event.wait()
    woke.append(name)


async def raiser():
    await asyncio.sleep(0)
    event.set()


await asyncio.gather(waiter('x'), waiter('y'), raiser())  # pyright: ignore
assert woke == ['x', 'y'], woke
assert event.is_set()
assert await event.wait() is True  # pyright: ignore
event.clear()
assert not event.is_set()

# === Semaphore: at most `value` holders at once ===
sem = asyncio.Semaphore(2)
inside = []
peak = [0]
live = [0]


async def limited(name):
    async with sem:
        live[0] += 1
        peak[0] = max(peak[0], live[0])
        inside.append(name)
        await asyncio.sleep(0)
        live[0] -= 1


await asyncio.gather(*[limited(i) for i in range(5)])  # pyright: ignore
assert peak[0] == 2, peak
assert inside == [0, 1, 2, 3, 4], inside

bounded = asyncio.BoundedSemaphore(1)
await bounded.acquire()  # pyright: ignore
bounded.release()
try:
    bounded.release()
    assert False, 'expected ValueError'
except ValueError as exc:
    assert str(exc) == 'BoundedSemaphore released too many times'

# === Barrier: nobody passes until every party arrives ===
barrier = asyncio.Barrier(3)
assert barrier.parties == 3
passed = []


async def party(name):
    index = await barrier.wait()
    passed.append((name, index))


await asyncio.gather(party('p'), party('q'), party('r'))  # pyright: ignore
assert sorted(index for _, index in passed) == [0, 1, 2], passed
assert barrier.n_waiting == 0

# === Queue: put, get, and the empty and full behaviours ===
queue = asyncio.Queue()
assert queue.empty()
assert queue.qsize() == 0
queue.put_nowait('first')
assert queue.qsize() == 1
assert not queue.empty()
assert queue.get_nowait() == 'first'
try:
    queue.get_nowait()
    assert False, 'expected QueueEmpty'
except asyncio.QueueEmpty:
    pass

small = asyncio.Queue(1)
assert small.maxsize == 1
small.put_nowait('only')
assert small.full()
try:
    small.put_nowait('overflow')
    assert False, 'expected QueueFull'
except asyncio.QueueFull:
    pass

# A blocked `put` is released as soon as a `get` makes room.
moved = []


async def producer():
    for item in ('a', 'b', 'c'):
        await small.put(item)
        moved.append(('put', item))


async def consumer():
    got = []
    for _ in range(4):
        got.append(await small.get())
        small.task_done()
    return got


producing = asyncio.ensure_future(producer())
consuming = asyncio.ensure_future(consumer())
await producing  # pyright: ignore
assert await consuming == ['only', 'a', 'b', 'c']  # pyright: ignore
assert moved == [('put', 'a'), ('put', 'b'), ('put', 'c')], moved

# `join` waits for every item to be marked done.
work = asyncio.Queue()
finished = []


async def worker():
    while True:
        item = await work.get()
        finished.append(item)
        work.task_done()


runner = asyncio.ensure_future(worker())
for item in range(3):
    work.put_nowait(item)
await work.join()  # pyright: ignore
assert finished == [0, 1, 2], finished
runner.cancel()

try:
    work.task_done()
    assert False, 'expected ValueError'
except ValueError as exc:
    assert str(exc) == 'task_done() called too many times'
