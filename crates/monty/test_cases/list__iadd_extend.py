# `xs += ys` is `xs.extend(ys)`, not `xs = xs + ys`. The two differ in what they
# accept: `+` concatenates lists only, while `extend` takes any iterable, and
# they differ in what they report when the right operand is neither.

# === Any iterable extends ===
xs = [1]
xs += (2, 3)
assert xs == [1, 2, 3]

xs = []
xs += 'ab'
assert xs == ['a', 'b']

xs = []
xs += {1: 'a', 2: 'b'}
assert xs == [1, 2]

xs = []
xs += range(3)
assert xs == [0, 1, 2]

xs = []
xs += b'ab'
assert xs == [97, 98]

xs = [0]
xs += (n * 2 for n in range(3))
assert xs == [0, 0, 2, 4]

# === The list keeps its identity, so an alias sees the update ===
xs = [1]
alias = xs
xs += [2]
assert xs is alias
assert alias == [1, 2]

# `+` builds a new list instead, leaving the original alone.
xs = [1]
alias = xs
xs = xs + [2]
assert xs is not alias
assert alias == [1]

# === Extending by itself doubles, rather than looping ===
xs = [1, 2]
xs += xs
assert xs == [1, 2, 1, 2]

# The same when the list contains itself, where a naive loop would not terminate.
xs = [1]
xs.append(xs)
xs += xs
assert len(xs) == 4
assert xs[1] is xs
assert xs[3] is xs


# === A non-iterable reports the iterator protocol's error, not concatenation's ===
def expect(fn, message):
    try:
        fn()
        raise AssertionError('expected TypeError')
    except TypeError as exc:
        assert str(exc) == message


def iadd_int():
    xs = [1]
    xs += 2


expect(iadd_int, "'int' object is not iterable")


def iadd_none():
    xs = [1]
    xs += None


expect(iadd_none, "'NoneType' object is not iterable")


def iadd_float():
    xs = [1]
    xs += 1.5


expect(iadd_float, "'float' object is not iterable")


# `+` still refuses everything but a list, and still says so its own way.
def add_int():
    [1] + 2


expect(add_int, 'can only concatenate list (not "int") to list')


def add_str():
    [1] + 'ab'


expect(add_str, 'can only concatenate list (not "str") to list')


def add_tuple():
    [1] + (2,)


expect(add_tuple, 'can only concatenate list (not "tuple") to list')


# === A subscript target takes a different compile path to the same operation ===
d = {'k': [1]}
d['k'] += 'ab'
assert d == {'k': [1, 'a', 'b']}
