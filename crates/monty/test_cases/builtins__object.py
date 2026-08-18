# `object`: the name every value is an instance of and every class a subclass
# of, which is what makes it the contract that proves nothing.

from collections.abc import Mapping
from dataclasses import dataclass


class Plain:
    def __init__(self) -> None:
        self.n = 1


@dataclass(frozen=True)
class Frozen:
    x: int


class Derived(object):
    pass


for value in [
    1,
    1.5,
    True,
    None,
    'a',
    b'b',
    [1],
    (1,),
    {1},
    frozenset({1}),
    {'a': 1},
    ...,
    Plain(),
    Frozen(1),
    int,
    object,
]:
    assert isinstance(value, object), f'{value!r} is not an object'

assert issubclass(int, object)
assert issubclass(Plain, object)
assert issubclass(Frozen, object)
assert issubclass(ValueError, object)
assert issubclass(Mapping, object)

assert isinstance(Derived(), Derived)
assert isinstance(Derived(), object)

assert repr(object) == "<class 'object'>", repr(object)
assert object.__name__ == 'object'

# `object` names itself in the builtin namespace, which is how a host maps the
# value back to the source text that denotes it.
import builtins

assert vars(builtins)['object'] is object
