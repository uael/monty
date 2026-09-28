# xfail=cpython
# `monty.instance(cls, fields)`: an instance of a class of the session holding
# these attributes, made with no `__init__` run. CPython has no `monty` module,
# so this runs on Monty alone.
import monty


class Plan:
    n = 21

    def __init__(self, x):
        raise AssertionError('__init__ runs never')

    def total(self):
        return self.n + self.x


# === an instance holding the fields, with no __init__ run ===
p = monty.instance(Plan, {'x': 1})
assert isinstance(p, Plan)
assert p.x == 1
assert p.total() == 22

# === the fields are the instance's own ===
q = monty.instance(Plan, {'x': 2})
q.x = 3
assert p.x == 1
assert q.total() == 24

# === no fields is an instance holding nothing ===
assert isinstance(monty.instance(Plan, {}), Plan)

# === a frozen dataclass is made the same, and stays frozen ===
from dataclasses import dataclass


@dataclass(frozen=True)
class Point:
    x: int
    y: int


r = monty.instance(Point, {'x': 1, 'y': 2})
assert r == Point(1, 2)
try:
    r.x = 5
except Exception as no:
    assert type(no).__name__ == 'FrozenInstanceError', type(no).__name__
else:
    raise AssertionError('frozen')

# === cls is a class of the session, and fields a dict keyed by str ===
try:
    monty.instance(int, {})
except TypeError as no:
    assert str(no) == 'instance() cls must be a class of the session, not type', str(no)
else:
    raise AssertionError('cls')
try:
    monty.instance(Plan, [])
except TypeError as no:
    assert str(no) == 'instance() fields must be a dict, not list', str(no)
else:
    raise AssertionError('fields')
try:
    monty.instance(Plan, {1: 2})
except TypeError as no:
    assert str(no) == 'instance() fields must be keyed by str, not int', str(no)
else:
    raise AssertionError('keys')
