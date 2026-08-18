# A sequence's `+` says what it can concatenate rather than that the operands
# are unsupported, and each sequence words it the way CPython does: the name is
# a literal on both sides, so it is not always the type's own.

import collections


def expect(fn, message):
    try:
        fn()
        raise AssertionError('expected TypeError')
    except TypeError as exc:
        assert str(exc) == message


expect(lambda: (1, 2) + 1, 'can only concatenate tuple (not "int") to tuple')
expect(lambda: (1, 2) + 'a', 'can only concatenate tuple (not "str") to tuple')
expect(lambda: (1, 2) + [3], 'can only concatenate tuple (not "list") to tuple')
expect(lambda: (1, 2) + None, 'can only concatenate tuple (not "NoneType") to tuple')

# The right operand is named by its full type name, dots and all.
expect(
    lambda: (1, 2) + collections.deque([3]),
    'can only concatenate tuple (not "collections.deque") to tuple',
)

# A namedtuple concatenates as a tuple, and says "tuple" rather than its own
# class name on either side.
P = collections.namedtuple('P', 'x y')
expect(lambda: P(1, 2) + 1, 'can only concatenate tuple (not "int") to tuple')
expect(lambda: P(1, 2) + [3], 'can only concatenate tuple (not "list") to tuple')

# `bytes` names the operands the other way round, and quotes neither.
expect(lambda: b'ab' + 1, "can't concat int to bytes")
expect(lambda: b'ab' + 'c', "can't concat str to bytes")
expect(lambda: b'ab' + None, "can't concat NoneType to bytes")
expect(lambda: b'ab' + [1], "can't concat list to bytes")
expect(lambda: b'ab' + collections.deque([1]), "can't concat collections.deque to bytes")

# The sequences that already read this way keep doing so.
expect(lambda: 'a' + 1, 'can only concatenate str (not "int") to str')
expect(lambda: [1] + 1, 'can only concatenate list (not "int") to list')
expect(lambda: collections.deque([1]) + 1, 'can only concatenate deque (not "int") to deque')


# === Augmented assignment reports the same concatenation wording ===
def tuple_iadd():
    a = (1, 2)
    a += 1


expect(tuple_iadd, 'can only concatenate tuple (not "int") to tuple')


def bytes_iadd():
    a = b'ab'
    a += 1


expect(bytes_iadd, "can't concat int to bytes")


def bytes_iadd_str():
    a = b'ab'
    a += 'c'


expect(bytes_iadd_str, "can't concat str to bytes")


# === A left operand that is not a sequence keeps the generic message ===
expect(lambda: 1 + (2,), "unsupported operand type(s) for +: 'int' and 'tuple'")
expect(lambda: 1 + b'ab', "unsupported operand type(s) for +: 'int' and 'bytes'")
expect(lambda: {1: 2} + 1, "unsupported operand type(s) for +: 'dict' and 'int'")
expect(lambda: {1} + 1, "unsupported operand type(s) for +: 'set' and 'int'")

# Only `+` and `+=` read as concatenation; the other operators do not.
expect(lambda: (1, 2) - 1, "unsupported operand type(s) for -: 'tuple' and 'int'")
expect(lambda: b'ab' - 1, "unsupported operand type(s) for -: 'bytes' and 'int'")
