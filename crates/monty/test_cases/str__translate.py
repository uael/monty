# Tests for str.translate() and str.maketrans()

# === str.translate() ===
assert 'abc'.translate({97: 'X', 98: None, 99: 100}) == 'Xd', 'str, None and int values'
assert 'abc'.translate({}) == 'abc', 'an empty table keeps each character'
assert 'ab'.translate({97: True}) == '\x01b', 'a bool counts as an int'
assert 'aé'.translate({233: 'e'}) == 'ae', 'non-ASCII keys'
assert 'abc'.translate({97: 'xyz'}) == 'xyzbc', 'a str of many characters'
assert 'abc'.translate(['x', 'y', 'z'] * 40) == 'yzx', 'a list serves as a table'


class Upper:
    def __getitem__(self, key):
        if key == 98:
            raise KeyError('b')
        return chr(key).upper()


assert 'abc'.translate(Upper()) == 'AbC', 'an object with __getitem__ serves as a table'

try:
    'abc'.translate({97: 1.5})
    assert False, 'a float value should raise'
except TypeError as e:
    assert str(e) == 'character mapping must return integer, None or str', str(e)
try:
    'abc'.translate({97: 0x110000})
    assert False, 'a value out of range should raise'
except ValueError as e:
    assert str(e) == 'character mapping must be in range(0x110000)', str(e)
try:
    'ab'.translate({97: 2**70})
    assert False, 'a large int should raise'
except ValueError as e:
    assert str(e) == 'character mapping must be in range(0x110000)', str(e)
try:
    'abc'.translate(None)
    assert False, 'None as a table should raise'
except TypeError as e:
    assert str(e) == "'NoneType' object is not subscriptable", str(e)


class Broken:
    def __getitem__(self, key):
        raise ValueError('broken')


try:
    'a'.translate(Broken())
    assert False, 'an error of the table should propagate'
except ValueError as e:
    assert str(e) == 'broken', str(e)

# === str.maketrans() ===
assert str.maketrans('ab', 'xy') == {97: 120, 98: 121}, 'two strings'
assert str.maketrans('ab', 'xy', 'c') == {97: 120, 98: 121, 99: None}, 'three strings'
assert str.maketrans({'a': 'b', 98: None}) == {97: 'b', 98: None}, 'a dict'
assert ''.maketrans('a', 'b') == {97: 98}, 'from an instance'
assert 'abc'.translate(str.maketrans('ab', 'xy', 'c')) == 'xy', 'maketrans then translate'
assert 'hello world'.translate(str.maketrans('', '', 'lo')) == 'he wrd', 'delete characters'

for args, kind, message in [
    (('ab', 'x'), ValueError, 'the first two maketrans arguments must have equal length'),
    (({'ab': 1},), ValueError, 'string keys in translatetable must be of length 1'),
    (({1.5: 1},), TypeError, 'keys in translate table mustbe strings or integers'),
    (({'a': 'b', 'cd': 1, 'e': 'f'},), ValueError, 'string keys in translatetable must be of length 1'),
    (({'a': [1], 2.5: 1, 'e': 'f'},), TypeError, 'keys in translate table mustbe strings or integers'),
    (('a',), TypeError, 'if you give only one argument to maketrans it must be a dict'),
    ((1, 'a'), TypeError, 'first maketrans argument must be a string if there is a second argument'),
    (('a', 1), TypeError, 'maketrans() argument 2 must be str, not int'),
    (('a', 'b', 1), TypeError, 'maketrans() argument 3 must be str, not int'),
    ((), TypeError, 'maketrans expected at least 1 argument, got 0'),
    (('a', 'b', 'c', 'd'), TypeError, 'maketrans expected at most 3 arguments, got 4'),
]:
    try:
        str.maketrans(*args)
        assert False, f'maketrans{args} should raise'
    except kind as e:
        assert str(e) == message, str(e)
