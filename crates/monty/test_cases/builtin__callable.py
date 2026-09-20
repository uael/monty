# === Functions, lambdas and methods ===
import math
import re
from collections import namedtuple
from functools import partial


def plain(a=1):
    return a


def capturing():
    held = 1
    return lambda: held


assert callable(plain) == True
assert callable(capturing) == True
assert callable(capturing()) == True
assert callable(lambda: 1) == True


# === Classes and their instances ===
class Holder:
    def method(self):
        return 1


holder = Holder()
assert callable(Holder) == True
assert callable(Holder.method) == True
assert callable(holder.method) == True
assert callable(holder) == False

# === Builtins, types and exception classes ===
assert callable(len) == True
assert callable(print) == True
assert callable(int) == True
assert callable(list) == True
assert callable(type) == True
assert callable(ValueError) == True
assert callable(ValueError('boom')) == False

# === Values that are not callable ===
assert callable(None) == False
assert callable(1) == False
assert callable(1.5) == False
assert callable('len') == False
assert callable(b'len') == False
assert callable([len]) == False
assert callable({'len': len}) == False
assert callable((len,)) == False
assert callable(range(3)) == False
assert callable(math) == False
assert callable(...) == False
assert callable(NotImplemented) == False


# === Iterators and generators: the factory is callable, the object is not ===
def counter():
    yield 1


assert callable(counter) == True
assert callable(counter()) == False
assert callable(iter([1])) == False
assert callable(i for i in [1]) == False

# === Library objects ===
Point = namedtuple('Point', 'x y')
assert callable(Point) == True
assert callable(Point(1, 2)) == False
assert callable(partial(Point, 1)) == True
assert callable(re.compile('a')) == False
assert callable(re.compile('a').match('a')) == False
assert callable(list[int]) == True

# === Arity ===
try:
    callable()
    assert False, 'expected a TypeError'
except TypeError as exc:
    assert str(exc) == 'callable() takes exactly one argument (0 given)'

try:
    callable(len, len)
    assert False, 'expected a TypeError'
except TypeError as exc:
    assert str(exc) == 'callable() takes exactly one argument (2 given)'
