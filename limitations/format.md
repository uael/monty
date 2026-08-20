# Format mini-language (f-string specs)

Monty implements CPython 3.14's format mini-language. It is reachable through
f-strings and through `str.format`, which share one formatter: a template's
replacement fields are parsed when it is applied, and each field's value and
spec go to the same code an f-string's do.

`str.format` supports the whole of PEP 3101's field grammar — automatic and
manual numbering (which cannot be mixed), keyword fields, `.attribute` and
`[key]` access, the `!s`/`!r`/`!a` conversions, and specs that nest fields of
their own one level deep. A field reads an attribute the way `getattr` does,
so a property in a field is computed as CPython computes it.

A t-string never *applies* a format spec: PEP 750 records the spec's rendered
text on the `Interpolation` and leaves formatting to the consumer, so
`t'{x:%Q}'` builds a `Template` carrying `format_spec == '%Q'` where the
f-string `f'{x:%Q}'` raises. A nested field inside a t-string spec
(`t'{x:>{w}}'`) *is* evaluated, with `str()` and no spec of its own, exactly as
CPython renders it. See ./string_templatelib.md.

The other CPython formatting mechanisms are not implemented:

- The `format()` builtin raises `NameError` (see ./builtins.md), and
  `str.format_map` raises `AttributeError`: it takes one mapping where
  `str.format` takes keyword arguments.
- Printf-style `%` formatting (`'%5.3f' % math.pi`, `'%s %s' % (a, b)`) is not
  implemented. `str` has no `__mod__`, so `str % value` raises
  `TypeError: unsupported operand type(s) for %: 'str' and '...'`. Use an
  f-string instead.

## Custom `__format__`

f-strings dispatch to a type's `__format__` only for `date`/`datetime`, which
interpret the spec as a `strftime` string (`f'{dt:%Y-%m-%d}'`); see
./datetime.md. There is no general `__format__` protocol: user
classes can't customise formatting (see ./classes.md), and all
other types use the builtin mini-language formatter. A format spec on a
user-class instance is silently applied to `str(obj)` (`f'{obj:>10}'` pads),
where CPython raises `TypeError: unsupported format string passed to
Foo.__format__`.

## The `n` type uses the C locale only

`n` always behaves as in the C/POSIX locale (Monty has no locale support):
like `d` for integers and `g` for floats, with no digit grouping. CPython
under a grouping locale would insert locale-specific separators; Monty never
does.

## `repr` of non-printable Unicode

`repr` escapes non-printable code points via the `unicode-general-category`
crate, whose Unicode version may lag CPython's, so a code point assigned in a
newer Unicode release than the crate ships could be escaped by Monty while
CPython prints it literally, or the reverse. Common text is unaffected.

## Width / precision bounds

- A `width` or `precision` whose decimal value overflows `usize` raises
  `SyntaxError: Invalid format specifier '...': width or precision overflows
  usize` rather than being accepted. CPython is bounded only by memory.
- Very large widths/precisions are additionally bounded by the resource
  tracker; see ./resource_limits.md.

## When spec errors are raised

CPython validates a *static* (literal) spec only when the f-string executes, so
a malformed spec in dead code never raises. Monty validates literal specs at
**compile time** for the structurally-malformed cases: two or more trailing
characters after the type field (`f'{1:kk}'`, `f'{1:10xyz}'`) and `usize`
overflow, raising `SyntaxError` instead of CPython's runtime `ValueError`. The
message text otherwise matches, minus CPython's `for object of type '...'`
suffix, which needs the runtime value type. Specs whose error *is*
value-type-dependent or only resolvable at format time (`Unknown format code
'k'`, the `Cannot specify …` grouping conflicts, and `Format specifier missing
precision`) are deferred to runtime and raise the exact CPython `ValueError`,
as do all dynamically-built specs (`f'{1:{spec}}'`).
