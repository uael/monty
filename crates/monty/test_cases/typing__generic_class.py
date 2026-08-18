# PEP 695 type parameters on a class: `class Held[T](Spawned[T])`.
#
# A type parameter is a real `typing.TypeVar`, bound once when the class
# statement runs, readable by the bases and the class body, and invisible
# outside them.

from dataclasses import dataclass


class Shows[T]:
    param = T


assert repr(Shows.param) == 'T'
assert Shows.param.__name__ == 'T'
assert repr(type(Shows.param)) == "<class 'typing.TypeVar'>"

# One object per class statement, so the class body sees the same `T` twice
assert Shows.param is Shows.param


class Twice[T]:
    a = T
    b = T


assert Twice.a is Twice.b

# Distinct parameters are distinct objects


class Pair[K, V]:
    k = K
    v = V


assert Pair.k.__name__ == 'K'
assert Pair.v.__name__ == 'V'
assert Pair.k is not Pair.v

# Each execution of a class statement makes its own


def make():
    class Fresh[T]:
        param = T

    return Fresh.param


assert make() is not make()

# === The parameter does not leak out of the class statement ===
try:
    T
    assert False, 'expected the type parameter to stay inside the class'
except NameError as exc:
    assert str(exc) == "name 'T' is not defined"

# An outer binding of the same name is untouched
T = 'outer'


class Shadows[T]:
    inner = T


assert T == 'outer'
assert Shadows.inner is not T

# === A generic class is subscriptable ===
assert repr(Shows[int]) == 'Shows[int]'
assert Shows[int].__origin__ is Shows
assert Shows[int].__args__ == (int,)
assert Shows[int] == Shows[int]
assert Shows[int] != Shows[str]

# A type parameter may itself be a subscript argument


class Boxes[T]:
    of = list[T]


assert repr(Boxes.of) == 'list[T]'

# === A generic base ===


class Spawned[T]:
    def __init__(self, value):
        self.value = value

    def get(self):
        return self.value


class Held[T](Spawned[T]):
    def twice(self):
        return self.get() * 2


held = Held(21)
assert held.get() == 21
assert held.twice() == 42
assert isinstance(held, Spawned)
assert issubclass(Held, Spawned)
assert type(held).__name__ == 'Held'

# The base is evaluated before the body, as CPython does


order = []


def base_of(name):
    order.append(name)
    return Spawned


def noted(name):
    order.append(name)
    return name


class Ordered[T](base_of('base')):
    marker = noted('body')


assert order == ['base', 'body']

# A non-generic subclass of a generic class works the same


class Plain(Held):
    pass


assert Plain(3).twice() == 6

# === A generic dataclass, and an undecorated generic subclass of one ===


@dataclass(eq=False)
class Boxed[T]:
    value: int


class Wrapped[T](Boxed[T]):
    def bump(self):
        return self.value + 1


wrapped = Wrapped(4)
assert wrapped.value == 4
assert wrapped.bump() == 5
assert repr(Boxed[int]) == 'Boxed[int]'

# === A generic class defined inside a function captures as it should ===


def outer():
    prefix = 'hi'

    class Base:
        def greet(self):
            return prefix

    class Sub[T](Base):
        def shout(self):
            return self.greet() + '!'

    return Sub().shout()


assert outer() == 'hi!'
