# `int`

## Shifts

A negative count raises `ValueError: negative shift count`, shifting zero gives
zero whatever the count, and a count past C `ssize_t` raises
`OverflowError: too many digits in integer` for `<<` while `>>` shifts
everything out and leaves the sign (`0` or `-1`), all as in CPython.

- **The `<<` overflow threshold is `2**63` rather than CPython's `2**63 - 37`.**
  CPython's is an artifact of its 30-bit digits: it converts the count first and
  then rejects the digit count the result would need. For a count in that
  37-wide band Monty attempts the shift, where CPython raises. Every count in
  the band asks for more than an exabyte, so what Monty raises instead is
  `MemoryError` against the session's `max_memory` (see ./resource_limits.md),
  or an allocator abort when no limit is configured.

## Mixing with `float`

An int of any size converts and the float operation is done; an int past the
float range raises `OverflowError: int too large to convert to float`. See
./float.md, which also lists the float-side divergences a mixed
operation inherits.
