from string.templatelib import Interpolation, Template

x = 42
name = 'world'

# === Structure ===
plain = t'hello'
assert type(plain) is Template
assert plain.strings == ('hello',)
assert plain.interpolations == ()

simple = t'hello {name}'
assert simple.strings == ('hello ', '')
assert len(simple.interpolations) == 1

# `strings` always has one more entry than `interpolations`, empty ones included.
assert t''.strings == ('',)
assert t'{x}'.strings == ('', '')
assert t'{x}{x}'.strings == ('', '', '')
assert t'a{x}b{x}c'.strings == ('a', 'b', 'c')

# === Interpolation fields ===
one = t'{x}'.interpolations[0]
assert type(one) is Interpolation
assert one.value == 42
assert one.expression == 'x'
assert one.conversion is None
assert one.format_spec == ''

converted = t'{x!r}'.interpolations[0]
assert converted.conversion == 'r'
assert t'{x!s}'.interpolations[0].conversion == 's'
assert t'{x!a}'.interpolations[0].conversion == 'a'

spec = t'{x:>5}'.interpolations[0]
assert spec.format_spec == '>5'
assert spec.conversion is None

# A nested field in the spec is rendered into text at build time.
width = 5
nested = t'{x:>{width}}'.interpolations[0]
assert nested.format_spec == '>5'

# The expression text is the source between `{` and the terminator. The
# whitespace rule (leading kept, trailing stripped) cannot be pinned here
# because `ruff format` normalises interior spaces away; see
# `tests/pep695_pep750.rs`.
assert t'{x + 1}'.interpolations[0].expression == 'x + 1'
assert t'{x + 1}'.interpolations[0].value == 43
assert t'{x!r}'.interpolations[0].expression == 'x'

# The `=` debug form puts its text in the literal segment and defaults to repr.
debug = t'{x=}'
assert debug.strings == ('x=', '')
assert debug.interpolations[0].expression == 'x'
assert debug.interpolations[0].conversion == 'r'
assert t'{x=!s}'.interpolations[0].conversion == 's'
assert t'{x=:>5}'.interpolations[0].conversion is None
assert t'{x=:>5}'.interpolations[0].format_spec == '>5'

# === Iteration interleaves, skipping empty segments ===
assert list(t'') == []
assert list(t'abc') == ['abc']
single = t'{x}'
assert list(single) == [single.interpolations[0]]

parts = list(t'a{x}b')
assert len(parts) == 3
assert parts[0] == 'a'
assert type(parts[1]) is Interpolation
assert parts[2] == 'b'

kinds = [type(p) is Interpolation for p in t'{x}mid{name}']
assert kinds == [True, False, True]

# === repr ===
assert repr(t'a{x}b') == "Template(strings=('a', 'b'), interpolations=(Interpolation(42, 'x', None, ''),))"
assert repr(t'{x!r:>5}'.interpolations[0]) == "Interpolation(42, 'x', 'r', '>5')"

# === values ===
assert t'{x}{name}'.values == (42, 'world')
assert t'plain'.values == ()

# === Implicit concatenation of adjacent t-strings ===
joined = t'a{x}b{name}'
assert joined.strings == ('a', 'b', '')
assert len(joined.interpolations) == 2

# === The imported names are the same types the values report ===
assert type(t'') is Template
assert type(t'{x}'.interpolations[0]) is Interpolation

# === Expressions are evaluated once, at construction ===
calls = []


def bump():
    calls.append('x')
    return len(calls)


built = t'{bump()}'
assert calls == ['x']
assert built.interpolations[0].value == 1
assert built.interpolations[0].value == 1
assert calls == ['x']


# === Templates work inside functions and closures ===
def render(value):
    return t'v={value}'


rendered = render('inner')
assert rendered.strings == ('v=', '')
assert rendered.interpolations[0].value == 'inner'
assert rendered.interpolations[0].expression == 'value'
