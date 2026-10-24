# `__bases__` of a class: the one it was made with, a class of the session as
# itself, a builtin as the type it is, and `object` for a class made with none.


class Plain:
    pass


class Boom(ValueError):
    pass


class Sub(Plain):
    pass


class Word(str):
    pass


# === the base a class was made with ===
assert Plain.__bases__ == (object,)
assert Boom.__bases__ == (ValueError,)
assert Sub.__bases__ == (Plain,)
assert Word.__bases__ == (str,)
assert Sub.__bases__[0] is Plain

# === an instance has no __bases__ of its own ===
try:
    Plain().__bases__
except AttributeError:
    pass
else:
    raise AssertionError('an instance has no __bases__')

# === the module of the class stays what it was ===
assert Sub.__module__ == Plain.__module__
