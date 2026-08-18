# `int / int` is rounded once, to the nearest representable float. Converting
# each operand first and dividing the two doubles rounds three times, and the
# three do not agree with the one.

# === Beyond the exactly-convertible range, but still immediate ints ===
assert (2**53 + 1) / 3 == 3002399751580331.0
assert (2**54 + 1) / 7 == 2573485501354569.5
assert (2**62 + 1) / 3 == 1.5372286728091292e18
assert 7 / (2**55 + 1) == 1.942890293094024e-16
assert (10**18 + 1) / (10**18 - 1) == 1.0
assert -(2**53 + 1) / 3 == -3002399751580331.0
assert (2**53 + 1) / -3 == -3002399751580331.0

# === Big ints on either side ===
assert 1 / (10**30) == 1e-30
assert 1 / (10**25) == 1e-25
assert 1 / (10**23) == 1e-23
assert 7 / (10**30) == 7e-30
assert -1 / (10**30) == -1e-30
assert (10**30) / 1 == 1e30
assert (10**30) / 7 == 1.4285714285714285e29
assert (10**100) / (3**50) == 1.3929555690985383e76
assert (3**50) / (10**100) == 7.178979876918526e-77
assert (10**400) / (10**100) == 1e300
assert (10**400) / (10**399) == 10.0
assert (10**400) / (10**400) == 1.0
assert (10**500) / (10**200) == 1e300
assert (10**200) / (10**500) == 1e-300
assert (2**1000) / (2**900) == 1.2676506002282294e30

# === Ties round to even, which needs the remainder's own sign, not a guess ===
assert (2**54 + 1) / (2**54) == 1.0
assert (2**54 + 2) / (2**54) == 1.0
assert (2**54 + 3) / (2**54) == 1.0000000000000002

# === The subnormal grid, whose spacing is fixed rather than relative ===
assert 1 / (2**1074) == 5e-324
assert 1 / (2**1075) == 0.0
assert 3 / (2**1076) == 5e-324
assert 1 / (2**1080) == 0.0
assert 1 / (10**400) == 0.0
assert 0 / (10**400) == 0.0
# A quotient too small to represent underflows to a signed zero rather than raising.
assert repr(-1 / (10**400)) == '-0.0'

# === The largest finite double, and the first quotient past it ===
assert (2**1023) / 1 == 8.98846567431158e307
assert (2**1024 - 2**971) / 1 == 1.7976931348623157e308

try:
    (2**1024 - 2**970) / 1
    assert False, 'expected OverflowError'
except OverflowError as exc:
    assert str(exc) == 'integer division result too large for a float'

try:
    (10**400) / 1
    assert False, 'expected OverflowError'
except OverflowError as exc:
    assert str(exc) == 'integer division result too large for a float'

try:
    -(10**400) / 1
    assert False, 'expected OverflowError'
except OverflowError as exc:
    assert str(exc) == 'integer division result too large for a float'

# === A zero divisor still reports division rather than overflow ===
try:
    (10**30) / 0
    assert False, 'expected ZeroDivisionError'
except ZeroDivisionError as exc:
    assert str(exc) == 'division by zero'
