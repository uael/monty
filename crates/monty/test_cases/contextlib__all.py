# Tests for contextlib.suppress and contextlib.AbstractContextManager.
from contextlib import AbstractContextManager, suppress

# === suppressing the exact class ===
ran = False
with suppress(ValueError):
    raise ValueError('x')
assert not ran

# The body after the raise is skipped, but the statement after the block runs.
reached = False
with suppress(ValueError):
    raise ValueError('x')
    reached = True  # pyright: ignore[reportUnreachable]
assert not reached

# === subclass matching ===
with suppress(Exception):
    raise ValueError('x')
with suppress(LookupError):
    raise KeyError('k')
with suppress(ArithmeticError):
    raise ZeroDivisionError('z')

# === a non-matching exception propagates ===
try:
    with suppress(TypeError):
        raise ValueError('x')
    raise AssertionError('expected ValueError')
except ValueError as e:
    assert str(e) == 'x'

# A sibling class does not catch: KeyError and IndexError share LookupError but
# neither is the other.
try:
    with suppress(IndexError):
        raise KeyError('k')
    raise AssertionError('expected KeyError')
except KeyError:
    pass

# === variadic, and the empty case ===
with suppress(SyntaxError, ValueError, OverflowError):
    raise OverflowError('o')
with suppress(SyntaxError, ValueError, OverflowError):
    raise SyntaxError('s')

# suppress() with no arguments suppresses nothing, and is not an error.
with suppress():
    pass
try:
    with suppress():
        raise ValueError('x')
    raise AssertionError('expected ValueError')
except ValueError:
    pass

# === a body that raises nothing ===
done = False
with suppress(ValueError):
    done = True
assert done

# === __enter__ returns None, so `as` binds None ===
with suppress(ValueError) as entered:
    pass
assert entered is None

# === the manager is reusable, and holds no state between blocks ===
reusable = suppress(ValueError)
with reusable:
    raise ValueError('first')
with reusable:
    raise ValueError('second')

# === the protocol methods called directly ===
s = suppress(ValueError)
assert s.__enter__() is None
assert s.__exit__(None, None, None) is None

# === arguments are validated on exit, not on construction ===
# CPython's suppress stores its arguments untouched and only calls issubclass
# when an exception reaches __exit__, so a bad argument is inert until then.
with suppress(1):  # pyright: ignore[reportArgumentType]
    pass

try:
    with suppress(1):  # pyright: ignore[reportArgumentType]
        raise ValueError('x')
    raise AssertionError('expected TypeError')
except TypeError as e:
    assert str(e) == 'issubclass() arg 2 must be a class, a tuple of classes, or a union'

# issubclass stops at the first match, so a bad argument *after* a matching one
# is never reached and never raises.
with suppress(ValueError, 1):  # pyright: ignore[reportArgumentType]
    raise ValueError('x')

# Before one, it is reached first.
try:
    with suppress(1, ValueError):  # pyright: ignore[reportArgumentType]
        raise ValueError('x')
    raise AssertionError('expected TypeError')
except TypeError as e:
    assert str(e) == 'issubclass() arg 2 must be a class, a tuple of classes, or a union'

# === nesting ===
with suppress(TypeError):
    with suppress(ValueError):
        raise ValueError('inner')

# The inner manager declines, the outer one catches.
with suppress(ValueError):
    with suppress(TypeError):
        raise ValueError('outer catches')


# === sandbox-defined exception classes ===
# `suppress` applies the rule an `except` clause applies, so a class defined in
# the sandbox catches its own instances and those of its subclasses.
class Halt(Exception):
    pass


class Deeper(Halt):
    pass


with suppress(Halt):
    raise Halt('exact')

with suppress(Halt):
    raise Deeper('subclass')

with suppress(ValueError, Halt):
    raise Halt('second candidate')

# A builtin never matches a sandbox class, and the reverse holds too.
try:
    with suppress(Halt):
        raise ValueError('unrelated')
    raise AssertionError('the ValueError should have propagated')
except ValueError as e:
    assert str(e) == 'unrelated'

try:
    with suppress(ValueError):
        raise Halt('unrelated')
    raise AssertionError('the Halt should have propagated')
except Halt as e:
    assert str(e) == 'unrelated'


# === AbstractContextManager ===
# Subscripting yields the base itself rather than a distinct alias object, so
# only the class is asserted here (see limitations/contextlib.md).
assert AbstractContextManager['X'] is not None
