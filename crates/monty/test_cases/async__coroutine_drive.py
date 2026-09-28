# A coroutine is a saved frame, as a generator is, so the same three methods drive it by hand.


async def gives(value):
    return value


async def waits_for(value):
    return await gives(value)


async def raises():
    raise ValueError('from the body')


SPENT = 'cannot reuse already awaited coroutine'

# === send runs the body and ends it ===
c = gives(1)
try:
    c.send(None)
    assert False, 'expected StopIteration'
except StopIteration:
    pass

# === a coroutine that is over is spent, where a generator is only exhausted ===
try:
    c.send(None)
    assert False, 'expected RuntimeError'
except RuntimeError as exc:
    assert str(exc) == SPENT
try:
    c.throw(KeyError('k'))
    assert False, 'expected RuntimeError'
except RuntimeError as exc:
    assert str(exc) == SPENT
assert c.close() is None

# === send drives a coroutine that awaits another one ===
c = waits_for(2)
try:
    c.send(None)
    assert False, 'expected StopIteration'
except StopIteration:
    pass

# === what the body raises comes out of send ===
c = raises()
try:
    c.send(None)
    assert False, 'expected ValueError'
except ValueError as exc:
    assert str(exc) == 'from the body'
try:
    c.send(None)
    assert False, 'expected RuntimeError'
except RuntimeError as exc:
    assert str(exc) == SPENT

# === close ends one that never ran, and says nothing ===
c = gives(3)
assert c.close() is None
try:
    c.send(None)
    assert False, 'expected RuntimeError'
except RuntimeError as exc:
    assert str(exc) == SPENT

# === throw raises where it was asked, and leaves the coroutine spent ===
c = gives(4)
try:
    c.throw(KeyError('thrown'))
    assert False, 'expected KeyError'
except KeyError as exc:
    assert str(exc) == "'thrown'"
try:
    c.send(None)
    assert False, 'expected RuntimeError'
except RuntimeError as exc:
    assert str(exc) == SPENT

# === a value sent to one that has not started names the coroutine ===
c = gives(5)
try:
    c.send(1)
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "can't send non-None value to a just-started coroutine"
c.close()

# === a coroutine is no iterator ===
c = gives(6)
try:
    iter(c)  # pyright: ignore
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "'coroutine' object is not iterable"
try:
    next(c)  # pyright: ignore
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "'coroutine' object is not an iterator"
c.close()

# === the type and the repr name it a coroutine ===
c = gives(7)
assert type(c).__name__ == 'coroutine'
assert repr(c).startswith('<coroutine object gives at 0x')
c.close()
