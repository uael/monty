# run-async
# The shape the sabre engine drives asyncio in: a custom awaitable whose
# `__await__` delegates to a coroutine that waits on an `Event`, jobs started
# with `ensure_future` and watched with `add_done_callback`, a `Lock` around the
# control path, and `gather` over a generator expression of live tasks.
import asyncio

log = []


class Journal:
    """An event that is replaced rather than cleared, as the engine's is."""

    def __init__(self):
        self.turn = asyncio.Event()
        self.rows = []

    def poke(self):
        was, self.turn = self.turn, asyncio.Event()
        was.set()

    async def rest(self, ready):
        while True:
            was = self.turn
            if ready():
                return
            await was.wait()

    def append(self, row):
        self.rows.append(row)
        self.poke()


journal = Journal()


class Spawned:
    """Awaitable through `__await__`, exactly like the engine's cursor."""

    def __init__(self, wanted):
        self.wanted = wanted

    def __await__(self):
        def over():
            return len(journal.rows) >= self.wanted

        yield from journal.rest(over).__await__()
        return journal.rows[self.wanted - 1]


lock = asyncio.Lock()
jobs = {}
ended = []


async def produce(name, count):
    async with lock:
        for i in range(count):
            journal.append((name, i))
            await asyncio.sleep(0)
    return name


def flying(name):
    job = jobs.get(name)
    return job if job is not None and not job.done() else None


def watch(job):
    if not job.cancelled() and isinstance(job.exception(), Exception):
        ended.append(('failed', job.exception()))
    else:
        ended.append(('done', job.result()))


for name in ('a', 'b'):
    jobs[name] = asyncio.ensure_future(produce(name, 2))
    jobs[name].add_done_callback(watch)

# The custom awaitable suspends until the journal has grown enough.
assert await Spawned(3) == ('b', 0)  # pyright: ignore

# `gather` over a generator expression of the still-running jobs, the exact
# call the engine's drain makes.
await asyncio.gather(*(job for name in jobs if (job := flying(name)) is not None), return_exceptions=True)  # pyright: ignore

assert journal.rows == [('a', 0), ('a', 1), ('b', 0), ('b', 1)], journal.rows
assert not lock.locked()

# Done callbacks have run by the time the loop next idles.
await asyncio.sleep(0)  # pyright: ignore
assert ended == [('done', 'a'), ('done', 'b')], ended

log.append('finished')
assert log == ['finished']
