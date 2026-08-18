# A shift count is checked in three steps, and the order is what decides the
# answer: the sign first, then the value being shifted, and only then the count's
# magnitude. Both shifts share the first two; they part on the third.


def expect(fn, exc_type, message):
    try:
        fn()
        raise AssertionError('expected an exception')
    except exc_type as exc:
        assert str(exc) == message


# === A negative count is refused before anything else is looked at ===
expect(lambda: 5 << -1, ValueError, 'negative shift count')
expect(lambda: 5 >> -1, ValueError, 'negative shift count')
expect(lambda: 0 << -1, ValueError, 'negative shift count')
expect(lambda: 0 >> -1, ValueError, 'negative shift count')
expect(lambda: 0 << -(10**30), ValueError, 'negative shift count')
expect(lambda: (10**30) >> -1, ValueError, 'negative shift count')

# === Shifting zero answers zero, however unnameable the count ===
assert 0 << (10**30) == 0
assert 0 >> (10**30) == 0
assert 0 << (2**63) == 0
assert 0 << (2**64) == 0
assert False << (10**30) == 0

# === `<<` has no representable answer for a count past C ssize_t ===
expect(lambda: 1 << (10**30), OverflowError, 'too many digits in integer')
expect(lambda: -1 << (10**30), OverflowError, 'too many digits in integer')
expect(lambda: 1 << (2**63), OverflowError, 'too many digits in integer')
expect(lambda: 1 << (2**64), OverflowError, 'too many digits in integer')
expect(lambda: (10**30) << (10**30), OverflowError, 'too many digits in integer')
expect(lambda: True << (10**30), OverflowError, 'too many digits in integer')

# === `>>` always has one: everything is shifted out, leaving the sign ===
assert 1 >> (10**30) == 0
assert 5 >> (10**30) == 0
assert -1 >> (10**30) == -1
assert -5 >> (10**30) == -1
assert (10**30) >> (10**30) == 0
assert -(10**30) >> (10**30) == -1
assert (2**200) >> (2**64) == 0
assert -(2**200) >> (2**64) == -1

# === Counts that do fit still shift ===
assert 1 << 64 == 18446744073709551616
assert (10**30) << 1 == 2000000000000000000000000000000
assert (2**200) >> 100 == 2**100
assert -(2**200) >> 100 == -(2**100)
assert 0 << 0 == 0
assert 5 >> 200 == 0


# === The augmented spellings take the same path ===
def zero_ishift():
    a = 0
    a <<= 10**30
    return a


assert zero_ishift() == 0


def one_ishift():
    a = 1
    a <<= 10**30


expect(one_ishift, OverflowError, 'too many digits in integer')


def rshift_saturates():
    a = -(10**30)
    a >>= 10**30
    return a


assert rshift_saturates() == -1
