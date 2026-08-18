# Runtime type forms: `types.GenericAlias` (list[int]), `typing.Union`
# (int | str), and the typing functions that read them.

import types
import typing
from typing import dataclass_transform, get_args, get_origin, overload

# === Subscripting a builtin class builds a GenericAlias ===
assert repr(list[int]) == 'list[int]'
assert repr(dict[str, int]) == 'dict[str, int]'
assert repr(tuple[int, ...]) == 'tuple[int, ...]'
assert repr(list[list[int]]) == 'list[list[int]]'
assert repr(frozenset[bytes]) == 'frozenset[bytes]'
assert repr(set[int]) == 'set[int]'
assert repr(type[int]) == 'type[int]'
assert repr(list[None]) == 'list[None]'
assert repr(tuple[()]) == 'tuple[()]'
assert repr(list['Wire']) == "list['Wire']"
assert repr(list[1]) == 'list[1]'

# `__origin__` / `__args__`
assert list[int].__origin__ is list
assert list[int].__args__ == (int,)
assert dict[str, int].__args__ == (str, int)
assert tuple[()].__args__ == ()
assert tuple[int, ...].__args__ == (int, Ellipsis)

# A tuple subscript and a spelled-out one are the same alias
assert list[(int, str)] == list[int, str]

# === type() of an alias ===
assert type(list[int]) is types.GenericAlias
assert repr(type(list[int])) == "<class 'types.GenericAlias'>"

# === Equality and hashing ===
assert list[int] == list[int]
assert list[int] != list[str]
assert list[int] != dict[str, int]
assert (list[int] == 1) is False
assert hash(list[int]) == hash(list[int])
d = {list[int]: 'ints'}
assert d[list[int]] == 'ints'

# === Unknown attributes fall through to the origin ===
assert list[int].__name__ == 'list'

# === Calling an alias builds the origin ===
assert list[int]() == []
assert dict[str, int]() == {}

# === Not every class is subscriptable ===
try:
    str[int]
    assert False, 'expected str to reject a subscript'
except TypeError as exc:
    assert str(exc) == "type 'str' is not subscriptable"

# === `__class_getitem__` on a sandbox class ===


class Held:
    def __class_getitem__(cls, item):
        return (cls.__name__, item)


assert Held[int] == ('Held', int)
assert Held[int, str] == ('Held', (int, str))


class Bound:
    @classmethod
    def __class_getitem__(cls, item):
        return ('bound', cls.__name__, item)


assert Bound[int] == ('bound', 'Bound', int)


class Inherited(Held):
    pass


assert Inherited[int] == ('Inherited', int)

# === Unions ===
assert repr(int | str) == 'int | str'
assert repr(int | None) == 'int | None'
assert repr(int | str | None) == 'int | str | None'
assert repr(list[int] | None) == 'list[int] | None'
assert repr(None | int) == 'None | int'

# A one-member union is the member itself
assert (int | int) is int

# Members are flattened and deduplicated, order preserved
assert (int | str | int).__args__ == (int, str)
assert ((int | str) | float).__args__ == (int, str, float)
assert (str | int).__args__ == (str, int)
assert (int | None).__args__ == (int, type(None))
assert (int | type(None)) == (int | None)

# === type() of a union, and the two names for it ===
assert type(int | str) is typing.Union
assert types.UnionType is typing.Union
assert repr(type(int | str)) == "<class 'typing.Union'>"
assert (int | str).__name__ == 'Union'

# === Union equality ignores order ===
assert (int | str) == (str | int)
assert hash(int | str) == hash(str | int)
assert (int | str) != (int | float)
assert ((int | str) == int) is False

# === A union of sandbox classes ===


class Exec:
    pass


class Chunk:
    pass


both = Exec | Chunk
assert both.__args__ == (Exec, Chunk)
assert isinstance(Exec(), both)
assert isinstance(Chunk(), both)
assert not isinstance(1, both)

# === isinstance against a union ===
assert isinstance(1, int | str)
assert isinstance('a', int | str)
assert not isinstance(1.0, int | str)
assert isinstance(None, int | None)
assert isinstance(1, (float, int | bytes))
assert not isinstance(1.0, (bytes, str | int))

# === A non-type operand is an ordinary operator error ===
try:
    1 | int
    assert False, 'expected | with a non-type to fail'
except TypeError as exc:
    assert str(exc) == "unsupported operand type(s) for |: 'int' and 'type'"

# === get_origin / get_args ===
assert get_origin(list[int]) is list
assert get_args(list[int]) == (int,)
assert get_origin(dict[str, int]) is dict
assert get_args(dict[str, int]) == (str, int)
assert get_origin(int | str) is typing.Union
assert get_args(int | str) == (int, str)
assert get_origin(int) is None
assert get_args(int) == ()
assert get_origin(1) is None
assert get_args('x') == ()
assert get_origin(None) is None

# === typing.Optional and typing.Union spell out the same thing ===
assert typing.Optional[int] == (int | None)
assert repr(typing.Optional[int]) == 'int | None'
assert get_args(typing.Optional[int]) == (int, type(None))
assert typing.Union[int, str] == (int | str)
assert typing.Union[int] is int
assert typing.Union[int, int] is int

# === The special forms that take a subscript ===
assert repr(typing.Literal['a', 'b']) == "typing.Literal['a', 'b']"
assert get_origin(typing.Literal['a', 'b']) is typing.Literal
assert get_args(typing.Literal['a', 'b']) == ('a', 'b')
assert repr(typing.ClassVar[int]) == 'typing.ClassVar[int]'
assert get_origin(typing.ClassVar[int]) is typing.ClassVar
assert repr(typing.Final[int]) == 'typing.Final[int]'
assert get_origin(typing.Final[int]) is typing.Final
assert repr(typing.Annotated[int, 'meta']) == "typing.Annotated[int, 'meta']"
assert get_origin(typing.Annotated[int, 'meta']) is typing.Annotated
assert get_args(typing.Annotated[int, 'meta']) == (int, 'meta')

# A PEP 695 alias holding one of them evaluates on the first `__value__` read
type Op = typing.Literal['abort', 'resume']

assert repr(Op.__value__) == "typing.Literal['abort', 'resume']"
assert Op.__value__ is Op.__value__

# === The `types` module names the runtime types exactly ===
assert isinstance(None, types.NoneType)
assert isinstance(..., types.EllipsisType)
assert isinstance(NotImplemented, types.NotImplementedType)
assert isinstance(types, types.ModuleType)
assert type(None) is types.NoneType

# === overload keeps the last, plain definition ===


@overload
def widen(x: int) -> int: ...


@overload
def widen(x: str) -> str: ...


def widen(x):
    return x


assert widen(3) == 3
assert widen('a') == 'a'

# Calling an overload stub that was never followed by an implementation
overloaded = overload(widen)
try:
    overloaded(1)
    assert False, 'expected the overload stub to refuse a call'
except NotImplementedError as exc:
    assert str(exc) == (
        'You should not call an overloaded function. A series of @overload-decorated '
        'functions outside a stub module should always be followed by an implementation '
        'that is not @overload-ed.'
    )

# === dataclass_transform is inert at runtime ===


@dataclass_transform(frozen_default=True)
def seal(cls):
    return cls


@seal
class Sealed:
    pass


assert Sealed.__name__ == 'Sealed'
assert seal(Sealed) is Sealed
