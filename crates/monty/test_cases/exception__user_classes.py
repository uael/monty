# A class that inherits from a builtin exception can be raised and caught. Its
# own name is what the traceback and `repr()` report; the builtin it descends
# from is what a broader `except` clause matches.


class Refused(Exception):
    pass


class Drift(Exception):
    pass


class Deeper(Refused):
    pass


# === Raising and catching by the class itself ===
try:
    raise Refused('why')
    assert False, 'expected Refused to be raised'
except Refused as exc:
    assert type(exc).__name__ == 'Refused'
    assert str(exc) == 'why'
    assert exc.args == ('why',)
    assert repr(exc) == "Refused('why')"

# === A subclass is caught by its base ===
try:
    raise Deeper('deep')
    assert False, 'expected Deeper to be raised'
except Refused as exc:
    assert type(exc).__name__ == 'Deeper'

# === And by the builtin it descends from ===
try:
    raise Refused('m')
    assert False, 'expected Refused to be raised'
except Exception as exc:
    assert str(exc) == 'm'

# === A sibling does not catch it ===
try:
    try:
        raise Refused('m')
    except Drift:
        assert False, 'expected Drift not to catch Refused'
except Refused:
    pass

# === A tuple of handlers ===
try:
    raise Deeper('t')
    assert False, 'expected Deeper to be raised'
except (ValueError, Refused) as exc:
    assert type(exc).__name__ == 'Deeper'

# === The first matching clause wins ===
order = []
try:
    raise Refused('o')
except Deeper:
    order.append('deeper')
except Refused:
    order.append('refused')
assert order == ['refused']

# === args follow BaseException ===
try:
    raise Refused()
    assert False, 'expected Refused to be raised'
except Refused as exc:
    assert exc.args == ()
    assert str(exc) == ''
    assert repr(exc) == 'Refused()'

try:
    raise Refused('a', 2)
    assert False, 'expected Refused to be raised'
except Refused as exc:
    assert exc.args == ('a', 2)
    assert str(exc) == "('a', 2)"
    assert repr(exc) == "Refused('a', 2)"

# === isinstance and issubclass ===
assert isinstance(Refused('a'), Refused)
assert isinstance(Refused('a'), Exception)
assert isinstance(Refused('a'), BaseException)
assert not isinstance(Refused('a'), ValueError)
assert isinstance(Deeper('a'), Refused)

assert issubclass(Refused, Exception)
assert issubclass(Deeper, Refused)
assert not issubclass(Refused, Drift)

# === The instance the handler binds is the one that was raised ===
marked = Refused('x')
marked.tag = 'kept'
try:
    raise marked
except Refused as exc:
    assert exc is marked
    assert exc.tag == 'kept'

# === A class may add its own behaviour ===


class Detailed(Exception):
    def __init__(self, path, line):
        self.path = path
        self.line = line
        self.args = (path, line)

    def where(self):
        return self.path + ':' + str(self.line)


try:
    raise Detailed('a.py', 3)
    assert False, 'expected Detailed to be raised'
except Exception as exc:
    assert exc.where() == 'a.py:3'
    assert exc.args == ('a.py', 3)
