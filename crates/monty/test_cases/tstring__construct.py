from string.templatelib import Interpolation, Template

# === Interpolation is built from its value and its three fields ===
bare = Interpolation(1)
assert (bare.value, bare.expression, bare.conversion, bare.format_spec) == (1, '', None, '')
full = Interpolation('v', 'name', 'r', '>5')
assert (full.value, full.expression, full.conversion, full.format_spec) == ('v', 'name', 'r', '>5')
named = Interpolation(value=2, expression='e', conversion=None, format_spec='')
assert repr(named) == "Interpolation(2, 'e', None, '')"

# === Template joins adjacent strings and pads between interpolations ===
built = Template('a', 'b', Interpolation(1, 'x'), Interpolation(2, 'y'), 'c')
assert built.strings == ('ab', '', 'c')
assert [i.value for i in built.interpolations] == [1, 2]
assert built.values == (1, 2)
assert Template().strings == ('',)
assert Template().interpolations == ()
parts = list(Template('p', Interpolation(3, 'z')))
assert len(parts) == 2 and parts[0] == 'p' and isinstance(parts[1], Interpolation) and parts[1].value == 3
assert isinstance(built, Template)


# === Wrong arguments raise as CPython does ===
def raised(f):
    try:
        f()
    except Exception as e:
        return f'{type(e).__name__}: {e}'
    return 'no error'


assert raised(lambda: Template(1)) == (
    "TypeError: Template.__new__ *args need to be of type 'str' or 'Interpolation', got int"
)
assert raised(lambda: Template(a=1)) == 'TypeError: Template.__new__ only accepts *args arguments'
assert raised(lambda: Interpolation(1, 2)) == "TypeError: Interpolation() argument 'expression' must be str, not int"
assert raised(lambda: Interpolation(1, 'x', 'q')) == (
    "ValueError: Interpolation() argument 'conversion' must be one of 's', 'a' or 'r'"
)
assert (
    raised(lambda: Interpolation(1, 'x', 5)) == "TypeError: Interpolation() argument 'conversion' must be str, not int"
)
assert raised(lambda: Interpolation(1, 'x', None, 3)) == (
    "TypeError: Interpolation() argument 'format_spec' must be str, not int"
)
assert raised(lambda: Interpolation()) == "TypeError: Interpolation() missing required argument 'value' (pos 1)"
