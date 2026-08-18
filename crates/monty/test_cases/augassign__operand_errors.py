# An augmented assignment whose left operand has no in-place form falls back to
# the binary operation, but it still names itself: CPython reports `-=`, never
# the `-` it borrowed to do the work.


def expect(fn, message):
    try:
        fn()
        raise AssertionError('expected TypeError')
    except TypeError as exc:
        assert str(exc) == message


# === One per operator ===
def isub():
    a = 5
    a -= 'x'


expect(isub, "unsupported operand type(s) for -=: 'int' and 'str'")


def imul():
    a = 1.5
    a *= None


expect(imul, "unsupported operand type(s) for *=: 'float' and 'NoneType'")


def idiv():
    a = 5
    a /= 'x'


expect(idiv, "unsupported operand type(s) for /=: 'int' and 'str'")


def ifloordiv():
    a = 5
    a //= 'x'


expect(ifloordiv, "unsupported operand type(s) for //=: 'int' and 'str'")


def imod():
    a = 1.5
    a %= 'x'


expect(imod, "unsupported operand type(s) for %=: 'float' and 'str'")


# `**` names itself `** or pow()` when it fails as a binary operator, because
# `pow()` shares the slot; the augmented form has only the one spelling.
def ipow():
    a = 5
    a **= 'x'


expect(ipow, "unsupported operand type(s) for **=: 'int' and 'str'")


def iand():
    a = 1.5
    a &= 1


expect(iand, "unsupported operand type(s) for &=: 'float' and 'int'")


def ior():
    a = 1.5
    a |= 1


expect(ior, "unsupported operand type(s) for |=: 'float' and 'int'")


def ixor():
    a = 1.5
    a ^= 1


expect(ixor, "unsupported operand type(s) for ^=: 'float' and 'int'")


def ilshift():
    a = 1.5
    a <<= 1


expect(ilshift, "unsupported operand type(s) for <<=: 'float' and 'int'")


def irshift():
    a = 1.5
    a >>= 1


expect(irshift, "unsupported operand type(s) for >>=: 'float' and 'int'")


# === A type with a real in-place form reports the augmented symbol too, once
# that form has declined the operand ===
def set_iand():
    a = {1}
    a &= 1


expect(set_iand, "unsupported operand type(s) for &=: 'set' and 'int'")


def set_isub():
    a = {1}
    a -= 1


expect(set_isub, "unsupported operand type(s) for -=: 'set' and 'int'")


# === The plain operators keep their own spelling ===
def binary_sub():
    5 - 'x'


expect(binary_sub, "unsupported operand type(s) for -: 'int' and 'str'")


def binary_mod():
    1.5 % 'x'


expect(binary_mod, "unsupported operand type(s) for %: 'float' and 'str'")


def binary_pow():
    5 ** 'x'


expect(binary_pow, "unsupported operand type(s) for ** or pow(): 'int' and 'str'")


def builtin_pow():
    pow(5, 'x')


expect(builtin_pow, "unsupported operand type(s) for ** or pow(): 'int' and 'str'")


# === A subscript and an attribute target compile to the same opcode by a
# different route, so each is asked separately ===
def subscript_target():
    d = {'k': 5}
    d['k'] -= 'x'


expect(subscript_target, "unsupported operand type(s) for -=: 'int' and 'str'")


class Holder:
    def __init__(self):
        self.v = 5


def attribute_target():
    h = Holder()
    h.v %= 'x'


expect(attribute_target, "unsupported operand type(s) for %=: 'int' and 'str'")
