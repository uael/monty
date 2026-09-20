from asyncio import CancelledError

# === `except Exception:` does not swallow it ===
seen = []
try:
    try:
        raise CancelledError('stopped')
    except Exception:
        seen.append('Exception')
except CancelledError as exc:
    seen.append(str(exc))

assert seen == ['stopped']

# === It is a BaseException, not an Exception ===
assert issubclass(CancelledError, BaseException) == True
assert issubclass(CancelledError, Exception) == False
err = CancelledError()
assert isinstance(err, BaseException) == True
assert isinstance(err, Exception) == False
assert type(err).__name__ == 'CancelledError'
assert err.args == ()
assert str(CancelledError('a')) == 'a'
assert CancelledError('a').args == ('a',)

# === It has to be imported: the bare name is no builtin ===
try:
    exec('CancelledError', {})
    assert False, 'expected a NameError'
except NameError as exc:
    assert str(exc) == "name 'CancelledError' is not defined"

# === A `finally` still runs when it unwinds ===
ran = []
try:
    try:
        raise CancelledError()
    finally:
        ran.append('finally')
except CancelledError:
    ran.append('caught')

assert ran == ['finally', 'caught']
