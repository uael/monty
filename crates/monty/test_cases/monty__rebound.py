# xfail=cpython
# `monty.rebound(x, source, target, memo=None)`: a deep copy of `x` in which
# what was made under the globals dict `source` is made again under `target`.
# CPython has no `monty` module, so this runs on Monty alone.
import monty

# === a function made under a dict is made again under another, and reads it ===
a = {}
exec('def f():\n    return k\nk = 1', a)
b = {'k': 2}
copied = monty.rebound({'f': a['f']}, a, b)
assert copied['f']() == 2
assert a['f']() == 1
assert copied['f'] is not a['f']

# === the copy is one function, however many hold it ===
copied = monty.rebound([a['f'], a['f']], a, b)
assert copied[0] is copied[1]

# === what was made under another dict is shared ===
c = {}
exec('def g():\n    return 3', c)
assert monty.rebound({'g': c['g']}, a, b)['g'] is c['g']


# === a plain function of the module is shared ===
def plain():
    return 4


assert monty.rebound({'p': plain}, a, b)['p'] is plain

# === a class is made again: distinct, with attributes of its own, its methods read the target ===
exec('class Plan:\n    n = 21\n\n    def m(self):\n        return k\n\n\np = Plan()', a)
copied = monty.rebound({'Plan': a['Plan'], 'p': a['p']}, a, b)
assert copied['Plan'] is not a['Plan']
assert copied['Plan'].n == 21
copied['Plan'].n = 5
assert a['Plan'].n == 21
assert copied['Plan']().m() == 2
assert a['p'].m() == 1

# === an instance of a class made again is one of the copy ===
assert isinstance(copied['p'], copied['Plan'])
assert not isinstance(copied['p'], a['Plan'])
assert copied['p'].m() == 2

# === a class and its base are made again together, and a subclass alone brings its base ===
exec('class Sub(Plan):\n    pass', a)
copied = monty.rebound({'Sub': a['Sub'], 'Plan': a['Plan']}, a, b)
assert copied['Sub'] is not a['Sub'] and copied['Plan'] is not a['Plan']
assert isinstance(copied['Sub'](), copied['Plan'])
alone = monty.rebound({'Sub': a['Sub']}, a, b)
assert not isinstance(alone['Sub'](), a['Plan'])
assert alone['Sub']().m() == 2

# === a closure takes cells of its own, so the two count apart ===
exec(
    'def counter():\n    n = 0\n\n    def bump():\n        nonlocal n\n        n += 1\n        return n\n\n    return bump\n\n\nbump = counter()',
    a,
)
assert a['bump']() == 1
copied = monty.rebound({'bump': a['bump']}, a, b)
assert copied['bump']() == 2
assert copied['bump']() == 3
assert a['bump']() == 2

# === a function that names itself names its copy ===
exec('def outer():\n    def fact(n):\n        return 1 if n < 2 else n * fact(n - 1)\n\n    return fact\n\n\nfact = outer()', a)
copied = monty.rebound({'fact': a['fact']}, a, b)
assert copied['fact'](5) == 120
assert copied['fact'] is not a['fact']

# === two closures over one cell share its copy ===
exec(
    'def pair():\n    n = 0\n\n    def up():\n        nonlocal n\n        n += 1\n        return n\n\n    def read():\n        return n\n\n    return up, read\n\n\nup, read = pair()',
    a,
)
copied = monty.rebound({'up': a['up'], 'read': a['read']}, a, b)
assert copied['up']() == 1
assert copied['read']() == 1
assert a['read']() == 0

# === a module is shared, and so is a value of the memo ===
exec('import math', a)
shared = [1]
copied = monty.rebound({'math': a['math'], 's': shared}, a, b, {id(shared): shared})
assert copied['math'] is a['math']
assert copied['s'] is shared

# === a dataclass keeps its options ===
exec('from dataclasses import dataclass\n\n\n@dataclass(frozen=True)\nclass Point:\n    x: int\n\n\nq = Point(1)', a)
copied = monty.rebound({'Point': a['Point'], 'q': a['q']}, a, b)
assert copied['q'].x == 1
assert copied['Point'](2).x == 2
assert copied['Point'](2) == copied['Point'](2)
try:
    copied['q'].x = 3
except Exception as no:
    assert type(no).__name__ == 'FrozenInstanceError', type(no).__name__
else:
    raise AssertionError('the copy of a frozen dataclass is frozen')

# === a property reads through the copy ===
exec('class Reader:\n    @property\n    def v(self):\n        return k', a)
copied = monty.rebound({'Reader': a['Reader']}, a, b)
assert copied['Reader']().v == 2
assert a['Reader']().v == 1

# === a bound method is bound to the copy of its instance, over the copy of its function ===
exec('m = p.m', a)
copied = monty.rebound({'m': a['m'], 'p': a['p']}, a, b)
assert copied['m']() == 2

# === a partial over a function made again is made again ===
exec('import functools\npf = functools.partial(f)', a)
copied = monty.rebound({'pf': a['pf']}, a, b)
assert copied['pf']() == 2
assert copied['pf'].func is not a['f']

# === a class holding an instance of itself resolves to the copy ===
exec('class Node:\n    pass\n\n\nNode.root = Node()', a)
copied = monty.rebound({'Node': a['Node']}, a, b)
assert isinstance(copied['Node'].root, copied['Node'])

# === made again twice: back into a namespace, it reads that namespace ===
home = {}
home.update(monty.rebound({'f': a['f'], 'Plan': a['Plan']}, a, home))
d = {'k': 4}
d.update(monty.rebound(home, home, d))
assert d['f']() == 4
assert d['Plan']().m() == 4
assert home['f'] is not d['f'] and home['f'] is not a['f']

# === what cannot be copied is refused as deepcopy refuses it ===
exec('g = (n for n in range(3))', a)
try:
    monty.rebound({'g': a['g']}, a, b)
except TypeError as no:
    assert str(no) == "cannot pickle 'generator' object", str(no)
else:
    raise AssertionError('a generator cannot be made again')

# === the source and the target are dicts ===
try:
    monty.rebound({}, 1, b)
except TypeError as no:
    assert str(no) == 'rebound() source must be a dict, not int', str(no)
else:
    raise AssertionError('source')
try:
    monty.rebound({}, a, None)
except TypeError as no:
    assert str(no) == 'rebound() target must be a dict, not NoneType', str(no)
else:
    raise AssertionError('target')

# === the module is one the interpreter names ===
import sys

assert 'monty' in sys.builtin_module_names
