# A type displays the module that defines it wherever CPython's `tp_name` does,
# but `__name__` is that display name after its last dot. The two differ for
# exactly the types whose display is qualified, and agree for every other.

import asyncio
import collections
import datetime
import itertools
import json
import re

# === Qualified display, bare name ===
assert collections.deque.__name__ == 'deque'
assert str(collections.deque) == "<class 'collections.deque'>"

assert type(re.compile('a')).__name__ == 'Pattern'
assert str(type(re.compile('a'))) == "<class 're.Pattern'>"

assert type(re.match('a', 'a')).__name__ == 'Match'
assert str(type(re.match('a', 'a'))) == "<class 're.Match'>"

assert datetime.datetime.__name__ == 'datetime'
assert str(datetime.datetime) == "<class 'datetime.datetime'>"

assert type(itertools.count()).__name__ == 'count'
assert str(type(itertools.count())) == "<class 'itertools.count'>"
assert type(itertools.repeat(1)).__name__ == 'repeat'
assert type(itertools.chain()).__name__ == 'chain'

# === `__qualname__` answers the same, Monty having no nesting to qualify with ===
assert collections.deque.__qualname__ == 'deque'
assert type(re.compile('a')).__qualname__ == 'Pattern'
assert datetime.datetime.__qualname__ == 'datetime'

# === An exception class is a type object and follows the same rule ===
assert json.JSONDecodeError.__name__ == 'JSONDecodeError'
assert re.PatternError.__name__ == 'PatternError'
assert re.error.__name__ == 'PatternError'
assert asyncio.CancelledError.__name__ == 'CancelledError'

try:
    json.loads('{')
    raise AssertionError('expected a decode error')
except json.JSONDecodeError as exc:
    assert type(exc).__name__ == 'JSONDecodeError'

try:
    re.compile('(')
    raise AssertionError('expected a pattern error')
except re.PatternError as exc:
    assert type(exc).__name__ == 'PatternError'

# === An undotted display is its own name ===
assert int.__name__ == 'int'
assert float.__name__ == 'float'
assert str.__name__ == 'str'
assert bytes.__name__ == 'bytes'
assert list.__name__ == 'list'
assert tuple.__name__ == 'tuple'
assert dict.__name__ == 'dict'
assert set.__name__ == 'set'
assert frozenset.__name__ == 'frozenset'
assert bool.__name__ == 'bool'
assert range.__name__ == 'range'
assert slice.__name__ == 'slice'
assert type(None).__name__ == 'NoneType'
assert ValueError.__name__ == 'ValueError'
assert Exception.__name__ == 'Exception'


# === A sandbox class has no module to qualify with, so nothing is dropped ===
class Holder:
    pass


assert Holder.__name__ == 'Holder'
assert Holder.__qualname__ == 'Holder'

Point = collections.namedtuple('Point', 'x y')
assert Point.__name__ == 'Point'
