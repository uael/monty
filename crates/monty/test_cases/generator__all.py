# === Plain generators ===
def counter(n):
    i = 0
    while i < n:
        yield i
        i += 1


g = counter(3)
assert type(g).__name__ == 'generator'
assert next(g) == 0
assert next(g) == 1
assert next(g) == 2
try:
    next(g)
    assert False, 'expected StopIteration'
except StopIteration as exc:
    assert str(exc) == ''

# A generator is its own iterator.
g = counter(2)
assert iter(g) is g
assert g.__iter__() is g
assert g.__next__() == 0
assert next(g) == 1

# === Every iterator consumer accepts one ===
assert list(counter(4)) == [0, 1, 2, 3]
assert tuple(counter(3)) == (0, 1, 2)
assert set(counter(3)) == {0, 1, 2}
assert sum(counter(5)) == 10
assert max(counter(5)) == 4
assert min(counter(5)) == 0
assert sorted(counter(3), reverse=True) == [2, 1, 0]
assert any(x > 1 for x in counter(3))
assert not all(x > 1 for x in counter(3))
assert [x for x in counter(3)] == [0, 1, 2]
assert [*counter(3)] == [0, 1, 2]
assert list(enumerate(counter(2))) == [(0, 0), (1, 1)]
assert list(zip(counter(2), 'ab')) == [(0, 'a'), (1, 'b')]
assert list(map(lambda v: v + 1, counter(3))) == [1, 2, 3]
assert list(filter(lambda v: v % 2 == 0, counter(4))) == [0, 2]
assert 2 in counter(4)
assert 9 not in counter(4)
assert ','.join(str(v) for v in counter(3)) == '0,1,2'
assert dict((v, v * 2) for v in counter(2)) == {0: 0, 1: 2}

a, b, c = counter(3)
assert (a, b, c) == (0, 1, 2)
first, *rest = counter(4)
assert first == 0
assert rest == [1, 2, 3]

total = 0
for value in counter(4):
    total += value
assert total == 6

# `for` with `else` and `break`
seen = []
for value in counter(5):
    if value == 3:
        break
    seen.append(value)
else:
    seen.append('no-break')
assert seen == [0, 1, 2]

seen = []
for value in counter(2):
    seen.append(value)
else:
    seen.append('no-break')
assert seen == [0, 1, 'no-break']

# === Nothing runs until the first step ===
trace = []


def records():
    trace.append('started')
    yield 1


g = records()
assert trace == []
assert next(g) == 1
assert trace == ['started']

# Argument binding still happens at the call, as in CPython.
try:
    counter()
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "counter() missing 1 required positional argument: 'n'"


# === Infinite generators ===
def naturals():
    n = 0
    while True:
        yield n
        n += 1


nat = naturals()
assert [next(nat) for _ in range(4)] == [0, 1, 2, 3]
assert next(nat) == 4


# === send() ===
def echo():
    while True:
        received = yield
        if received is None:
            return 'done'
        yield received * 2


g = echo()
assert next(g) is None
assert g.send(5) == 10
assert next(g) is None
try:
    g.send(None)
    assert False, 'expected StopIteration'
except StopIteration as exc:
    assert str(exc) == 'done'

# `send` before the generator has started is only allowed with None.
g = echo()
try:
    g.send(1)
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "can't send non-None value to a just-started generator"


# === StopIteration carries the return value ===
def with_value():
    yield 1
    return 42


g = with_value()
assert next(g) == 1
try:
    next(g)
    assert False, 'expected StopIteration'
except StopIteration as exc:
    assert str(exc) == '42'

# A `for` loop swallows the value, as CPython's does.
assert list(with_value()) == [1]


# === throw() ===
def catcher():
    try:
        yield 1
    except ValueError as exc:
        yield 'caught ' + str(exc)
    yield 'after'


g = catcher()
assert next(g) == 1
assert g.throw(ValueError('boom')) == 'caught boom'
assert next(g) == 'after'

# An unhandled throw kills the generator and reaches the caller.
g = catcher()
assert next(g) == 1
try:
    g.throw(KeyError('nope'))
    assert False, 'expected KeyError'
except KeyError as exc:
    assert str(exc) == "'nope'"
try:
    next(g)
    assert False, 'expected StopIteration'
except StopIteration:
    pass

# === close() ===
trace = []


def cleaner():
    try:
        yield 1
        yield 2
    finally:
        trace.append('cleanup')


g = cleaner()
assert next(g) == 1
g.close()
assert trace == ['cleanup']
# Closing again is a no-op, and the generator stays exhausted.
g.close()
assert trace == ['cleanup']
assert list(g) == []

# Closing an unstarted generator runs nothing.
trace = []
cleaner().close()
assert trace == []


# A generator that swallows GeneratorExit and yields again is an error.
def stubborn():
    try:
        yield 1
    except BaseException:
        yield 2


g = stubborn()
assert next(g) == 1
try:
    g.close()
    assert False, 'expected RuntimeError'
except RuntimeError as exc:
    assert str(exc) == 'generator ignored GeneratorExit'

# GeneratorExit is not an Exception, so `except Exception` lets it through.
trace = []


def careful():
    try:
        yield 1
    except Exception:
        trace.append('wrong')
    finally:
        trace.append('finally')


g = careful()
assert next(g) == 1
g.close()
assert trace == ['finally']


# === yield from ===
def inner():
    yield 1
    yield 2
    return 'inner-done'


def outer():
    result = yield from inner()
    yield result


assert list(outer()) == [1, 2, 'inner-done']


def flatten(nested):
    for chunk in nested:
        yield from chunk


assert list(flatten([[1, 2], [3], [], [4]])) == [1, 2, 3, 4]


# Delegating to plain iterables works and yields `None` as the value.
def from_sequences():
    a = yield from [10, 20]
    b = yield from (30,)
    yield a
    yield b


assert list(from_sequences()) == [10, 20, 30, None, None]


# `send` reaches the delegate.
def sub():
    a = yield 'a'
    b = yield a
    return b


def top():
    result = yield from sub()
    yield result


g = top()
assert next(g) == 'a'
assert g.send('x') == 'x'
assert g.send('y') == 'y'


# `throw` reaches the delegate, whose handler keeps the delegation alive.
def guarded():
    try:
        yield 'one'
    except ValueError:
        yield 'handled'
    yield 'two'


def wrapper():
    yield from guarded()
    yield 'end'


g = wrapper()
assert next(g) == 'one'
assert g.throw(ValueError('x')) == 'handled'
assert next(g) == 'two'
assert next(g) == 'end'


# A delegate that does not handle the throw propagates into the delegator.
def unguarded():
    yield 'a'


def catching_wrapper():
    try:
        yield from unguarded()
    except ValueError as exc:
        yield 'outer caught ' + str(exc)


g = catching_wrapper()
assert next(g) == 'a'
assert g.throw(ValueError('deep')) == 'outer caught deep'

# `close` reaches the delegate's `finally`.
trace = []


def cleaning_sub():
    try:
        yield 1
    finally:
        trace.append('sub')


def cleaning_top():
    try:
        yield from cleaning_sub()
    finally:
        trace.append('top')


g = cleaning_top()
assert next(g) == 1
g.close()
assert trace == ['sub', 'top']


# Nested delegation chains through both levels.
def level0():
    yield 1
    return 'zero'


def level1():
    value = yield from level0()
    yield value
    return 'one'


def level2():
    value = yield from level1()
    yield value


assert list(level2()) == [1, 'zero', 'one']

# === Generator expressions are lazy ===
trace = []


def watched(values):
    for value in values:
        trace.append(value)
        yield value


gen = (v * 2 for v in watched([1, 2, 3]))
assert trace == []
assert next(gen) == 2
assert trace == [1]
assert list(gen) == [4, 6]
assert trace == [1, 2, 3]

assert type(x for x in [1]).__name__ == 'generator'

# The outermost iterable is evaluated eagerly, so a bad one raises at once.
try:
    (x for x in 5)
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "'int' object is not iterable"

# `next(genexpr, default)`, the shape that used to fail as 'list' is not an iterator.
values = [1, 2, 3]
assert next((v for v in values if v > 5), None) is None
assert next((v for v in values if v > 1), None) == 2
assert next((v for v in values if v > 1)) == 2

# Filters, multiple clauses, and captured names.
factor = 10
assert list(v * factor for v in values if v != 2) == [10, 30]
assert list((a, b) for a in [1, 2] for b in 'xy') == [(1, 'x'), (1, 'y'), (2, 'x'), (2, 'y')]
assert list(a + b for a in [1, 2] for b in [10, 20] if a * b > 15) == [21, 12, 22]

# A generator expression is single-shot.
once = (v for v in [1, 2])
assert list(once) == [1, 2]
assert list(once) == []


def closure_genexpr(scale):
    return (v * scale for v in [1, 2, 3])


assert list(closure_genexpr(3)) == [3, 6, 9]

# A genexpr over an infinite generator stays lazy.
assert next(v for v in naturals() if v > 100) == 101


# === Generators are closures like any other function ===
def adders(base):
    def step(x):
        return x + base

    for value in [1, 2]:
        yield step(value)


assert list(adders(10)) == [11, 12]


def counting(start):
    total = start
    while True:
        received = yield total
        total += received


g = counting(100)
assert next(g) == 100
assert g.send(5) == 105
assert g.send(10) == 115


# === Exceptions propagate out of a generator to its consumer ===
def raiser():
    yield 1
    raise ValueError('inside')


g = raiser()
assert next(g) == 1
try:
    next(g)
    assert False, 'expected ValueError'
except ValueError as exc:
    assert str(exc) == 'inside'
# The generator is exhausted afterwards.
try:
    next(g)
    assert False, 'expected StopIteration'
except StopIteration:
    pass

try:
    list(raiser())
    assert False, 'expected ValueError'
except ValueError as exc:
    assert str(exc) == 'inside'


# === Re-entering a running generator is rejected ===
def reentrant():
    yield next(holder[0])


holder = [None]
g = reentrant()
holder[0] = g
try:
    next(g)
    assert False, 'expected ValueError'
except ValueError as exc:
    assert str(exc) == 'generator already executing'


# === yield in nested statements ===
def branchy(flag):
    if flag:
        yield 'yes'
    else:
        yield 'no'
    for value in [1, 2]:
        if value == 2:
            yield value


assert list(branchy(True)) == ['yes', 2]
assert list(branchy(False)) == ['no', 2]


def in_try():
    try:
        yield 'body'
    except KeyError:
        yield 'handler'
    finally:
        yield 'finally'


assert list(in_try()) == ['body', 'finally']


def in_except():
    try:
        raise KeyError('k')
    except KeyError as exc:
        yield str(exc)
        yield 'still here'


assert list(in_except()) == ["'k'", 'still here']


def in_with():
    class Ctx:
        def __enter__(self):
            return 'entered'

        def __exit__(self, *args):
            trace.append('exited')
            return False

    with Ctx() as value:
        yield value


trace = []
assert list(in_with()) == ['entered']
assert trace == ['exited']


# === Generators of generators ===
def outer_gen(n):
    for value in counter(n):
        yield counter(value)


assert [list(g) for g in outer_gen(3)] == [[], [0], [0, 1]]
