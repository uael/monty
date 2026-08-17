# Tests for the contextvars module: ContextVar get/set/reset and its Token.
from contextvars import ContextVar

# === name and repr ===
v = ContextVar('myvar')
assert v.name == 'myvar'
assert repr(v).startswith("<ContextVar name='myvar' at 0x")
assert repr(v).endswith('>')

w = ContextVar('w', default=7)
assert w.name == 'w'
assert repr(w).startswith("<ContextVar name='w' default=7 at 0x")

# === get before any set ===
# A variable with no default and no value raises LookupError, whose message is
# the variable's own repr.
try:
    v.get()
    raise AssertionError('expected LookupError')
except LookupError as e:
    assert str(e) == repr(v)

assert v.get(42) == 42
assert w.get() == 7
assert w.get(42) == 42

# === set, then get ===
t = v.set(1)
assert v.get() == 1
# An explicit default is ignored once the variable holds a value.
assert v.get(99) == 1
assert t.var is v
# `t.old_value` for a first set is not asserted: CPython reports Token.MISSING
# where Monty reports None (see limitations/contextvars.md).
assert repr(t).startswith('<Token var=')
assert repr(t).endswith('>')

t2 = v.set(2)
assert v.get() == 2
assert t2.old_value == 1

# === reset restores what the token recorded ===
v.reset(t2)
assert v.get() == 1
# Resetting past the first set returns the variable to unset, not to a default.
v.reset(t)
try:
    v.get()
    raise AssertionError('expected LookupError')
except LookupError as e:
    assert str(e) == repr(v)

# A spent token reports itself as used, and refuses a second reset.
assert repr(t).startswith('<Token used var=')
try:
    v.reset(t)
    raise AssertionError('expected RuntimeError')
except RuntimeError as e:
    assert str(e) == f'{t!r} has already been used once'

# === a token belongs to one variable ===
a = ContextVar('a')
ta = a.set(1)
try:
    w.reset(ta)
    raise AssertionError('expected ValueError')
except ValueError as e:
    assert str(e) == f'{ta!r} was created by a different ContextVar'
# The refused reset left both variables alone.
assert a.get() == 1
assert w.get() == 7

# === reset type check ===
try:
    a.reset(5)  # pyright: ignore[reportArgumentType]
    raise AssertionError('expected TypeError')
except TypeError as e:
    assert str(e) == 'expected an instance of Token, got 5'

# === values are ordinary objects, and identity is preserved ===
box = [0, 1]
holder = ContextVar('holder')
holder.set(box)
assert holder.get() is box

# === construction errors ===
try:
    ContextVar(1)  # pyright: ignore[reportArgumentType]
    raise AssertionError('expected TypeError')
except TypeError as e:
    assert str(e) == 'context variable name must be a str'

try:
    ContextVar('a', bogus=1)  # pyright: ignore[reportCallIssue]
    raise AssertionError('expected TypeError')
except TypeError as e:
    assert str(e) == "ContextVar() got an unexpected keyword argument 'bogus'"

# The ceiling counts the keyword-only `default` as well as the positional name.
try:
    ContextVar('a', default=1, x=2)  # pyright: ignore[reportCallIssue]
    raise AssertionError('expected TypeError')
except TypeError as e:
    assert str(e) == 'ContextVar() takes at most 2 arguments (3 given)'

# === method arity ===
try:
    v.get(1, 2)  # pyright: ignore[reportCallIssue]
    raise AssertionError('expected TypeError')
except TypeError as e:
    assert str(e) == 'get expected at most 1 argument, got 2'

try:
    v.set()  # pyright: ignore[reportCallIssue]
    raise AssertionError('expected TypeError')
except TypeError as e:
    assert str(e) == 'ContextVar.set() takes exactly one argument (0 given)'

# === two variables are independent, and unequal even when identically named ===
one = ContextVar('same')
two = ContextVar('same')
one.set('first')
two.set('second')
assert one.get() == 'first'
assert two.get() == 'second'
assert one != two
