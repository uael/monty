# `dataclasses.field()`: per-field defaults, factories, and the flags that keep
# a field out of the constructor, the repr, or the comparison.
from dataclasses import MISSING, dataclass, field, fields


@dataclass
class Rec:
    a: int
    label: str = field(default='')
    tags: tuple[str, ...] = field(default=(), compare=False)
    xs: list[int] = field(default_factory=list)


r = Rec(1)
assert r.a == 1
assert r.label == ''
assert r.tags == ()
assert r.xs == []

# === default_factory builds a fresh value per instance ===
assert Rec(1).xs is not Rec(1).xs
Rec(1).xs.append(9)
assert Rec(1).xs == []

# === a plain default stays on the class; a factory leaves nothing behind ===
assert Rec.tags == ()
assert not hasattr(Rec, 'xs')

# === compare=False keeps a field out of __eq__ (and out of the hash) ===
assert Rec(1, tags=('x',)) == Rec(1, tags=())
assert Rec(1, label='a') != Rec(1, label='b')
assert repr(Rec(1, tags=('x',))) == "Rec(a=1, label='', tags=('x',), xs=[])"


@dataclass(frozen=True)
class Frozen:
    a: int
    tags: tuple[str, ...] = field(default=(), compare=False)


assert hash(Frozen(1, ('x',))) == hash(Frozen(1, ()))


# === repr=False hides a field from the repr but keeps it a field ===
@dataclass
class Hidden:
    shown: int
    secret: int = field(default=0, repr=False)


assert repr(Hidden(1, 2)) == 'Hidden(shown=1)'
assert Hidden(1, 2).secret == 2
assert Hidden(1, 2) != Hidden(1, 3)


# === init=False keeps a field out of the constructor ===
@dataclass
class Derived:
    a: int
    doubled: int = field(init=False, default=0)


d = Derived(3)
assert d.doubled == 0
try:
    Derived(3, 6)
    raise AssertionError('an init=False field is not a parameter')
except TypeError:
    pass
try:
    Derived(3, doubled=6)
    raise AssertionError('an init=False field is not a keyword either')
except TypeError as exc:
    assert str(exc) == "Derived.__init__() got an unexpected keyword argument 'doubled'", str(exc)


# === an init=False field with no default is simply never set ===
@dataclass
class Unset:
    a: int
    later: int = field(init=False)


u = Unset(1)
assert u.a == 1
try:
    u.later
    raise AssertionError('an unset field has no value')
except AttributeError:
    pass


# === per-field kw_only ===
@dataclass
class Mixed:
    a: int
    b: int = field(kw_only=True)
    c: int = 3


assert Mixed(1, b=2).c == 3
assert Mixed(1, 4, b=2).c == 4
assert Mixed.__match_args__ == ('a', 'c')
try:
    Mixed(1, 2)
    raise AssertionError('b is keyword-only')
except TypeError as exc:
    assert str(exc) == "Mixed.__init__() missing 1 required keyword-only argument: 'b'", str(exc)


# === the Field objects themselves ===
names = [f.name for f in fields(Rec)]
assert names == ['a', 'label', 'tags', 'xs']
by_name = {f.name: f for f in fields(Rec)}
assert by_name['a'].default is MISSING
assert by_name['a'].default_factory is MISSING
assert by_name['label'].default == ''
assert by_name['tags'].compare is False
assert by_name['a'].compare is True
assert by_name['a'].init is True
assert by_name['a'].repr is True
assert by_name['a'].kw_only is False
assert by_name['a'].hash is None
assert by_name['a'].doc is None
assert by_name['xs'].default_factory is list
assert by_name['a'].metadata == {}

# `fields()` returns a tuple of the very objects the class holds
assert isinstance(fields(Rec), tuple)
assert fields(Rec)[0] is fields(Rec)[0]


# === metadata and doc ===
@dataclass
class Annotated:
    x: int = field(default=0, metadata={'units': 'm'}, doc='the x')


meta = fields(Annotated)[0]
assert meta.metadata['units'] == 'm'
assert meta.doc == 'the x'


# === MISSING is a singleton, and never a value a field takes ===
assert MISSING is MISSING


# === field() rejects what CPython rejects ===
try:
    field(default=1, default_factory=list)
    raise AssertionError('default and default_factory are exclusive')
except ValueError as exc:
    assert str(exc) == 'cannot specify both default and default_factory', str(exc)

try:
    field(1)
    raise AssertionError('field() takes no positional arguments')
except TypeError as exc:
    assert str(exc) == 'field() takes 0 positional arguments but 1 was given', str(exc)

try:
    field(metadata=5)
    raise AssertionError('metadata must be a mapping')
except TypeError as exc:
    assert str(exc) == 'mappingproxy() argument must be a mapping, not int', str(exc)


# === a mutable default is still rejected, whether written plainly or via field() ===
try:

    @dataclass
    class BadPlain:
        xs: list[int] = []

    raise AssertionError('a mutable default is rejected')
except ValueError as exc:
    assert str(exc) == "mutable default <class 'list'> for field xs is not allowed: use default_factory", str(exc)

try:

    @dataclass
    class BadField:
        xs: list[int] = field(default=[])

    raise AssertionError('a mutable default is rejected through field() too')
except ValueError as exc:
    assert str(exc) == "mutable default <class 'list'> for field xs is not allowed: use default_factory", str(exc)


# === and a non-default field after a defaulted one ===
try:

    @dataclass
    class BadOrder:
        a: int = 1
        b: int

    raise AssertionError('a non-default field cannot follow a defaulted one')
except TypeError as exc:
    assert str(exc) == "non-default argument 'b' follows default argument 'a'", str(exc)
