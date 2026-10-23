from typing import Literal, Optional, Union
import copy

# === Building literal aliases ===
one = Literal['a']
assert repr(type(one)) == "<class 'typing._LiteralGenericAlias'>"
assert type(one).__name__ == '_LiteralGenericAlias'
assert type(Literal[1]) is type(Literal['a', 'b'])
assert one.__origin__ is Literal
assert one.__args__ == ('a',)
assert one.__parameters__ == ()
assert one.__name__ == 'Literal'
assert one.__qualname__ == 'Literal'
assert Literal['a', 'b'].__args__ == ('a', 'b')
assert Literal[('a', 'b')].__args__ == ('a', 'b')
assert Literal[()].__args__ == ()
assert Literal[(3,), 4].__args__ == ((3,), 4)
assert Literal[None].__args__ == (None,)

# === Flattening and deduplication ===
assert Literal[1, 1].__args__ == (1,)
assert Literal[1, True].__args__ == (1, True)
assert Literal[1, 1.0].__args__ == (1, 1.0)
assert Literal[True, 1, True].__args__ == (True, 1)
assert Literal[Literal[1, 2], 3].__args__ == (1, 2, 3)
assert Literal[1, Literal[2, 1], (3,)].__args__ == (1, 2, (3,))
assert Literal[Literal[1], Literal[2, 1]].__args__ == (1, 2)
assert Literal[Literal[1, 2]] == Literal[1, 2]
# an unhashable value is deduplicated too
assert Literal[[1], [1], 2, 2].__args__ == ([1], 2)
assert Literal[[1], 2, [1], True].__args__ == ([1], 2, True)

# === repr ===
assert repr(Literal['a', 'b']) == "typing.Literal['a', 'b']"
assert repr(Literal[None]) == 'typing.Literal[None]'
assert repr(Literal[1, True]) == 'typing.Literal[1, True]'
assert repr(Literal[()]) == 'typing.Literal[()]'
assert repr(Literal[int]) == 'typing.Literal[int]'
assert repr(Literal[list[int]]) == 'typing.Literal[list[int]]'
assert repr(Literal[...]) == 'typing.Literal[...]'
assert str(Literal['a']) == "typing.Literal['a']"
assert f'{Literal[1, 2]}' == 'typing.Literal[1, 2]'

# === Equality and hash ===
assert Literal[1, 2] == Literal[2, 1]
assert Literal[1] != Literal[True]
assert Literal[1, 0] != Literal[True, False]
assert Literal[1] == Literal[1, 1]
assert Literal['a'] != Literal['a', 'b']
assert Literal[1] != 1
assert Literal[1] != list[int]
assert hash(Literal[1, 2]) == hash(Literal[2, 1])
assert hash(Literal['a']) == hash(Literal['a'])
assert {Literal[1, 2]: 'x'}[Literal[2, 1]] == 'x'
assert len({Literal[1], Literal[True], Literal[1, 1]}) == 2

try:
    hash(Literal[[1]])
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "cannot use 'tuple' as a set element (unhashable type: 'list')"

try:
    Literal[[1]] == Literal[[1]]
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "cannot use 'tuple' as a set element (unhashable type: 'list')"

# === Unions ===
maybe = Literal['a'] | None
assert type(maybe) is Union
assert maybe.__args__ == (Literal['a'], type(None))
assert repr(maybe) == "typing.Literal['a'] | None"
assert repr(None | Literal['a']) == "None | typing.Literal['a']"
assert repr(Literal['a'] | int) == "typing.Literal['a'] | int"
assert repr(int | Literal['a']) == "int | typing.Literal['a']"
assert repr(Literal['a'] | Literal['b']) == "typing.Literal['a'] | typing.Literal['b']"
assert repr(Literal['a'] | (int | str)) == "typing.Literal['a'] | int | str"
assert repr((int | str) | Literal['a']) == "int | str | typing.Literal['a']"
assert repr(Literal['a'] | list[int]) == "typing.Literal['a'] | list[int]"
assert Literal['a'] | Literal['a'] == Literal['a']
assert repr(Literal['a'] | 1) == "typing.Literal['a'] | 1"
assert repr(1 | Literal['a']) == "1 | typing.Literal['a']"
assert Literal['a'] | None == None | Literal['a']
assert repr(Optional[Literal['a']]) == "typing.Literal['a'] | None"
assert repr(Union[Literal['a'], None]) == "typing.Literal['a'] | None"

# === Members of a union, as a reader of a shape walks them ===
shape = Literal['a', 'b'] | None
origins = [getattr(x, '__origin__', x) for x in shape.__args__]
assert origins == [Literal, type(None)]
assert shape.__args__[0].__args__ == ('a', 'b')

# === Errors ===
try:
    Literal['a']['b']  # pyright: ignore
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "typing.Literal['a'] is not a generic class"

try:
    Literal['a']()
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == 'Cannot instantiate typing.Literal'

try:
    isinstance(1, Literal[1])
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == 'Subscripted generics cannot be used with class and instance checks'

assert bool(Literal['a'])

# === copy ===
assert copy.copy(Literal['a', 'b']) == Literal['a', 'b']
assert copy.deepcopy(Literal['a', 'b']) == Literal['a', 'b']
nested = [Literal[1, 2]]
assert copy.deepcopy(nested) == [Literal[1, 2]]
