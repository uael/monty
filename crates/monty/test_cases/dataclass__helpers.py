# The `dataclasses` module helpers: `fields`, `asdict`, `astuple`, `replace`
# and `is_dataclass`, including `__post_init__`, which every one of them runs
# through the constructor.
from dataclasses import asdict, astuple, dataclass, field, fields, is_dataclass, replace


@dataclass(frozen=True)
class Point:
    x: int
    y: int


@dataclass
class Shape:
    name: str
    points: list[Point]
    origin: Point = Point(0, 0)
    lookup: dict[str, Point] = field(default_factory=dict)


s = Shape('tri', [Point(1, 2)], Point(3, 4), {'a': Point(5, 6)})

# === is_dataclass accepts a class or an instance ===
assert is_dataclass(Point)
assert is_dataclass(Point(1, 2))
assert not is_dataclass(5)
assert not is_dataclass(int)


# === fields() works on either too ===
assert [f.name for f in fields(Point)] == ['x', 'y']
assert [f.name for f in fields(Point(1, 2))] == ['x', 'y']
try:
    fields(5)
    raise AssertionError('fields() needs a dataclass')
except TypeError as exc:
    assert str(exc) == 'must be called with a dataclass type or instance', str(exc)


# === asdict recurses through dataclasses, lists, tuples and dicts ===
assert asdict(Point(1, 2)) == {'x': 1, 'y': 2}
assert asdict(s) == {
    'name': 'tri',
    'points': [{'x': 1, 'y': 2}],
    'origin': {'x': 3, 'y': 4},
    'lookup': {'a': {'x': 5, 'y': 6}},
}

# the result is a fresh structure, not the instance's own containers
assert asdict(s)['points'] is not s.points


# === astuple does the same, collapsing to tuples ===
assert astuple(Point(1, 2)) == (1, 2)
assert astuple(s) == ('tri', [(1, 2)], (3, 4), {'a': (5, 6)})


# === both take a factory ===
assert asdict(Point(1, 2), dict_factory=list) == [('x', 1), ('y', 2)]
assert astuple(Point(1, 2), tuple_factory=list) == [1, 2]

# and both refuse anything that is not a dataclass instance
try:
    asdict(5)
    raise AssertionError('asdict() needs an instance')
except TypeError as exc:
    assert str(exc) == 'asdict() should be called on dataclass instances', str(exc)
try:
    astuple(Point)
    raise AssertionError('astuple() needs an instance, not a class')
except TypeError as exc:
    assert str(exc) == 'astuple() should be called on dataclass instances', str(exc)


# === replace builds a new instance, carrying over what was not named ===
p = Point(1, 2)
q = replace(p, y=9)
assert q == Point(1, 9)
assert p == Point(1, 2)
assert replace(p) == p
assert replace(p) is not p

try:
    replace(5)
    raise AssertionError('replace() needs an instance')
except TypeError as exc:
    assert str(exc) == 'replace() should be called on dataclass instances', str(exc)


# replace refuses to set a field the constructor does not take
@dataclass
class Derived:
    a: int
    cached: int = field(init=False, default=0)


try:
    replace(Derived(1), cached=5)
    raise AssertionError('an init=False field cannot be replaced')
except TypeError as exc:
    assert str(exc) == 'field cached is declared with init=False, it cannot be specified with replace()', str(exc)


# === __post_init__ runs after the fields are bound ===
@dataclass
class Sum:
    a: int
    b: int
    total: int = field(init=False, default=0)

    def __post_init__(self):
        self.total = self.a + self.b


assert Sum(2, 3).total == 5
assert repr(Sum(2, 3)) == 'Sum(a=2, b=3, total=5)'


# it sees defaults and factory-built values too
@dataclass
class Checked:
    xs: list[int] = field(default_factory=list)
    n: int = field(init=False, default=0)

    def __post_init__(self):
        self.n = len(self.xs)


assert Checked([1, 2, 3]).n == 3
assert Checked().n == 0


# a raising __post_init__ aborts the construction
@dataclass(frozen=True)
class Positive:
    value: int

    def __post_init__(self):
        if self.value <= 0:
            raise ValueError(f'value must be positive, got {self.value}')


assert Positive(1).value == 1
try:
    Positive(-1)
    raise AssertionError('__post_init__ should have rejected this')
except ValueError as exc:
    assert str(exc) == 'value must be positive, got -1', str(exc)

# replace() runs it again on the new instance
assert replace(Positive(1), value=2).value == 2
try:
    replace(Positive(1), value=-2)
    raise AssertionError('replace() runs __post_init__ too')
except ValueError:
    pass
