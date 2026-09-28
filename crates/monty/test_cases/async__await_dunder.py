# An object whose class defines `__await__` is awaitable, and what that generator
# yields travels out of the coroutine to whoever drives its frame.

said = []


class Act(str):
    def __await__(self):
        return (yield self)


class Nothing:
    def __await__(self):
        if False:
            yield


class Plain:
    pass


async def word():
    got = await Act('act://one')
    said.append(got)
    said.append(await Nothing())


# === the yield travels out, and what is sent in is what the await gives ===
c = word()
out = c.send(None)
assert out == 'act://one'
assert type(out).__name__ == 'Act'
try:
    c.send('answered')
    assert False, 'expected StopIteration'
except StopIteration:
    pass
assert said == ['answered', None]


# === a class with no __await__ is not awaitable ===
async def bad():
    await Plain()  # pyright: ignore


c = bad()
try:
    c.send(None)
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "'Plain' object can't be awaited"


# === a generator is not awaitable either ===
def counts():
    yield 1


async def worse():
    await counts()  # pyright: ignore


c = worse()
try:
    c.send(None)
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "'generator' object can't be awaited"
