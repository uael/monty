from collections import Counter, defaultdict

# === `|` builds a new dict, leaving both operands alone ===
a = {'a': 1, 'b': 2}
b = {'b': 3, 'c': 4}
merged = a | b
assert merged == {'a': 1, 'b': 3, 'c': 4}
assert list(merged.items()) == [('a', 1), ('b', 3), ('c', 4)]
assert merged is not a
assert merged is not b
assert type(merged) is dict
assert a == {'a': 1, 'b': 2}
assert b == {'b': 3, 'c': 4}

# === A collision takes the right value at the left's position ===
assert list(({'x': 1, 'y': 2, 'z': 3} | {'z': 30, 'y': 20, 'w': 40}).items()) == [
    ('x', 1),
    ('y', 20),
    ('z', 30),
    ('w', 40),
]
# which is exactly what `{**left, **right}` builds
assert (a | b) == {**a, **b}
assert list((a | b).keys()) == list({**a, **b}.keys())

# === Empty operands and self-union ===
assert {} | {} == {}
assert {'a': 1} | {} == {'a': 1}
assert {} | {'a': 1} == {'a': 1}
same = {'a': 1, 'b': 2}
assert list((same | same).items()) == [('a', 1), ('b', 2)]

# === Values are shared with the operands, not copied ===
inner = [1, 2]
assert ({} | {'k': inner})['k'] is inner

# === Non-string keys ===
assert {1: 'a', 2: 'b'} | {2: 'c', 3: 'd'} == {1: 'a', 2: 'c', 3: 'd'}
assert {(1, 2): 'a'} | {(1, 2): 'b', (3,): 'c'} == {(1, 2): 'b', (3,): 'c'}

# === `|` needs a dict on both sides ===
for right, right_name in (([('b', 2)], 'list'), ('ab', 'str'), (3, 'int'), (None, 'NoneType'), ({1, 2}, 'set')):
    try:
        {'a': 1} | right
        assert False, 'expected TypeError'
    except TypeError as e:
        assert str(e) == f"unsupported operand type(s) for |: 'dict' and '{right_name}'"

for left, left_name in (([('b', 2)], 'list'), ('ab', 'str'), (3, 'int')):
    try:
        left | {'a': 1}
        assert False, 'expected TypeError'
    except TypeError as e:
        assert str(e) == f"unsupported operand type(s) for |: '{left_name}' and 'dict'"

# === `|=` mutates the left operand and keeps its identity ===
d = {'x': 1}
target = d
d |= {'y': 2, 'x': 10}
assert d is target
assert list(d.items()) == [('x', 10), ('y', 2)]

# an alias sees `|=`, where `d = d | other` rebinds and leaves the alias behind
aliased = {'x': 1}
alias = aliased
aliased |= {'y': 2}
assert alias == {'x': 1, 'y': 2}
rebound = {'x': 1}
rebound_alias = rebound
rebound = rebound | {'y': 2}
assert rebound_alias == {'x': 1}

# === `|=` takes everything `update` takes ===
d = {'x': 1}
d |= [('y', 2), ('z', 3)]
assert d == {'x': 1, 'y': 2, 'z': 3}

d = {'x': 1}
d |= (('y', 2),)
assert d == {'x': 1, 'y': 2}

d = {'x': 1}
d |= {'y': 2}.items()
assert d == {'x': 1, 'y': 2}

d = {'x': 1}
d |= []
assert d == {'x': 1}

d = {'a': 1}
d |= d
assert d == {'a': 1}

# === `|=` raises `update`'s error for a non-iterable, not the operator's ===
try:
    d = {'k': 0}
    d |= 3
    assert False, 'expected TypeError'
except TypeError as e:
    assert str(e) == "'int' object is not iterable"

try:
    d = {'k': 0}
    d |= None
    assert False, 'expected TypeError'
except TypeError as e:
    assert str(e) == "'NoneType' object is not iterable"

# === A defaultdict operand rebuilds through its own type, carrying its factory ===
dd = defaultdict(int, {'a': 1, 'shared': 100})
plain = {'shared': 7, 'p': 0}
assert type(dd | plain) is defaultdict
assert type(plain | dd) is defaultdict
assert (dd | plain).default_factory is int
assert (plain | dd).default_factory is int
assert list((dd | plain).items()) == [('a', 1), ('shared', 7), ('p', 0)]
assert list((plain | dd).items()) == [('shared', 100), ('p', 0), ('a', 1)]
assert dd == {'a': 1, 'shared': 100}

# the left factory wins when both sides have one
dd2 = defaultdict(list, {'x': 1})
assert (dd | dd2).default_factory is int
assert (dd2 | dd).default_factory is list

# `|=` keeps the left operand, so a defaultdict stays one, factory included
dd3 = defaultdict(int, {'a': 1})
dd3_target = dd3
dd3 |= {'b': 2}
assert dd3 is dd3_target
assert type(dd3) is defaultdict
assert dd3.default_factory is int
dd3 |= [('c', 3)]
assert dd3 == {'a': 1, 'b': 2, 'c': 3}

# === Counter keeps its multiset `|`; mixing with a plain dict degrades to dict ===
c1 = Counter(a=3, b=1)
c2 = Counter(a=1, b=4, c=2)
assert type(c1 | c2) is Counter
assert c1 | c2 == Counter(a=3, b=4, c=2)

mixed = c1 | {'a': 10}
assert type(mixed) is dict
assert mixed == {'a': 10, 'b': 1}
mixed = {'a': 10} | c1
assert type(mixed) is dict
assert mixed == {'a': 3, 'b': 1}

# a defaultdict on either side still wins over a Counter
assert type(c1 | defaultdict(int)) is defaultdict
assert type(defaultdict(int) | c1) is defaultdict

# `c |= mapping` stays the Counter's multiset max, not a dict update
c3 = Counter(a=9, b=1)
c3_target = c3
c3 |= {'a': 5, 'c': 2}
assert c3 is c3_target
assert type(c3) is Counter
assert c3 == Counter(a=9, b=1, c=2)

# `|=` reads the right operand as a plain mapping whatever kind it is, so a
# plain dict on the left stays a plain dict taking the right's values verbatim
p1 = {'a': 9, 'z': 0}
p1 |= Counter(a=5, c=2)
assert type(p1) is dict
assert list(p1.items()) == [('a', 5), ('z', 0), ('c', 2)]
p2 = {'a': 9}
p2 |= defaultdict(list, {'b': 2})
assert type(p2) is dict
assert p2 == {'a': 9, 'b': 2}
