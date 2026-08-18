# Tests for the builtins module and vars() over a module namespace.
import builtins
import math

# === vars() on a module gives its namespace as a dict ===
math_ns = vars(math)
assert type(math_ns) is dict
assert math_ns['pi'] == math.pi
assert math_ns['sqrt'] is math.sqrt
assert 'nope' not in math_ns

# === the builtins namespace holds the names a bare name resolves to ===
ns = vars(builtins)
assert type(ns) is dict

# Functions, type constructors and exception classes are all present, and are
# the very same objects the bare names resolve to.
assert ns['len'] is len
assert ns['sorted'] is sorted
assert ns['int'] is int
assert ns['str'] is str
assert ns['dict'] is dict
assert ns['ValueError'] is ValueError
assert ns['Exception'] is Exception
assert ns['StopIteration'] is StopIteration

# Singletons too.
assert ns['None'] is None
assert ns['True'] is True
assert ns['False'] is False
assert ns['Ellipsis'] is Ellipsis
assert ns['NotImplemented'] is NotImplemented


# === the reverse lookup the namespace exists for ===
def name_of(value: object) -> str:
    for name, got in vars(builtins).items():
        if got is value:
            return name
    raise AssertionError('not a builtin')


assert name_of(int) == 'int'
assert name_of(str) == 'str'
assert name_of(len) == 'len'
assert name_of(KeyError) == 'KeyError'

# === names that belong to a module, not to builtins ===
assert 'pi' not in ns
assert 'sqrt' not in ns

# === vars() rejects anything without a __dict__ ===
for bad in [1, 'x', [1], (1,), {1: 2}, None, 1.5]:
    try:
        vars(bad)
        raise AssertionError('expected TypeError')
    except TypeError as e:
        assert str(e) == 'vars() argument must have __dict__ attribute'

try:
    vars(math, math)  # pyright: ignore[reportCallIssue]
    raise AssertionError('expected TypeError')
except TypeError as e:
    assert str(e) == 'vars expected at most 1 argument, got 2'

# === the result is an ordinary dict, usable as one ===
copied = vars(math)
assert len(copied) == len(math_ns)
assert sorted(copied.keys()) == sorted(math_ns.keys())
assert 'pi' in copied.keys()
assert math.pi in copied.values()
assert copied.get('nope') is None
assert copied.get('nope', 'fallback') == 'fallback'
