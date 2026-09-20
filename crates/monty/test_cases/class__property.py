# === A property is computed on every read ===
reads = []


class Temperature:
    def __init__(self, celsius):
        self.celsius = celsius

    @property
    def fahrenheit(self):
        reads.append(self.celsius)
        return self.celsius * 9 / 5 + 32


t = Temperature(100)
assert t.fahrenheit == 212.0
assert reads == [100]
t.celsius = 0
assert t.fahrenheit == 32.0
assert reads == [100, 0]

# === A property is read-only ===
try:
    t.fahrenheit = 1
    assert False, 'expected a write to a property to be refused'
except AttributeError as exc:
    assert str(exc) == "property 'fahrenheit' of 'Temperature' object has no setter"

# The refused write left no instance attribute behind.
assert t.fahrenheit == 32.0

# === The getter runs with the instance ===


class Pair:
    def __init__(self, a, b):
        self.a = a
        self.b = b

    @property
    def total(self):
        return self.a + self.b

    @property
    def doubled(self):
        return self.total * 2


p = Pair(1, 2)
assert p.total == 3
assert p.doubled == 6

# === A getter that raises propagates ===


class Fussy:
    @property
    def bad(self):
        raise ValueError('no')


try:
    Fussy().bad
    assert False, 'expected the getter to raise'
except ValueError as exc:
    assert str(exc) == 'no'

# === property() is the same object the decorator builds ===


class Manual:
    def _get(self):
        return 7

    value = property(_get)


assert Manual().value == 7
assert type(Manual.value).__name__ == 'property'

# === A property read on the class gives the descriptor ===
assert repr(Manual.value).startswith('<property object at 0x')
