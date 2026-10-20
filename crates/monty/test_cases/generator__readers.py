# A reader steps a generator on the loop of the VM: each reader gives what CPython gives.

seen: list[int] = []


def counted(n: int):
    for i in range(n):
        seen.append(i)
        yield i


# === `any()` and `all()` stop at the value that decides ===
def above(n: int, limit: int):
    for i in counted(n):
        yield i > limit


seen.clear()
assert any(above(5, 1))
assert seen == [0, 1, 2], seen
seen.clear()
assert all(above(0, 1))
assert not any(above(0, 1))

# === `next()` gives each value, then its default or `StopIteration` ===
g = counted(2)
assert next(g) == 0
assert next(g, 'end') == 1
assert next(g, 'end') == 'end'
try:
    next(g)
    raise AssertionError('expected StopIteration')
except StopIteration:
    pass

# === `for`, a comprehension and unpacking read it one value at a time ===
total = 0
for v in counted(3):
    total += v
assert total == 3
assert [v * 2 for v in counted(3)] == [0, 2, 4]
a, b, c = counted(3)
assert (a, b, c) == (0, 1, 2)
first, *rest = counted(3)
assert (first, rest) == (0, [1, 2])
assert [*counted(2), 9] == [0, 1, 9]
assert {*counted(2)} == {0, 1}

# === a builtin of one iterable reads it whole ===
assert list(counted(3)) == [0, 1, 2]
assert tuple(counted(2)) == (0, 1)
assert set(counted(2)) == {0, 1}
assert frozenset(counted(2)) == frozenset({0, 1})
assert sum(counted(4)) == 6
assert sum(counted(3), 10) == 13
assert sorted(counted(3), reverse=True) == [2, 1, 0]
assert min(counted(3)) == 0
assert max(counted(3)) == 2
assert min(3, 1) == 1


def pairs():
    yield ('a', 1)
    yield ('b', 2)


assert dict(pairs()) == {'a': 1, 'b': 2}


def words():
    yield 'x'
    yield 'y'


assert '-'.join(words()) == 'x-y'
out = [0]
out.extend(counted(2))
assert out == [0, 0, 1]

# === a reader of a generator that is done is done at once ===
done = counted(1)
assert list(done) == [0]
assert list(done) == []
assert next(done, 'end') == 'end'
assert not any(done)
assert all(done)


# === what the generator raises reaches the handler around the reader ===
def failing():
    yield 1
    raise ValueError('no')


caught: list[str] = []
try:
    for v in failing():
        try:
            caught.append(str(v))
        except ValueError:
            caught.append('body')
except ValueError:
    caught.append('loop')
try:
    list(failing())
except ValueError:
    caught.append('list')
try:
    sum(failing())
except ValueError:
    caught.append('sum')
try:
    a, b = failing()
except ValueError as exc:
    caught.append(str(exc))


def failing_tests():
    yield False
    raise ValueError('no')


try:
    any(failing_tests())
except ValueError:
    caught.append('any')
assert caught == ['1', 'loop', 'list', 'sum', 'no', 'any'], caught
