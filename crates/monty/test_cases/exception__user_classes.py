# === Defining and raising an exception class ===
class Fault(Exception):
    pass


class Halt(Fault):
    pass


try:
    raise Halt('stopped')
    assert False, 'expected Halt'
except Halt as exc:
    assert type(exc) is Halt
    assert str(exc) == 'stopped'
    assert exc.args == ('stopped',)

# A base catches its subclasses, and `Exception` catches them all.
try:
    raise Halt('a')
    assert False, 'expected Halt'
except Fault as exc:
    assert str(exc) == 'a'

try:
    raise Halt('b')
    assert False, 'expected Halt'
except Exception as exc:
    assert str(exc) == 'b'


# A sibling class does not catch it.
class Other(Exception):
    pass


try:
    try:
        raise Halt('c')
    except Other:
        assert False, 'Other must not catch Halt'
except Halt as exc:
    assert str(exc) == 'c'

# A user class does not catch a builtin exception.
try:
    try:
        raise ValueError('builtin')
    except Halt:
        assert False, 'Halt must not catch ValueError'
except ValueError as exc:
    assert str(exc) == 'builtin'

# A tuple of handlers works, mixing builtin and user classes.
try:
    raise Halt('tuple')
    assert False, 'expected Halt'
except (KeyError, Fault) as exc:
    assert str(exc) == 'tuple'

# === isinstance / issubclass on exceptions ===
err = Halt('x')
assert isinstance(err, Halt)
assert isinstance(err, Fault)
assert isinstance(err, Exception)
assert not isinstance(err, ValueError)
assert issubclass(Halt, Fault)
assert issubclass(Halt, Exception)

# === Raising a bare class instantiates it ===
try:
    raise Halt
    assert False, 'expected Halt'
except Halt as exc:
    assert exc.args == ()
    assert str(exc) == ''

# === Zero, one and many constructor arguments ===
assert Halt().args == ()
assert Halt('one').args == ('one',)
assert Halt('one', 2).args == ('one', 2)
assert str(Halt('one', 2)) == "('one', 2)"
assert repr(Halt('one')) == "Halt('one')"
assert repr(Halt()) == 'Halt()'

# Builtin exceptions take several arguments too.
assert ValueError('m', 2).args == ('m', 2)
assert str(ValueError('m', 2)) == "('m', 2)"
assert repr(ValueError('m', 2)) == "ValueError('m', 2)"
assert ValueError().args == ()


# === A custom __init__ with super() ===
class Raised(Exception):
    def __init__(self, message, seed):
        super().__init__(message)
        self.seed = seed


try:
    raise Raised('boom', 7)
    assert False, 'expected Raised'
except Raised as exc:
    assert exc.seed == 7
    assert exc.args == ('boom',)
    assert str(exc) == 'boom'


# Without the super() call the constructor arguments are still recorded.
class Recorded(Exception):
    def __init__(self, a, b):
        self.pair = (a, b)


rec = Recorded(1, 2)
assert rec.pair == (1, 2)
assert rec.args == (1, 2)


# === Instance attributes and methods on exceptions ===
class Detailed(Exception):
    def __init__(self, code):
        super().__init__('code ' + str(code))
        self.code = code

    def doubled(self):
        return self.code * 2


detailed = Detailed(21)
assert detailed.code == 21
assert detailed.doubled() == 42
assert str(detailed) == 'code 21'


# === A custom __str__ is used by str() ===
class Custom(Exception):
    def __str__(self):
        return 'always this'


assert str(Custom('ignored')) == 'always this'
assert Custom('ignored').args == ('ignored',)

# === raise ... from ... records __cause__ ===
try:
    try:
        raise ValueError('inner')
    except ValueError as inner:
        raise Halt('outer') from inner
    assert False, 'expected Halt'
except Halt as exc:
    assert isinstance(exc.__cause__, ValueError)
    assert str(exc.__cause__) == 'inner'
    # An exception raised while handling another also records __context__.
    assert isinstance(exc.__context__, ValueError)

# Unset chaining slots read as None.
assert Halt('plain').__cause__ is None
assert Halt('plain').__context__ is None
assert ValueError('plain').__cause__ is None

# A builtin exception can be given a cause too.
try:
    try:
        raise KeyError('k')
    except KeyError as inner:
        raise RuntimeError('wrapped') from inner
    assert False, 'expected RuntimeError'
except RuntimeError as exc:
    assert isinstance(exc.__cause__, KeyError)

# === Implicit context without an explicit cause ===
try:
    try:
        raise ValueError('first')
    except ValueError:
        raise Halt('second')
    assert False, 'expected Halt'
except Halt as exc:
    assert exc.__cause__ is None
    assert isinstance(exc.__context__, ValueError)


# === A non-exception class cannot be raised ===
class NotAnError:
    pass


try:
    raise NotAnError()
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == 'exceptions must derive from BaseException'
