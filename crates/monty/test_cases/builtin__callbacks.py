# === map ===
seen = []


def note(x):
    seen.append(x)
    return x * 2


assert list(map(note, [1, 2, 3])) == [2, 4, 6]
assert seen == [1, 2, 3]
assert list(map(lambda a, b: a + b, (x for x in [1, 2, 3]), [10, 20])) == [11, 22]
assert list(map(lambda a, b, c: str(a) + b + c, [1, 2], 'ab', (y for y in 'xy'))) == ['1ax', '2by']
try:
    map(note, [1], 5)
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "'int' object is not iterable"

# === filter ===
assert list(filter(lambda x: x % 2, (x for x in range(6)))) == [1, 3, 5]
assert list(filter(None, (x for x in [0, 1, '', 'a', None]))) == [1, 'a']

# === sorted and list.sort ===
seen = []
assert sorted([3, 1, 2], key=note) == [1, 2, 3]
assert seen == [3, 1, 2]
assert sorted('bca', key=lambda c: -ord(c), reverse=True) == ['a', 'b', 'c']
xs = [3, 1, 2]
assert xs.sort(key=lambda x: -x) is None
assert xs == [3, 2, 1]

# the list is empty while its key runs, as CPython detaches it
xs = [3, 1, 2]
lens = []
xs.sort(key=lambda x: lens.append(len(xs)) or x)
assert lens == [0, 0, 0]
assert xs == [1, 2, 3]

xs = [3, 1, 2]
try:
    xs.sort(key=lambda x: xs.append(9) or x)
    assert False, 'expected ValueError'
except ValueError as exc:
    assert str(exc) == 'list modified during sort'
assert xs == [1, 2, 3]

xs = [3, 1, 2]
try:
    xs.sort(key=lambda x: 1 / (x - 1))
    assert False, 'expected ZeroDivisionError'
except ZeroDivisionError as exc:
    assert str(exc) == 'division by zero'
assert xs == [3, 1, 2]

try:
    sorted([1, 'a'], key=lambda x: x)
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "'<' not supported between instances of 'str' and 'int'"

# === min and max ===
assert min([3, 1, 2], key=lambda x: -x) == 3
assert max(3, 1, 2, key=lambda x: -x) == 1
assert max((x for x in 'abc'), key=ord) == 'c'
assert min([], key=len, default='none') == 'none'
assert max([1, 1], key=lambda x: 0) == 1
try:
    max([], key=len)
    assert False, 'expected ValueError'
except ValueError as exc:
    assert str(exc) == 'max() iterable argument is empty'
try:
    min((x for x in []), key=len)
    assert False, 'expected ValueError'
except ValueError as exc:
    assert str(exc) == 'min() iterable argument is empty'
try:
    min(1, 'a', key=lambda x: x)
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "'<' not supported between instances of 'str' and 'int'"
try:
    max(1, 2, key=len, default=0)
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == 'Cannot specify a default for max() with multiple positional arguments'

# === zip and enumerate over a generator ===
assert list(zip((x for x in [1, 2, 3]), 'ab')) == [(1, 'a'), (2, 'b')]
assert list(zip('ab', (x for x in [1, 2, 3]))) == [('a', 1), ('b', 2)]
assert list(zip((x for x in [1, 2]), 'ab', strict=True)) == [(1, 'a'), (2, 'b')]
try:
    list(zip((x for x in [1]), 'ab', strict=True))
    assert False, 'expected ValueError'
except ValueError as exc:
    assert str(exc) == 'zip() argument 2 is longer than argument 1'
try:
    list(zip('a', 'b', (x for x in [1, 2]), strict=True))
    assert False, 'expected ValueError'
except ValueError as exc:
    assert str(exc) == 'zip() argument 3 is longer than arguments 1-2'
try:
    list(zip('abc', (x for x in [1]), strict=True))
    assert False, 'expected ValueError'
except ValueError as exc:
    assert str(exc) == 'zip() argument 2 is shorter than argument 1'
assert list(enumerate((x for x in 'ab'), 1)) == [(1, 'a'), (2, 'b')]
