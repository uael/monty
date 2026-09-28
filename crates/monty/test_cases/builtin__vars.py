# === vars() of a class ===
class Foo:
    x = 1

    def m(self):
        return self.x


assert sorted(k for k in vars(Foo) if not k.startswith('_')) == ['m', 'x']
assert vars(Foo)['x'] == 1

# === vars() of an instance ===
f = Foo()
assert vars(f) == {}
f.y = 2
assert vars(f) == {'y': 2}

# === vars() of a module ===
import sys

assert vars(sys)['maxsize'] == sys.maxsize
assert 'argv' in vars(sys)

# === vars() with no argument is locals() ===
def g():
    a = 1
    b = 'two'
    return vars()


assert g() == {'a': 1, 'b': 'two'}

# === the result is a plain dict ===
assert type(vars(f)) is dict

# === an object with no namespace ===
for one in (1, 'a', [1], (1,), {1: 2}, None):
    try:
        vars(one)
        raise AssertionError('expected TypeError')
    except TypeError as exc:
        assert str(exc) == 'vars() argument must have __dict__ attribute'

# === too many arguments ===
try:
    vars(f, f)
    raise AssertionError('expected TypeError')
except TypeError as exc:
    assert str(exc) == 'vars expected at most 1 argument, got 2'
