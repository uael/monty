# Tests for operator.attrgetter: single, multi-argument and dotted forms.
from operator import attrgetter
from pathlib import Path


class Inner:
    def __init__(self, c: int) -> None:
        self.c = c


class Outer:
    def __init__(self, x: int, y: int, c: int) -> None:
        self.x = x
        self.y = y
        self.b = Inner(c)


obj = Outer(1, 2, 3)

# === one argument yields the attribute itself ===
assert attrgetter('x')(obj) == 1
assert attrgetter('y')(obj) == 2

# === two or more yield a tuple, in argument order ===
assert attrgetter('x', 'y')(obj) == (1, 2)
assert attrgetter('y', 'x')(obj) == (2, 1)
assert attrgetter('x', 'y', 'x')(obj) == (1, 2, 1)
assert type(attrgetter('x', 'y')(obj)) is tuple
# Even with one argument the result is the bare attribute, not a 1-tuple.
assert type(attrgetter('x')(obj)) is not tuple

# === dotted paths walk one attribute at a time ===
assert attrgetter('b.c')(obj) == 3
assert attrgetter('b.c', 'x')(obj) == (3, 1)
assert attrgetter('b')(obj) is obj.b

# === reusable, and independent of the object it is applied to ===
get_x = attrgetter('x')
assert get_x(obj) == 1
assert get_x(Outer(9, 0, 0)) == 9

# === the sorting use ===
items = [Outer(3, 0, 0), Outer(1, 0, 0), Outer(2, 0, 0)]
assert [o.x for o in sorted(items, key=attrgetter('x'))] == [1, 2, 3]
assert [o.x for o in sorted(items, key=attrgetter('x'), reverse=True)] == [3, 2, 1]

# === repr rebuilds the arguments ===
assert repr(attrgetter('x')) == "operator.attrgetter('x')"
assert repr(attrgetter('x', 'y')) == "operator.attrgetter('x', 'y')"
assert repr(attrgetter('b.c')) == "operator.attrgetter('b.c')"

# === construction errors ===
try:
    attrgetter()
    raise AssertionError('expected TypeError')
except TypeError as e:
    assert str(e) == 'attrgetter expected 1 argument, got 0'

try:
    attrgetter(1)
    raise AssertionError('expected TypeError')
except TypeError as e:
    assert str(e) == 'attribute name must be a string'

# A non-string anywhere in the list is rejected, not just the first.
try:
    attrgetter('x', 1)
    raise AssertionError('expected TypeError')
except TypeError as e:
    assert str(e) == 'attribute name must be a string'

try:
    attrgetter(x=1)
    raise AssertionError('expected TypeError')
except TypeError as e:
    assert str(e) == 'attrgetter() takes no keyword arguments'

# === call errors ===
try:
    get_x()
    raise AssertionError('expected TypeError')
except TypeError as e:
    assert str(e) == 'attrgetter expected 1 argument, got 0'

try:
    get_x(obj, obj)
    raise AssertionError('expected TypeError')
except TypeError as e:
    assert str(e) == 'attrgetter expected 1 argument, got 2'

# === a missing attribute raises what getattr would ===
try:
    attrgetter('nope')(obj)
    raise AssertionError('expected AttributeError')
except AttributeError as e:
    assert str(e) == "'Outer' object has no attribute 'nope'"

# The failure is reported at the component that is missing, not the whole path.
try:
    attrgetter('b.nope')(obj)
    raise AssertionError('expected AttributeError')
except AttributeError as e:
    assert str(e) == "'Inner' object has no attribute 'nope'"

# === paths are split eagerly, so an empty component is an empty name ===
# 'x.' walks to `obj.x`, an int, then asks it for the empty attribute.
try:
    attrgetter('')(obj)
    raise AssertionError('expected AttributeError')
except AttributeError as e:
    assert str(e) == "'Outer' object has no attribute ''"

try:
    attrgetter('x.')(obj)
    raise AssertionError('expected AttributeError')
except AttributeError as e:
    assert str(e) == "'int' object has no attribute ''"

try:
    attrgetter('.x')(obj)
    raise AssertionError('expected AttributeError')
except AttributeError as e:
    assert str(e) == "'Outer' object has no attribute ''"

# === attributes of builtin types work too ===
assert attrgetter('name')(Path('/tmp/a.txt')) == 'a.txt'
assert attrgetter('suffix', 'stem')(Path('/tmp/a.txt')) == ('.txt', 'a')
