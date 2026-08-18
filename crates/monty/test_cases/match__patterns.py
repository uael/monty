# PEP 634 `match` statements: every pattern kind, the guard, and the binding
# rules a case relies on.

import collections
from dataclasses import dataclass


def kind(x):
    match x:
        case None:
            return 'none'
        case True:
            return 'true'
        case 0:
            return 'zero'
        case 'hi':
            return 'greeting'
        case [1, 2]:
            return 'onetwo'
        case [a, b, c]:
            return f'three {a}{b}{c}'
        case [first, *rest]:
            return f'many {first} {rest}'
        case {'k': v}:
            return f'mapping {v}'
        case int() | float():
            return 'number'
        case str() as s:
            return f'string {s}'
        case _:
            return 'other'


assert kind(None) == 'none'
assert kind(True) == 'true'
assert kind(0) == 'zero'
assert kind('hi') == 'greeting'
assert kind([1, 2]) == 'onetwo'
assert kind([7, 8, 9]) == 'three 789'
assert kind([1, 2, 3, 4]) == 'many 1 [2, 3, 4]'
assert kind({'k': 5}) == 'mapping 5'
assert kind(42) == 'number'
assert kind(4.5) == 'number'
assert kind('yo') == 'string yo'
assert kind(()) == 'other'

# === Singletons compare with `is`, so `True` is not `1` ===


def singleton(x):
    match x:
        case True:
            return 'True'
        case False:
            return 'False'
        case None:
            return 'None'
        case 1:
            return 'one'
        case 0:
            return 'zero'
        case _:
            return 'other'


assert singleton(True) == 'True'
assert singleton(1) == 'one'
assert singleton(False) == 'False'
assert singleton(0) == 'zero'
assert singleton(None) == 'None'

# === Sequence patterns ===


def seq(x):
    match x:
        case []:
            return 'empty'
        case [a]:
            return f'one {a}'
        case [a, *mid, b]:
            return f'ends {a} {b} mid {mid}'
        case _:
            return 'other'


assert seq([]) == 'empty'
assert seq([1]) == 'one 1'
assert seq([1, 2]) == 'ends 1 2 mid []'
assert seq([1, 2, 3, 4]) == 'ends 1 4 mid [2, 3]'
assert seq((1, 2, 3)) == 'ends 1 3 mid [2]'
assert seq(range(3)) == 'ends 0 2 mid [1]'
assert seq(collections.deque([1, 2])) == 'ends 1 2 mid []'
# A str is a sequence, and deliberately not one a sequence pattern accepts
assert seq('ab') == 'other'
assert seq(b'ab') == 'other'
assert seq({'a': 1}) == 'other'
assert seq(1) == 'other'

# The star always binds a list, whatever the subject was
match (1, 2, 3):
    case [_, *tail]:
        assert tail == [2, 3]

# `*_` matches without binding
match [1, 2, 3]:
    case [head, *_]:
        assert head == 1

# === Mapping patterns ===


def mapping(x):
    match x:
        case {'a': 1, 'b': b}:
            return f'a1 b{b}'
        case {'a': a, **rest}:
            return f'a{a} rest {sorted(rest)}'
        case {}:
            return 'any mapping'
        case _:
            return 'other'


assert mapping({'a': 1, 'b': 2}) == 'a1 b2'
assert mapping({'a': 9, 'c': 3, 'd': 4}) == "a9 rest ['c', 'd']"
assert mapping({'z': 1}) == 'any mapping'
assert mapping([]) == 'other'
# A mapping pattern ignores the keys it does not name
match {'a': 1, 'b': 2}:
    case {'a': got}:
        assert got == 1

# === Class patterns ===


@dataclass
class Point:
    x: int
    y: int


class Turn:
    def __init__(self, role):
        self.role = role


def described(p):
    match p:
        case Point(0, 0):
            return 'origin'
        case Point(x=0, y=y):
            return f'on y at {y}'
        case Point(a, b) if a == b:
            return f'diagonal {a}'
        case Point(a, b):
            return f'point {a},{b}'
        case Turn() as t:
            return f'turn {t.role}'
        case _:
            return 'other'


assert described(Point(0, 0)) == 'origin'
assert described(Point(0, 5)) == 'on y at 5'
assert described(Point(3, 3)) == 'diagonal 3'
assert described(Point(1, 2)) == 'point 1,2'
assert described(Turn('user')) == 'turn user'
assert described(7) == 'other'

# A class pattern matches a subclass too


class Origin(Point):
    pass


assert described(Origin(0, 0)) == 'origin'

# The builtin classes whose single positional sub-pattern is the whole subject


def unwrapped(v):
    match v:
        case int(n):
            return ('int', n)
        case str(s):
            return ('str', s)
        case list([a, b]):
            return ('list', a, b)
        case dict(d):
            return ('dict', d)
        case _:
            return ('other',)


assert unwrapped(3) == ('int', 3)
assert unwrapped('a') == ('str', 'a')
assert unwrapped([1, 2]) == ('list', 1, 2)
assert unwrapped({'k': 1}) == ('dict', {'k': 1})
assert unwrapped(4.5) == ('other',)

# === Guards run after the pattern bound, and a failed guard tries the next case ===


def guarded(v):
    match v:
        case [a, b] if a > b:
            return 'descending'
        case [a, b] if a < b:
            return 'ascending'
        case [_, _]:
            return 'equal'
        case _:
            return 'other'


assert guarded([2, 1]) == 'descending'
assert guarded([1, 2]) == 'ascending'
assert guarded([1, 1]) == 'equal'

# === Alternatives ===


def alt(v):
    match v:
        case 1 | 2 | 3:
            return 'small'
        case [x] | (x,):
            return f'single {x}'
        case str() | bytes():
            return 'texty'
        case _:
            return 'other'


assert alt(2) == 'small'
assert alt([9]) == 'single 9'
assert alt((9,)) == 'single 9'
assert alt('a') == 'texty'
assert alt(b'a') == 'texty'
assert alt(4.5) == 'other'

# === Value patterns read a dotted name ===


class Color:
    RED = 'red'
    BLUE = 'blue'


def named(v):
    match v:
        case Color.RED:
            return 'is red'
        case Color.BLUE:
            return 'is blue'
        case _:
            return 'unnamed'


assert named('red') == 'is red'
assert named('blue') == 'is blue'
assert named('green') == 'unnamed'

# === A subject expression is evaluated exactly once ===

calls = []


def subject():
    calls.append(1)
    return [1, 2]


match subject():
    case [1, 2]:
        pass
assert calls == [1]

# === The walrus binds the subject too ===
pair = [1, 2]
match kept := pair:
    case [a, b]:
        assert kept is pair
        assert (a, b) == (1, 2)

# === Nested matches ===


def nested(v):
    match v:
        case [x]:
            match x:
                case 1:
                    return 'one'
                case _:
                    return 'inner other'
        case _:
            return 'outer other'


assert nested([1]) == 'one'
assert nested([2]) == 'inner other'
assert nested(3) == 'outer other'

# === A case body may return out of the match ===


def early(v):
    match v:
        case [x, *_]:
            return x
    return 'fell through'


assert early([5, 6]) == 5
assert early(1) == 'fell through'

# === ...and break out of a loop around it ===

seen = []
for item in [1, 2, 3, 4]:
    match item:
        case 3:
            break
        case n:
            seen.append(n)
assert seen == [1, 2]

# === No case matching falls through with nothing bound ===

ran = False
match 99:
    case 1:
        ran = True
    case 2:
        ran = True
assert ran is False

# === A class pattern's errors ===


class Plain:
    pass


try:
    match Plain():
        case Plain(1):
            pass
    assert False, 'expected a positional sub-pattern to be refused'
except TypeError as exc:
    assert str(exc) == 'Plain() accepts 0 positional sub-patterns (1 given)'


class BadArgs:
    __match_args__ = 'x'
    x = 1


try:
    match BadArgs():
        case BadArgs(1):
            pass
    assert False, 'expected a non-tuple __match_args__ to be refused'
except TypeError as exc:
    assert str(exc) == 'BadArgs.__match_args__ must be a tuple (got str)'


class OneArg:
    __match_args__ = ('x',)
    x = 1


try:
    match OneArg():
        case OneArg(1, x=2):
            pass
    assert False, 'expected a duplicated sub-pattern to be refused'
except TypeError as exc:
    assert str(exc) == "OneArg() got multiple sub-patterns for attribute 'x'"

not_a_class = 5
try:
    match 1:
        case not_a_class():
            pass
    assert False, 'expected a non-class pattern to be refused'
except TypeError as exc:
    assert str(exc) == 'called match pattern must be a class'

# A named attribute the subject does not have is a failed match, not an error


class TwoArgs:
    __match_args__ = ('a', 'b')
    a = 1


match TwoArgs():
    case TwoArgs(1, 2):
        assert False, 'expected the missing attribute to fail the match'
    case _:
        pass
