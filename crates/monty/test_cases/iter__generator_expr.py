# === Basic generator expression ===
result = list(x * 2 for x in range(5))
assert result == [0, 2, 4, 6, 8]

# === With condition ===
result = list(x for x in range(10) if x % 2 == 0)
assert result == [0, 2, 4, 6, 8]

# === Nested generators ===
result = list(x + y for x in range(3) for y in range(2))
assert result == [0, 1, 1, 2, 2, 3]

# === Generator in function call ===
result = sum(x for x in range(5))
assert result == 10

# === Generator with unpacking ===
pairs = [(1, 2), (3, 4)]
result = list(a + b for a, b in pairs)
assert result == [3, 7]

# === It is a generator, and its own iterator ===
gen = (x for x in [1, 2])
assert type(gen).__name__ == 'generator'
assert iter(gen) is gen
assert repr(gen)[:31] == '<generator object <genexpr> at '
assert next(gen) == 1
assert next(gen) == 2
try:
    next(gen)
    assert False, 'expected StopIteration'
except StopIteration:
    pass
assert next(gen, 'done') == 'done'

# === The body runs one element at a time ===
seen = []


def note(x):
    seen.append(x)
    return x


gen = (note(x) for x in range(3))
assert seen == []
assert next(gen) == 0
assert seen == [0]
assert list(gen) == [1, 2]
assert seen == [0, 1, 2]

# === The outermost iterable is read where the expression is written ===
try:
    (x for x in 5)
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "'int' object is not iterable"

items = [1, 2, 3]
gen = (x for x in items)
items = [9]
assert list(gen) == [1, 2, 3]

# === Every later iterable is read late ===
rows = [[1, 2], [3]]
gen = (y for row in rows for y in row)
rows.append([4])
assert list(gen) == [1, 2, 3, 4]

# === Two filters on one generator, in the order they are written ===
order = []


def keep(tag, value):
    order.append(tag)
    return value


assert list(x for x in range(3) if keep('a', x) if keep('b', True)) == [1, 2]
assert order == ['a', 'a', 'b', 'a', 'b']

# === The targets do not leak ===
q = 'kept'
assert list(q for q in range(2)) == [0, 1]
assert q == 'kept'


# === It reads the scope that holds it ===
def capture():
    k = 100
    return list(x + k for x in range(3))


assert capture() == [100, 101, 102]

# === A walrus inside it binds outside it ===
gen = (w := x for x in range(3))
assert list(gen) == [0, 1, 2]
assert w == 2


def inner_walrus():
    gen = (v := x * 2 for x in range(3))
    assert list(gen) == [0, 2, 4]
    return v


assert inner_walrus() == 4

# === send, close and throw drive it ===
gen = (x for x in range(3))
assert gen.send(None) == 0
gen.close()
assert next(gen, 'shut') == 'shut'

gen = (x for x in range(3))
assert next(gen) == 0
try:
    gen.throw(ValueError('stop'))
    assert False, 'expected ValueError'
except ValueError as exc:
    assert str(exc) == 'stop'

# === One inside a comprehension ===
assert [list(y for y in range(x)) for x in range(3)] == [[], [0], [0, 1]]


# === Its targets can be an attribute or a subscript ===
class Box:
    value = 0


box = Box()
holder = {}
assert list(1 for box.value in [7]) == [1]
assert box.value == 7
assert list(1 for holder['k'] in [8]) == [1]
assert holder == {'k': 8}
