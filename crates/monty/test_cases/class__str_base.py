# === An instance of a class that inherits str is a string ===


class Act(str):
    kind = 'act'

    def who(self):
        return 'who:' + self


a = Act('hello')
assert a == 'hello'
assert 'hello' == a
assert hash(a) == hash('hello')
assert len(a) == 5
assert bool(a)
assert not Act('')
assert repr(a) == "'hello'"
assert str(a) == 'hello'
assert f'{a}' == 'hello'
assert f'{a!r}' == "'hello'"
assert f'{a:>7}' == '  hello'
assert '%s' % a == 'hello'

# === It answers every string method, and they give a plain str back ===
assert a.startswith('he')
assert a.endswith('lo')
assert a.partition('l') == ('he', 'l', 'lo')
assert a.rpartition('l') == ('hel', 'l', 'o')
assert a.removesuffix('lo') == 'hel'
assert a.removeprefix('he') == 'llo'
assert a.upper() == 'HELLO'
assert a.replace('l', 'L') == 'heLLo'
assert a.split('l') == ['he', '', 'o']
assert a.splitlines() == ['hello']
sep = '-'
assert sep.join([a, 'x']) == 'hello-x'
assert a.join(['1', '2']) == '1hello2'
assert a[1:3] == 'el'
assert a[0] == 'h'
assert list(a) == ['h', 'e', 'l', 'l', 'o']
assert 'ell' in a
assert a in 'xhelloy'
assert a + '!' == 'hello!'
assert a * 2 == 'hellohello'
assert a.encode() == b'hello'
assert type(a.upper()) is str
assert type(a[1:]) is str
assert type(a + 'x') is str
assert type(str(a)) is str

# === The class adds methods, class variables and type identity ===
assert a.who() == 'who:hello'
assert a.kind == 'act'
assert Act.kind == 'act'
assert Act.__name__ == 'Act'
assert type(a) is Act
assert a.__class__ is Act
assert isinstance(a, Act)
assert isinstance(a, str)
assert issubclass(Act, str)
assert not issubclass(str, Act)
member = 'who'
assert getattr(a, member)() == 'who:hello'
assert hasattr(a, 'who')
assert not hasattr(a, 'nope')

# === A method of the class wins over the string method of the same name ===


class Shouted(str):
    def upper(self):
        return 'SHOUTED'


assert Shouted('q').upper() == 'SHOUTED'
assert Shouted('q').lower() == 'q'

# === The constructor takes what str() takes ===
assert Act() == ''
assert Act(5) == '5'
assert Act([1]) == '[1]'
assert Act(b'ab', 'utf-8') == 'ab'

# === The chain goes on below a class that inherits str ===


class Sub(Act):
    def who(self):
        return 'sub'


s = Sub('hi')
assert s == 'hi'
assert s.who() == 'sub'
assert s.kind == 'act'
assert type(s) is Sub
assert isinstance(s, Sub)
assert isinstance(s, Act)
assert isinstance(s, str)
assert issubclass(Sub, Act)
assert issubclass(Sub, str)
assert not isinstance(a, Sub)

# === It compares, sorts and keys a dict or a set as the string it is ===
assert Act('a') < Act('b')
assert sorted([Act('b'), 'a', Act('c')]) == ['a', 'b', 'c']
assert {Act('x'): 1}['x'] == 1
assert {'x': 2}[Act('x')] == 2
assert len({Act('x'), 'x'}) == 1
assert Act('x') in {'x'}

# === It matches as a string ===
match a:
    case str() as matched:
        assert matched == 'hello'
    case _:
        raise AssertionError

match a:
    case 'hello':
        pass
    case _:
        raise AssertionError

# === The builtin is named in a message about an operation on the string ===
try:
    a + 1
    raise AssertionError
except TypeError as exc:
    assert str(exc) == 'can only concatenate str (not "int") to str'

# === The class is named in a message about the object ===
try:
    a.nope
    raise AssertionError
except AttributeError as exc:
    assert str(exc) == "'Act' object has no attribute 'nope'"
