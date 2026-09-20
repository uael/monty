# A function whose body yields hands back a generator, resumed a value at a time.


def counter(n):
    i = 0
    while i < n:
        got = yield i
        i = got if got is not None else i + 1


# === it is a generator, and its own iterator ===
g = counter(3)
assert type(g).__name__ == 'generator'
assert iter(g) is g
assert repr(g).startswith('<generator object counter at 0x')

# === next() steps it, and exhaustion raises StopIteration ===
assert next(g) == 0
assert next(g) == 1
assert next(g) == 2
try:
    next(g)
    raise AssertionError('expected StopIteration')
except StopIteration:
    pass

# === and again after it is exhausted ===
try:
    next(g)
    raise AssertionError('expected StopIteration')
except StopIteration:
    pass


# === a send() that runs the body out raises StopIteration at the caller ===
def two():
    got = yield 1
    if got:
        return
    yield 2


e = two()
assert next(e) == 1
try:
    e.send('stop')
    raise AssertionError('expected StopIteration')
except StopIteration:
    pass


# === send() resumes it with a value, which the yield evaluates to ===
s = counter(10)
assert next(s) == 0
assert s.send(5) == 5
assert next(s) == 6

# === a value cannot be sent to one that has not started ===
try:
    counter(3).send(1)
    raise AssertionError('expected TypeError')
except TypeError as e:
    assert str(e) == "can't send non-None value to a just-started generator"

# === every way of walking an iterator reaches it ===
assert list(counter(3)) == [0, 1, 2]
assert tuple(counter(3)) == (0, 1, 2)
assert sum(counter(4)) == 6
assert [x * 2 for x in counter(3)] == [0, 2, 4]
assert sorted(counter(3), reverse=True) == [2, 1, 0]
assert max(counter(3)) == 2

seen = []
for x in counter(3):
    seen.append(x)
assert seen == [0, 1, 2]


# === unpacking, which is iteration too ===
def pair():
    yield 'a'
    yield 'b'


first, second = pair()
assert first == 'a'
assert second == 'b'


# === a body that can yield but never does is an empty generator ===
def never():
    if False:
        yield


assert list(never()) == []
assert type(never()).__name__ == 'generator'


# === a bare `yield` yields None ===
def bare():
    yield
    yield


assert list(bare()) == [None, None]


# === generators nest: one can drive another ===
def inner():
    yield 1
    yield 2


def outer():
    for x in inner():
        yield x * 10


assert list(outer()) == [10, 20]


# === `finally` runs where it stands when the body ends ===
log = []


def cleaned():
    try:
        yield 1
        yield 2
    finally:
        log.append('closed')


assert list(cleaned()) == [1, 2]
assert log == ['closed']


# === an exception leaves the generator exhausted, as a return does ===
def raiser():
    yield 1
    raise ValueError('boom')


r = raiser()
assert next(r) == 1
try:
    next(r)
    raise AssertionError('expected ValueError')
except ValueError as e:
    assert str(e) == 'boom'
try:
    next(r)
    raise AssertionError('expected StopIteration')
except StopIteration:
    pass


# === arguments and closures are bound at the call, not at the first resume ===
def closed_over(base):
    step = 10

    def helper(x):
        return base + x * step

    yield helper(1)
    yield helper(2)


assert list(closed_over(100)) == [110, 120]


# === two generators from one function run independently ===
a, b = counter(3), counter(3)
assert next(a) == 0
assert next(a) == 1
assert next(b) == 0
assert next(a) == 2
assert next(b) == 1
