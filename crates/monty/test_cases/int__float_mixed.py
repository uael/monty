# An int mixed with a float is converted to a float and the float operation
# done, whatever the int's size. The conversion is the first thing that happens,
# so an int past the float range fails the operation rather than saturating to an
# infinity the arithmetic would go on to carry.

mid = 2**64
huge = 10**400


def expect(fn, exc_type, message):
    try:
        fn()
        raise AssertionError('expected an exception')
    except exc_type as exc:
        assert str(exc) == message


# === A big int that does convert takes every operator ===
assert mid + 1.5 == 1.8446744073709552e19
assert 1.5 + mid == 1.8446744073709552e19
assert mid - 1.5 == 1.8446744073709552e19
assert 1.5 - mid == -1.8446744073709552e19
assert mid * 1.5 == 2.7670116110564327e19
assert 1.5 * mid == 2.7670116110564327e19
assert mid / 1.5 == 1.2297829382473034e19
assert 1.5 / mid == 8.131516293641283e-20
assert mid % 1.5 == 1.0
assert 1.5 % mid == 1.5
assert mid // 1.5 == 1.2297829382473034e19
assert 1.5 // mid == 0.0
assert divmod(mid, 1.5) == (1.2297829382473034e19, 1.0)
assert divmod(1.5, mid) == (0.0, 1.5)
assert mid**1.5 == 7.922816251426434e28
assert 0.5**mid == 0.0
assert pow(0.5, mid) == 0.0
assert pow(mid, -1.0) == 5.421010862427522e-20

# The remainder's sign follows the divisor, as it does for any float pair.
assert -mid % 1.5 == 0.5
assert mid % -1.5 == -0.5
assert -1.5 % mid == 1.8446744073709552e19
assert divmod(-mid, 1.5) == (-1.2297829382473034e19, 0.5)

# `divmod` agrees with the two operators it is defined as.
assert divmod(mid, 1.5) == (mid // 1.5, mid % 1.5)
assert divmod(-mid, 1.5) == (-mid // 1.5, -mid % 1.5)
assert divmod(mid, -1.5) == (mid // -1.5, mid % -1.5)

# === An int past the float range fails the operation ===
CONVERT = 'int too large to convert to float'
expect(lambda: huge + 1.5, OverflowError, CONVERT)
expect(lambda: 1.5 + huge, OverflowError, CONVERT)
expect(lambda: huge - 1.5, OverflowError, CONVERT)
expect(lambda: 1.5 - huge, OverflowError, CONVERT)
expect(lambda: huge * 1.5, OverflowError, CONVERT)
expect(lambda: 1.5 * huge, OverflowError, CONVERT)
expect(lambda: huge / 1.5, OverflowError, CONVERT)
expect(lambda: 1.5 / huge, OverflowError, CONVERT)
expect(lambda: huge % 1.5, OverflowError, CONVERT)
expect(lambda: 1.5 % huge, OverflowError, CONVERT)
expect(lambda: huge // 1.5, OverflowError, CONVERT)
expect(lambda: 1.5 // huge, OverflowError, CONVERT)
expect(lambda: divmod(huge, 1.5), OverflowError, CONVERT)
expect(lambda: divmod(1.5, huge), OverflowError, CONVERT)
expect(lambda: huge**1.5, OverflowError, CONVERT)
expect(lambda: 0.5**huge, OverflowError, CONVERT)
expect(lambda: pow(huge, 1.5), OverflowError, CONVERT)
expect(lambda: pow(0.5, huge), OverflowError, CONVERT)
expect(lambda: -huge + 1.5, OverflowError, CONVERT)
expect(lambda: huge**-1, OverflowError, CONVERT)

# The largest int that still converts, and the first that does not.
assert (2**1023) + 0.0 == 8.98846567431158e307
expect(lambda: (2**1024) + 0.0, OverflowError, CONVERT)

# === The conversion happens before the divisor is looked at ===
# A convertible int over zero is a division error; an unconvertible one never
# gets that far.
expect(lambda: mid / 0.0, ZeroDivisionError, 'division by zero')
expect(lambda: mid % 0.0, ZeroDivisionError, 'division by zero')
expect(lambda: mid // 0.0, ZeroDivisionError, 'division by zero')
expect(lambda: divmod(mid, 0.0), ZeroDivisionError, 'division by zero')
expect(lambda: huge / 0.0, OverflowError, CONVERT)
expect(lambda: huge % 0.0, OverflowError, CONVERT)
expect(lambda: huge // 0.0, OverflowError, CONVERT)
expect(lambda: divmod(huge, 0.0), OverflowError, CONVERT)
expect(lambda: 0.0**-mid, ZeroDivisionError, 'zero to a negative power')
expect(lambda: 0.0**-huge, OverflowError, CONVERT)


# === The augmented spellings reach the same arms ===
def imod():
    a = mid
    a %= 1.5
    return a


assert imod() == 1.0


def ifloordiv_huge():
    a = huge
    a //= 1.5


expect(ifloordiv_huge, OverflowError, CONVERT)


# === Comparison does not convert, so it answers for any size ===
assert (huge > 1e308) is True
assert (huge == float('inf')) is False
assert (float('inf') > huge) is True
assert (huge < float('inf')) is True
assert (float('nan') == huge) is False
