# Tests for multi-module import statements (import a, b, c)

# === Basic multi-module import ===
import sys, math

assert isinstance(sys.version, str)
assert math.pi > 3.14

# === Multi-module import with alias ===
import sys as s, math as m

assert isinstance(s.version, str)
assert m.pi > 3.14

# === Mixed alias and non-alias ===
import sys, math as m2

assert isinstance(sys.version, str)
assert m2.pi > 3.14

# === One module object per name ===
# every import of a module hands back the object the first import created
assert sys is s, 'repeated import of sys must be one object'
assert math is m, 'repeated import of math must be one object'
assert m is m2, 'aliases of one module must be one object'
assert sys is not math, 'different modules must be different objects'


def imported_inside():
    import sys

    return sys


assert imported_inside() is sys, 'an import in a function must reach the same object'
