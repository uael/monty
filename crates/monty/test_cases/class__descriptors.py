# Descriptors are built either with the assignment form (`x = property(_get)`)
# or with the `@property` / `@staticmethod` / `@classmethod` decorator; the
# assignment form is the only way to reach the setter and deleter slots, since
# a property object exposes no `.setter` / `.deleter` methods to decorate with.

# === property: getter, setter ===
class Temperature:
    def _get_f(self):
        return self.c * 9 / 5 + 32

    def _set_f(self, value):
        self.c = (value - 32) * 5 / 9

    fahrenheit = property(_get_f, _set_f)

    def __init__(self, c):
        self.c = c


t = Temperature(100)
assert t.fahrenheit == 212
t.fahrenheit = 32
assert t.c == 0


# A read-only property rejects assignment.
class ReadOnly:
    def _value(self):
        return 42

    value = property(_value)


r = ReadOnly()
assert r.value == 42
try:
    r.value = 1
    assert False, 'expected AttributeError'
except AttributeError as exc:
    assert str(exc) == "property 'value' of 'ReadOnly' object has no setter"

# Accessed on the class, a property is the descriptor object itself.
assert type(ReadOnly.value) is property


# The `.setter` form returns a new property with the setter attached.
class Chained:
    def _get(self):
        return self._v

    def _set(self, value):
        self._v = value

    v = property(_get)
    v = v.setter(_set)


chained = Chained()
chained.v = 5
assert chained.v == 5


# === staticmethod ===
class Math:
    def add(a, b):
        return a + b

    add = staticmethod(add)


assert Math.add(2, 3) == 5
assert Math().add(2, 3) == 5


# === classmethod ===
class Registry:
    label = 'registry'

    def make(cls, n):
        return (cls.label, n)

    make = classmethod(make)


assert Registry.make(1) == ('registry', 1)
assert Registry().make(1) == ('registry', 1)


class SubRegistry(Registry):
    label = 'sub'


# The bound class is the one the attribute was reached through.
assert SubRegistry.make(2) == ('sub', 2)
assert SubRegistry().make(2) == ('sub', 2)


# === Inherited descriptors ===
class Derived(Temperature):
    pass


derived = Derived(0)
assert derived.fahrenheit == 32
derived.fahrenheit = 212
assert derived.c == 100


# === The decorator form ===
class Decorated:
    def __init__(self, v):
        self._v = v

    @property
    def value(self):
        return self._v

    @staticmethod
    def tag():
        return 'tag'

    @classmethod
    def named(cls):
        return cls.__name__


dec = Decorated(7)
assert dec.value == 7
assert Decorated.tag() == 'tag'
assert dec.tag() == 'tag'
assert Decorated.named() == 'Decorated'
assert dec.named() == 'Decorated'


# === property: deleter ===
class Deletable:
    def __init__(self):
        self._v = 1

    def _get(self):
        return self._v

    def _del(self):
        self._v = 'gone'

    value = property(_get, None, _del)


d = Deletable()
assert d.value == 1
del d.value
# The getter reads what the deleter left behind.
assert d.value == 'gone'
# The property owns the name, so a second delete runs the deleter again rather
# than reaching the instance `__dict__`.
del d.value
assert d.value == 'gone'


# A property with no deleter refuses `del`, as it refuses assignment.
undeletable = ReadOnly()
try:
    del undeletable.value
    assert False, 'expected AttributeError'
except AttributeError as exc:
    assert str(exc) == "property 'value' of 'ReadOnly' object has no deleter"
