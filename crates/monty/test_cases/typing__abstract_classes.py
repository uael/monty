# The classes the interpreter provides rather than the sandbox:
# `collections.abc`'s abstract classes and `typing.Protocol`. Both answer
# isinstance/issubclass, may be subscripted, and may stand in a base list.

import collections
import collections.abc as abc
from collections.abc import Callable, Hashable, Iterable, Iterator, Mapping, Sequence
from typing import Protocol, runtime_checkable

# === They are classes, and print like the ones in CPython ===
assert repr(Mapping) == "<class 'collections.abc.Mapping'>"
assert repr(abc.Callable) == "<class 'collections.abc.Callable'>"
assert repr(Protocol) == "<class 'typing.Protocol'>"

# === isinstance over builtin objects ===
assert isinstance({}, Mapping)
assert isinstance(collections.defaultdict(), Mapping)
assert isinstance(collections.Counter(), Mapping)
assert not isinstance([], Mapping)

assert isinstance([], Sequence)
assert isinstance('x', Sequence)
assert isinstance(b'x', Sequence)
assert isinstance((1,), Sequence)
assert isinstance(range(3), Sequence)
assert isinstance(collections.deque(), Sequence)
assert not isinstance({}, Sequence)
assert not isinstance({1}, Sequence)

assert isinstance([], Iterable)
assert isinstance({}, Iterable)
assert isinstance('x', Iterable)
assert not isinstance(1, Iterable)
assert isinstance(iter([]), Iterator)
assert isinstance(iter({}), Iterator)
assert not isinstance([], Iterator)

assert isinstance(len, Callable)
assert isinstance(int, Callable)
assert not isinstance(1, Callable)

assert isinstance(1, Hashable)
assert isinstance('x', Hashable)
assert isinstance(frozenset(), Hashable)
assert not isinstance([], Hashable)
assert not isinstance({}, Hashable)

assert isinstance({1}, abc.Set)
assert isinstance(frozenset(), abc.Set)
assert isinstance({}.keys(), abc.Set)
assert not isinstance([], abc.Set)

assert isinstance({}.keys(), abc.KeysView)
assert isinstance({}.items(), abc.ItemsView)
assert isinstance({}.values(), abc.ValuesView)
assert isinstance({}.keys(), abc.MappingView)
assert not isinstance({}.keys(), abc.ItemsView)

assert isinstance([], abc.Sized)
assert isinstance([], abc.Container)
assert isinstance([], abc.Collection)
assert not isinstance(1, abc.Sized)
assert isinstance('x', abc.Reversible)
assert not isinstance({1}, abc.Reversible)
assert isinstance(b'x', abc.ByteString)
assert isinstance([], abc.MutableSequence)
assert not isinstance((1,), abc.MutableSequence)
assert isinstance({}, abc.MutableMapping)
assert isinstance({1}, abc.MutableSet)
assert not isinstance(frozenset(), abc.MutableSet)

# === issubclass over builtin types ===
assert issubclass(dict, Mapping)
assert issubclass(list, Sequence)
assert issubclass(str, Sequence)
assert issubclass(list, Iterable)
assert issubclass(int, Hashable)
assert not issubclass(list, Hashable)
assert not issubclass(list, Mapping)
assert issubclass(bool, int)

# === Subscripting an abstract class ===
assert repr(Mapping[str, int]) == 'collections.abc.Mapping[str, int]'
assert repr(Callable[[int], str]) == 'collections.abc.Callable[[int], str]'
assert repr(abc.Coroutine[int, str, None]) == 'collections.abc.Coroutine[int, str, None]'
assert Mapping[str, int].__origin__ is Mapping
assert Mapping[str, int].__args__ == (str, int)

# === A sandbox class matches structurally, without naming a base ===


class Looped:
    def __iter__(self):
        return iter([1, 2])


assert isinstance(Looped(), Iterable)
assert issubclass(Looped, Iterable)
# Sequence has no structural hook in CPython either: only a declared base counts
assert not isinstance(Looped(), Sequence)


class Called:
    def __call__(self):
        return 1


assert isinstance(Called(), Callable)
assert not isinstance(Looped(), Callable)


class Boxed:
    def __len__(self):
        return 2

    def __contains__(self, item):
        return item == 1


assert isinstance(Boxed(), abc.Sized)
assert isinstance(Boxed(), abc.Container)
assert not isinstance(Boxed(), abc.Collection)

# === An abstract class in the base list contributes its members ===


class Countdown(Iterator):
    def __init__(self, n):
        self.n = n

    def __next__(self):
        if self.n == 0:
            raise StopIteration
        self.n -= 1
        return self.n


# `Iterator.__iter__` returns self, so the class needed only `__next__`
counter = Countdown(3)
assert iter(counter) is counter
assert list(Countdown(3)) == [2, 1, 0]
assert isinstance(Countdown(1), Iterator)
assert isinstance(Countdown(1), Iterable)
assert issubclass(Countdown, Iterator)

# A subscripted abstract base resolves through `__mro_entries__` to the class
# it subscripted, so the same members arrive


class Typed(Iterator[int]):
    def __next__(self):
        raise StopIteration


assert list(Typed()) == []
assert isinstance(Typed(), Iterator)

# === Protocol ===


class Kernel(Protocol):
    def make(self, seed: object) -> object: ...


assert Kernel._is_protocol is True

try:
    Kernel()
    assert False, 'expected a protocol to refuse instantiation'
except TypeError as exc:
    assert str(exc) == 'Protocols cannot be instantiated'

# A concrete implementation of a protocol is an ordinary class


class Native(Kernel):
    def make(self, seed):
        return seed


assert Native._is_protocol is False
assert Native().make(7) == 7

# A protocol that was never made runtime-checkable refuses every check, even
# for a class that really does derive from it


class Real:
    def close(self):
        return 'closed'


for check in (lambda: isinstance(Native(), Kernel), lambda: issubclass(Native, Kernel), lambda: isinstance(1, Kernel)):
    try:
        check()
        assert False, 'expected a bare protocol to refuse the check'
    except TypeError as exc:
        assert str(exc) == 'Instance and class checks can only be used with @runtime_checkable protocols'


@runtime_checkable
class Closeable(Protocol):
    def close(self) -> None: ...


assert Closeable._is_runtime_protocol is True
assert Closeable.__protocol_attrs__ == frozenset({'close'})
assert isinstance(Real(), Closeable)
assert not isinstance(Looped(), Closeable)
assert issubclass(Real, Closeable)
assert not issubclass(Looped, Closeable)

try:
    runtime_checkable(1)
    assert False, 'expected runtime_checkable to reject a non-class'
except TypeError as exc:
    assert str(exc) == 'issubclass() arg 1 must be a class'

try:
    runtime_checkable(Real)
    assert False, 'expected runtime_checkable to reject a non-protocol class'
except TypeError as exc:
    # The tail is the class repr, which Monty writes without a module prefix
    head, _, tail = str(exc).partition('got ')
    assert head == '@runtime_checkable can be only applied to protocol classes, '
    assert tail.endswith("Real'>"), tail


# An annotation declares a protocol member as surely as a `def` does, and it is
# the only way to declare a data one. A protocol that read its bindings alone
# would collect nothing here, and an empty member set is satisfied by every
# object, so the check would silently pass anything.
@runtime_checkable
class Contented(Protocol):
    content: str


@runtime_checkable
class Both(Protocol):
    content: str

    def go(self) -> int: ...


@runtime_checkable
class Deeper(Contented, Protocol):
    n: int


class Carries:
    def __init__(self) -> None:
        self.content = 'a'

    def go(self) -> int:
        return 1


class CarriesNothing:
    def __init__(self) -> None:
        self.other = 1


assert Contented.__protocol_attrs__ == frozenset({'content'})
assert Both.__protocol_attrs__ == frozenset({'content', 'go'})
assert Deeper.__protocol_attrs__ == frozenset({'content', 'n'})

assert isinstance(Carries(), Contented)
assert isinstance(Carries(), Both)
assert not isinstance(CarriesNothing(), Contented)
assert not isinstance(CarriesNothing(), Both)
assert not isinstance(Carries(), Deeper)
