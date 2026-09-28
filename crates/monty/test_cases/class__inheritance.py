# === A subclass inherits methods and class variables ===


class Base:
    kind = 'base'

    def __init__(self, tag):
        self.tag = tag

    def describe(self):
        return self.kind + ':' + self.tag

    def who(self):
        return 'base'


class Derived(Base):
    kind = 'derived'

    def who(self):
        return 'derived'


d = Derived('x')
assert d.tag == 'x'
assert d.describe() == 'derived:x'
assert d.who() == 'derived'
assert Base('y').who() == 'base'
assert Derived.kind == 'derived'
assert Base.kind == 'base'

# === The chain is walked derived-first, to any depth ===


class One:
    def m(self):
        return 'one'


class Two(One):
    pass


class Three(Two):
    def n(self):
        return 'three'


assert Three().m() == 'one'
assert Three().n() == 'three'

# === isinstance and issubclass follow the chain ===
assert isinstance(d, Derived)
assert isinstance(d, Base)
assert not isinstance(Base('y'), Derived)
assert isinstance(Three(), One)

assert issubclass(Derived, Base)
assert not issubclass(Base, Derived)
assert issubclass(Derived, Derived)
assert issubclass(Three, One)
assert issubclass(Derived, (int, Base))
assert not issubclass(Derived, (int, str))

try:
    issubclass(d, Base)
    assert False, 'expected an instance to be refused as the first argument'
except TypeError as exc:
    assert str(exc) == 'issubclass() arg 1 must be a class'

# === Inherited dunders ===


class Shown:
    def __init__(self, n):
        self.n = n

    def __repr__(self):
        return 'Shown(' + str(self.n) + ')'

    def __eq__(self, other):
        return isinstance(other, Shown) and self.n == other.n


class Quiet(Shown):
    pass


assert repr(Quiet(1)) == 'Shown(1)'
assert Quiet(1) == Quiet(1)
assert Quiet(1) == Shown(1)
assert Quiet(1) != Quiet(2)


class Walked:
    def __iter__(self):
        return iter([1, 2])


class AlsoWalked(Walked):
    pass


assert list(AlsoWalked()) == [1, 2]

# === A base is evaluated in the enclosing scope ===
Outer = Base


def make():
    class Inner(Outer):
        pass

    return Inner


assert issubclass(make(), Base)

# === A decorator still applies to a subclass ===
seen = []


def mark(cls):
    seen.append(cls.__name__)
    return cls


@mark
class Marked(Base):
    pass


assert seen == ['Marked']
assert issubclass(Marked, Base)
