# === __getitem__ / __setitem__ / __delitem__ ===
class Store:
    def __init__(self):
        self.items = {}

    def __getitem__(self, key):
        return self.items[key]

    def __setitem__(self, key, value):
        self.items[key] = value

    def __delitem__(self, key):
        del self.items[key]

    def __len__(self):
        return len(self.items)


s = Store()
s['a'] = 1
s['b'] = 2
assert s['a'] == 1
assert s['b'] == 2
assert len(s) == 2
del s['a']
assert len(s) == 1
assert 'a' not in s.items


# A class defining no `__delitem__` refuses `del obj[k]`.
class NoDelete:
    def __getitem__(self, key):
        return key


try:
    del NoDelete()[0]
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "'NoDelete' object doesn't support item deletion"


# A slice reaches the dunder as a slice object.
class Sliced:
    def __getitem__(self, key):
        return (key.start, key.stop, key.step)


assert Sliced()[2:5] == (2, 5, None)
assert Sliced()[::2] == (None, None, 2)


# An exception raised inside the dunder propagates.
class Raising:
    def __getitem__(self, key):
        raise KeyError(key)


try:
    Raising()['nope']
    assert False, 'expected KeyError'
except KeyError as exc:
    assert exc.args == ('nope',)


# === __call__ ===
class Adder:
    def __init__(self, base):
        self.base = base

    def __call__(self, n, extra=0):
        return self.base + n + extra


add = Adder(10)
assert add(1) == 11
assert add(1, 2) == 13
assert add(n=5) == 15


# === __bool__ and __len__ truthiness ===
class Flagged:
    def __init__(self, on):
        self.on = on

    def __bool__(self):
        return self.on


assert Flagged(True)
assert not Flagged(False)
assert bool(Flagged(False)) is False


class Sized:
    def __init__(self, n):
        self.n = n

    def __len__(self):
        return self.n


assert Sized(3)
assert not Sized(0)
assert len(Sized(7)) == 7


# `__bool__` wins over `__len__`.
class Both:
    def __bool__(self):
        return True

    def __len__(self):
        return 0


assert Both()


# An instance with neither is always truthy.
class Plain:
    pass


assert Plain()


# === Inherited dunders ===
class BaseStore(Store):
    pass


inherited = BaseStore()
inherited['k'] = 9
assert inherited['k'] == 9
assert len(inherited) == 1

# === Missing dunders raise CPython's messages ===
try:
    Plain()[0]
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "'Plain' object is not subscriptable"

try:
    Plain()[0] = 1
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "'Plain' object does not support item assignment"

try:
    len(Plain())
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "object of type 'Plain' has no len()"

try:
    Plain()()
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "'Plain' object is not callable"
