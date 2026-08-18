# === Boolean 'and' operator ===
# returns first falsy value, or last value if all truthy
assert (5 and 3) == 3
assert (0 and 3) == 0
assert (1 and 2 and 3) == 3

# === Boolean 'or' operator ===
# returns first truthy value, or last value if all falsy
assert (5 or 3) == 5
assert (0 or 3) == 3
assert (0 or 0 or 3) == 3

# === Boolean 'not' operator ===
assert (not 5) == False
assert (not 0) == True
assert (not None) == True

# === Complex boolean expressions ===
assert ((1 and 2) or (3 and 0)) == 2
assert (not (0 and 1)) == True

# === Boolean bitwise operators ===
assert (True & True) == True
assert (True & False) == False
assert (False | True) == True
assert (False | False) == False
assert (True ^ False) == True
assert (True ^ True) == False
assert type(True & True) == bool
assert type(False | True) == bool
assert type(True ^ False) == bool

# Mixing bool and int uses integer bitwise operations.
assert type(True & 1) == int
assert type(1 | False) == int
assert type(True ^ 1) == int

# === bool is a subclass of int: arithmetic ===
# Every arithmetic operator reads True as 1 and False as 0, on either side.
assert True + True == 2
assert True + False == 1
assert False + False == 0
assert 1 + True == 2
assert True + 1 == 2
assert -3 + True == -2
assert True + 1.5 == 2.5
assert 1.5 + True == 2.5
assert False + 1.5 == 1.5

assert True - True == 0
assert False - True == -1
assert 2 - True == 1
assert True - 2 == -1
assert 1.5 - True == 0.5
assert True - 1.5 == -0.5

assert True % True == 0
assert False % True == 0
assert 2 % True == 0
assert True % 2 == 1
assert True % -3 == -2
assert -3 % True == 0
assert 1.5 % True == 0.5
assert True % 1.5 == 1.0
assert False % -2.0 == -0.0

assert True * 3 == 3
assert False * 3 == 0
assert True / 2 == 0.5
assert True // 2 == 0
assert True**3 == 1
assert 2**True == 2
assert 2**False == 1
assert False**0 == 1
assert True << True == 2
assert True >> True == 0

# The result is an int (or a float), never a bool.
assert type(True + True) == int
assert type(True - True) == int
assert type(True % True) == int
assert type(True * True) == int
assert type(True**True) == int
assert type(True + 1.5) == float
assert type(True / True) == float

# A big int on the other side promotes the same way.
big = 10**30
assert big + True == 10**30 + 1
assert True + big == 10**30 + 1
assert big - True == 10**30 - 1
assert True - big == 1 - 10**30
assert big % True == 0
assert True % big == 1
assert big * True == 10**30
assert big // True == 10**30

# === bool is a subclass of int: augmented assignment ===
n = True
n += True
assert n == 2
n = True
n -= 2
assert n == -1
n = 5
n += False
assert n == 5
n = 5
n %= True
assert n == 0
n = 1.5
n += True
assert n == 2.5

# === A false divisor is a zero divisor ===
try:
    True % False
    assert False, 'expected ZeroDivisionError'
except ZeroDivisionError as e:
    assert str(e) == 'division by zero'
try:
    5 // False
    assert False, 'expected ZeroDivisionError'
except ZeroDivisionError as e:
    assert str(e) == 'division by zero'
try:
    1.5 / False
    assert False, 'expected ZeroDivisionError'
except ZeroDivisionError as e:
    assert str(e) == 'division by zero'
try:
    False**-1
    assert False, 'expected ZeroDivisionError'
except ZeroDivisionError as e:
    assert str(e) == 'zero to a negative power'

# === Counting with sum() over a predicate ===
assert sum([True, True, False]) == 2
assert sum(x > 2 for x in [1, 2, 3, 4]) == 2
assert sum([1], True) == 2
assert sum([1.5], True) == 2.5
assert 'a\nb'.count('\n') + bool('c') == 2

# === bool is a subclass of int: builtins ===
assert abs(True) == 1
assert int(True) == 1
assert float(True) == 1.0
assert bin(True) == '0b1'
assert hex(True) == '0x1'
assert oct(False) == '0o0'
assert chr(True) == '\x01'
assert round(True) == 1
assert round(1.25, True) == 1.2
assert divmod(True, 2) == (0, 1)
assert divmod(2, True) == (2, 0)
assert min(True, 2) == 1
assert max(False, 2) == 2
assert pow(True, 2) == 1
assert pow(2, True) == 2
assert pow(2, 3, True) == 0
assert bytes(True) == b'\x00'
assert bytes(False) == b''
assert list(range(True)) == [0]
assert list(range(True, 3)) == [1, 2]
assert list(range(0, 5, True)) == [0, 1, 2, 3, 4]
assert len(range(True)) == 1

# === bool is a subclass of int: sequence and method arguments ===
assert 'abc'[True] == 'b'
assert 'abc'[True:] == 'bc'
assert 'abc'.zfill(True) == 'abc'
assert 'abc'.ljust(True) == 'abc'
assert 'abc'.rjust(True) == 'abc'
assert 'abc'.center(True) == 'abc'
assert 'abc'.expandtabs(True) == 'abc'
assert 'a,b,c'.split(',', True) == ['a', 'b,c']
assert 'a,b,c'.rsplit(',', False) == ['a,b,c']
assert 'aaa'.replace('a', 'b', True) == 'baa'
assert 'abc'.find('b', True) == 1

assert b'abc'[True] == 98
assert b'a,b'.split(b',', True) == [b'a', b'b']
assert b'abc'.find(b'b', True) == 1
assert b'abc'.count(b'b', False) == 1
assert b'aaa'.replace(b'a', b'b', True) == b'baa'
assert b'abc'.startswith(b'b', True) == True
assert b'abc'.ljust(True) == b'abc'
assert b'abc'.zfill(True) == b'abc'

assert [7, 8, 9][True] == 8
assert [7, 8, 9].index(8, True) == 1
assert [7, 8, 9].pop(True) == 8
assert (7, 8, 9)[True] == 8
assert (7, 8, 9).index(8, True) == 1
assert [7, 8] * True == [7, 8]
assert True * [7, 8] == [7, 8]
assert 'ab' * True == 'ab'
assert 'ab' * False == ''

items = [7, 8, 9]
items.insert(True, 1)
assert items == [7, 1, 8, 9]
items[True] = 2
assert items == [7, 2, 8, 9]
del items[False]
assert items == [2, 8, 9]

from collections import namedtuple

Point = namedtuple('Point', 'x y')
assert Point(7, 8)[True] == 8
assert Point(7, 8)[False] == 7

import math

assert math.factorial(True) == 1
assert math.gcd(True, 4) == 1
assert math.isqrt(True) == 1
assert math.comb(True, True) == 1

import itertools

assert list(itertools.repeat(1, True)) == [1]
assert list(itertools.islice([1, 2, 3], True)) == [1]

from collections import deque

assert deque([1, 2])[True] == 2
assert deque([1, 2], True) == deque([2], 1)
assert deque([1, 2]).index(2, True) == 1
