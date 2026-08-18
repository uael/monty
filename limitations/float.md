# `float`

## Mixing with `int`

An int mixed with a float is converted and the float operation done, for every
operator and either operand order, whatever the int's size. An int past the
float range raises `OverflowError: int too large to convert to float`, and it
raises before the divisor is examined, so `10**400 / 0.0` reports the conversion
rather than the division. All as in CPython.

## Divergences

- **`**` does not raise on overflow.** `1.5 ** 1e19` and `1e300 ** 2.0` answer
  `inf` where CPython raises `OverflowError: (34, 'Result too large')`. Reached
  through an int operand too (`1.5 ** (2**64)`), since that converts to a float
  first.
- **`//` divides and floors** rather than taking CPython's `divmod` quotient, so
  the two disagree wherever the division itself loses the answer: `float('inf')
  // 7.0` is `inf` here and `nan` in CPython, and `1e-300 // -1e30` is `-0.0`
  here and `-1.0` in CPython. `divmod`'s quotient is the same value, so it
  diverges with it; its remainder is `%`'s and does not.
- **A negative base raised to a non-integral power gives `nan`** where CPython
  returns a complex number (`(-7) ** 1.5`). Monty has no complex type.
- **`0 ** float('-inf')` raises `ZeroDivisionError`** where CPython answers
  `inf`. CPython treats an infinite exponent as a limit rather than as a
  negative power.
