# No builtin type implements `@`, so every operand pair reports the same
# unsupported-operand TypeError CPython raises. The exception type matters as
# much as the text: code catching TypeError has to catch this.

try:
    1 @ 2
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "unsupported operand type(s) for @: 'int' and 'int'"

try:
    'a' @ 'b'
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "unsupported operand type(s) for @: 'str' and 'str'"

try:
    [1] @ 2
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "unsupported operand type(s) for @: 'list' and 'int'"

try:
    None @ None
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "unsupported operand type(s) for @: 'NoneType' and 'NoneType'"

try:
    1.5 @ (2, 3)
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "unsupported operand type(s) for @: 'float' and 'tuple'"

# A big int is a heap value rather than an immediate, so it reaches `@` by a
# different route than the cases above.
try:
    (10**30) @ 1
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == "unsupported operand type(s) for @: 'int' and 'int'"
