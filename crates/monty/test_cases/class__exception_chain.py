# === A sandbox exception class takes a cause and a context, as a builtin one does ===
class Refused(Exception): ...


class Overflow(Refused): ...


# === raise ... from an exception binds the object that was raised ===
try:
    try:
        raise KeyError('k')
    except KeyError as err:
        raise Refused('no', 2) from err
except Refused as e:
    assert type(e) is Refused
    assert e.args == ('no', 2)
    assert isinstance(e, Exception)
    assert isinstance(e.__cause__, KeyError)
    assert e.__suppress_context__ is True

# === raise ... from None suppresses the context and keeps the class ===
try:
    try:
        raise KeyError('k')
    except KeyError:
        raise Overflow('big') from None
except Refused as e:
    assert type(e) is Overflow
    assert e.__cause__ is None
    assert e.__suppress_context__ is True
    assert repr(e) == "Overflow('big')"
    assert str(e) == 'big'

# === A raise in a handler records the handled exception as the context ===
try:
    try:
        1 / 0
    except ZeroDivisionError:
        raise Refused('ctx')
except Refused as e:
    assert isinstance(e.__context__, ZeroDivisionError)
    assert e.__cause__ is None
    assert e.__suppress_context__ is False

# === An exception that no raise chained has no cause and no context ===
plain = Refused('plain')
assert plain.__cause__ is None
assert plain.__context__ is None
assert plain.__suppress_context__ is False

# === A sandbox exception is the cause and the context of a builtin one ===
try:
    try:
        raise Refused('inner')
    except Refused as inner:
        raise ValueError('outer') from inner
except ValueError as e:
    assert type(e.__cause__) is Refused
    assert e.__cause__.args == ('inner',)
    assert type(e.__context__) is Refused

# === A chain of sandbox exceptions ===
try:
    try:
        raise Refused('one')
    except Refused:
        raise Refused('two')
except Refused as e:
    assert e.args == ('two',)
    assert e.__context__.args == ('one',)
    assert e.__context__.__context__ is None

# === The same object raised again in its own handler takes no context ===
kept = Refused('same')
try:
    try:
        raise kept
    except Refused:
        raise kept
except Refused as e:
    assert e is kept
    assert e.__context__ is None

try:
    raise kept from None
except Refused as e:
    assert e is kept

# === A cause that is no exception is refused ===
try:
    raise Refused('typed') from 3
except TypeError as e:
    assert str(e) == 'exception causes must derive from BaseException'


# === In a generator and in a coroutine ===
def gen():
    try:
        yield 1
    except Refused as e:
        try:
            raise ValueError('in gen') from e
        except ValueError as v:
            yield type(v.__cause__).__name__, v.__cause__.args


g = gen()
assert next(g) == 1
assert g.throw(Refused('thrown')) == ('Refused', ('thrown',))
g.close()
