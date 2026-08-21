class Box:
    def __init__(self):
        self.a = 1
        self.b = 2


# === `del name` at module scope ===
x = 1
del x
try:
    x
    assert False, 'expected NameError after del'
except NameError as exc:
    assert str(exc) == "name 'x' is not defined"

# Deleting an already-unbound module name raises rather than silently passing.
try:
    del x
    assert False, 'expected NameError deleting an unbound name'
except NameError as exc:
    assert str(exc) == "name 'x' is not defined"

# === `del` with several targets, applied left to right ===
p = 1
q = 2
del p, q
try:
    p
    assert False, 'expected NameError for p'
except NameError:
    pass
try:
    q
    assert False, 'expected NameError for q'
except NameError:
    pass

# A parenthesized list is the same statement.
r = 1
s = 2
del (r, s)
try:
    r
    assert False, 'expected NameError for r'
except NameError:
    pass

# === `del container[key]` ===
d = {'a': 1, 'b': 2}
del d['a']
assert d == {'b': 2}

try:
    del d['missing']
    assert False, 'expected KeyError'
except KeyError as exc:
    assert str(exc) == "'missing'"

lst = [10, 20, 30]
del lst[1]
assert lst == [10, 30]
del lst[-1]
assert lst == [10]

try:
    del lst[5]
    assert False, 'expected IndexError'
except IndexError as exc:
    assert str(exc) == 'list assignment index out of range'

# === `del obj.attr` ===
box = Box()
del box.a
assert hasattr(box, 'a') is False
assert box.b == 2

try:
    del box.a
    assert False, 'expected AttributeError'
except AttributeError as exc:
    assert str(exc) == "'Box' object has no attribute 'a'"

# A type with no instance dict rejects deletion of an unknown attribute.
try:
    del 'text'.nope
    assert False, 'expected AttributeError deleting a str attribute'
except AttributeError as exc:
    assert str(exc) == "'str' object has no attribute 'nope' and no __dict__ for setting new attributes"


# === Function locals ===
def unbound_read():
    v = 1
    del v
    return v


try:
    unbound_read()
    assert False, 'expected UnboundLocalError'
except UnboundLocalError as exc:
    assert str(exc) == "cannot access local variable 'v' where it is not associated with a value"


def del_before_bind():
    del w
    return 1


try:
    del_before_bind()
    assert False, 'expected UnboundLocalError deleting an unbound local'
except UnboundLocalError as exc:
    assert str(exc) == "cannot access local variable 'w' where it is not associated with a value"


# A `del` makes the name local, shadowing the module binding entirely.
shadowed = 'module'


def del_makes_local():
    del shadowed


try:
    del_makes_local()
    assert False, 'expected UnboundLocalError for a name only deleted'
except UnboundLocalError as exc:
    assert str(exc) == "cannot access local variable 'shadowed' where it is not associated with a value"


assert shadowed == 'module'


# `global` sends the delete to the module namespace.
target = 'gone'


def del_global():
    global target
    del target


del_global()
try:
    target
    assert False, 'expected NameError after global del'
except NameError:
    pass


# === Closure cells ===
def outer():
    cell = 1

    def inner():
        nonlocal cell
        del cell

    inner()
    return cell


try:
    outer()
    assert False, 'expected UnboundLocalError reading a deleted cell'
except UnboundLocalError as exc:
    assert str(exc) == "cannot access local variable 'cell' where it is not associated with a value"


# === Mixed target kinds in one statement ===
mixed_box = Box()
mixed_map = {'k': 1}
mixed_name = 1
del mixed_name, mixed_map['k'], mixed_box.b
assert mixed_map == {}
assert hasattr(mixed_box, 'b') is False

# === `del` on a subscript whose container expression has effects ===
calls = []


def container():
    calls.append('called')
    return grid


grid = {'k': 1}
del container()['k']
assert calls == ['called']
assert grid == {}

# === `del` on a `@dataclass` instance obeys the decorator's options ===
from dataclasses import FrozenInstanceError, dataclass


@dataclass
class Record:
    x: int
    y: int = 5


rec = Record(1)
del rec.x
assert hasattr(rec, 'x') is False
try:
    rec.x
    assert False, 'expected AttributeError reading a deleted field'
except AttributeError as exc:
    assert str(exc) == "'Record' object has no attribute 'x'"

# A plain default stays bound on the class, so deleting the instance attribute
# uncovers it rather than unbinding the name.
del rec.y
assert rec.y == 5


@dataclass(frozen=True)
class Sealed:
    x: int


sealed = Sealed(1)
try:
    del sealed.x
    assert False, 'expected FrozenInstanceError deleting a frozen field'
except FrozenInstanceError as exc:
    assert str(exc) == "cannot delete field 'x'"
assert sealed.x == 1

# Frozen refuses every name on the instance, field or not.
try:
    del sealed.absent
    assert False, 'expected FrozenInstanceError deleting a frozen non-field'
except FrozenInstanceError as exc:
    assert str(exc) == "cannot delete field 'absent'"


@dataclass(slots=True)
class Slotted:
    x: int


slotted = Slotted(1)
del slotted.x
assert hasattr(slotted, 'x') is False
try:
    del slotted.absent
    assert False, 'expected AttributeError deleting an undeclared slot'
except AttributeError as exc:
    assert str(exc) == "'Slotted' object has no attribute 'absent' and no __dict__ for setting new attributes"
