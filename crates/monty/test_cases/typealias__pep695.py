# === `type X = ...` builds a real runtime object ===
type Simple = int

assert Simple.__name__ == 'Simple'
assert Simple.__value__ is int
assert repr(Simple) == 'Simple'
assert str(Simple) == 'Simple'

# The value is memoized: reading it twice gives the same object.
type Boxed = [1, 2]
assert Boxed.__value__ is Boxed.__value__
assert Boxed.__value__ == [1, 2]

# === The value is lazy ===
# Nothing is evaluated at the `type` statement, so an alias may name itself and
# may name things that do not exist yet.
type SelfRef = ('leaf', SelfRef)
assert SelfRef.__value__[0] == 'leaf'
assert SelfRef.__value__[1] is SelfRef

type Later = defined_later
defined_later = 'now'
assert Later.__value__ == 'now'

# === Aliases are ordinary values ===
registry = {}
registry[Simple.__name__] = Simple
assert registry['Simple'] is Simple

aliases = [Simple, Boxed]
assert [a.__name__ for a in aliases] == ['Simple', 'Boxed']

# === Read-only attributes ===
try:
    Simple.__name__ = 'other'
    assert False, 'expected AttributeError assigning __name__'
except AttributeError:
    pass

try:
    Simple.missing
    assert False, 'expected AttributeError for an unknown attribute'
except AttributeError:
    pass


# === Aliases inside functions and classes ===
def make():
    type Local = [1, 2]
    return Local


made = make()
assert made.__name__ == 'Local'
assert made.__value__ == [1, 2]


def closure_alias():
    captured = 'captured'
    type Uses = captured
    return Uses


assert closure_alias().__value__ == 'captured'


class Holder:
    type Member = [3, 4]


assert Holder.Member.__name__ == 'Member'
assert Holder.Member.__value__ == [3, 4]


# === PEP 695 type parameters parse on functions, classes and aliases ===
def identity[T](value: T) -> T:
    return value


assert identity(3) == 3
assert identity('x') == 'x'


def bounded[T: int, *Ts, **P](value):
    return value


assert bounded(1) == 1


class Generic[T]:
    label = 'generic'

    def get(self, value: T) -> T:
        return value


assert Generic().get(5) == 5
assert Generic.label == 'generic'

type Parametrised[T] = [5]
assert Parametrised.__name__ == 'Parametrised'
assert Parametrised.__value__ == [5]
