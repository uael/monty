# `__bases__` of a class: the one it was made with, a class of the session as
# itself, and `object` for a class made with none.


class Plain:
    pass


class Sub(Plain):
    pass


# === the base a class was made with ===
assert Plain.__bases__ == (object,)
assert Sub.__bases__ == (Plain,)
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
