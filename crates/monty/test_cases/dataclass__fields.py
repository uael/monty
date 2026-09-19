# `dataclasses.fields()` answers the `Field` objects a `@dataclass` recorded,
# in definition order, for the class and for any of its instances.
from dataclasses import dataclass, fields


@dataclass
class Point:
    x: int
    y: str = 'a'


@dataclass
class Empty:
    pass


class Plain:
    pass


# === The class and an instance answer the same fields ===
assert [f.name for f in fields(Point)] == ['x', 'y']
assert [f.name for f in fields(Point(1))] == ['x', 'y']
assert fields(Empty) == ()
assert type(fields(Point)).__name__ == 'tuple'

# === A field reports its name and its default ===
x, y = fields(Point)
assert x.name == 'x'
assert y.name == 'y'
assert y.default == 'a'

# === Anything else is refused ===
for bad in (Plain, Plain(), 5, 'x', None):
    try:
        fields(bad)
        assert False, 'expected a non-dataclass to be refused'
    except TypeError as exc:
        assert str(exc) == 'must be called with a dataclass type or instance'
