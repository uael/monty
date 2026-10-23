# `yield from` hands the whole of another iterator on, and gives back what it returned.


def inner(n):
    i = 0
    while i < n:
        yield i
        i += 1
    return 'inner ' + str(n)


def outer(n):
    got = yield from inner(n)
    yield got


# === every value of the receiver comes out of the waiter ===
assert list(outer(3)) == [0, 1, 2, 'inner 3']
assert list(outer(0)) == ['inner 0']


# === an ordinary iterable is delegated to as well ===
def over_iterables():
    yield from [1, 2]
    yield from 'ab'
    yield from range(2)
    yield from {'k': 'v'}


assert list(over_iterables()) == [1, 2, 'a', 'b', 0, 1, 'k']


# === a sent value reaches the receiver, and its return value the waiter ===
def totals():
    total = 0
    while True:
        got = yield total
        if got is None:
            return total
        total += got


def sums():
    total = yield from totals()
    yield 'sum ' + str(total)


s = sums()
assert next(s) == 0
assert s.send(5) == 5
assert s.send(3) == 8
assert s.send(None) == 'sum 8'


# === delegation nests ===
def one():
    yield 'a'
    return 'one'


def two():
    got = yield from one()
    yield got
    return 'two'


def three():
    got = yield from two()
    yield got


assert list(three()) == ['a', 'one', 'two']


# === the receiver is what raises, and the waiter may catch it ===
def raises():
    yield 1
    raise ValueError('from the receiver')


def catches():
    try:
        yield from raises()
    except ValueError as exc:
        yield 'caught ' + str(exc)
    yield 'after'


assert list(catches()) == [1, 'caught from the receiver', 'after']


# === throw() reaches the receiver, where the value is suspended ===
def handles():
    try:
        yield 1
    except KeyError as exc:
        yield 'receiver caught ' + str(exc)
    return 'handled'


def waits():
    got = yield from handles()
    yield got


w = waits()
assert next(w) == 1
assert w.throw(KeyError('k')) == "receiver caught 'k'"
assert next(w) == 'handled'


# === a receiver that returns while the thrown exception is handled ===
def recovers():
    try:
        yield 1
    except ValueError:
        return 'recovered'


def above():
    got = yield from recovers()
    yield got


a = above()
assert next(a) == 1
assert a.throw(ValueError('v')) == 'recovered'


# === close() reaches the receiver first, so the innermost finally runs first ===
order = []


def innermost():
    try:
        yield 'deep'
    finally:
        order.append('innermost')


def middle():
    try:
        yield from innermost()
    finally:
        order.append('middle')


def outermost():
    try:
        yield from middle()
    finally:
        order.append('outermost')


k = outermost()
assert next(k) == 'deep'
k.close()
assert order == ['innermost', 'middle', 'outermost']


# === a receiver that is over already ends the delegation at once ===
def empty():
    for _ in []:
        yield


def uses_empty():
    got = yield from empty()
    yield repr(got)


assert list(uses_empty()) == ['None']


# === a delegation in a loop, and one whose receiver is the same generator twice ===
def repeated():
    for _ in range(2):
        yield from inner(2)


assert list(repeated()) == [0, 1, 0, 1]


# === the waiter carries on after the delegation ===
def carries_on():
    yield from inner(1)
    yield 'own'
    yield from inner(1)
    yield 'own again'


assert list(carries_on()) == [0, 'own', 0, 'own again']
