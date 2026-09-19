# Ending a generator where it stands, and raising at the `yield` it stopped at.

log = []


def cleaned():
    try:
        yield 1
        yield 2
    finally:
        log.append('closed')


# === close() runs the finally blocks of a suspended frame ===
g = cleaned()
assert next(g) == 1
assert g.close() is None
assert log == ['closed']

# === and leaves it exhausted ===
try:
    next(g)
    raise AssertionError('expected StopIteration')
except StopIteration:
    pass

# === closing again is a no-op, as is closing one never started or finished ===
assert g.close() is None
assert cleaned().close() is None
assert log == ['closed']

done = cleaned()
assert list(done) == [1, 2]
assert done.close() is None
assert log == ['closed', 'closed']


# === a body that yields while closing is refused ===
def stubborn():
    while True:
        try:
            yield 1
        except GeneratorExit:
            yield 2


s = stubborn()
assert next(s) == 1
try:
    s.close()
    raise AssertionError('expected RuntimeError')
except RuntimeError as e:
    assert str(e) == 'generator ignored GeneratorExit'


# === `except Exception` does not swallow the exit ===
broad_log = []


def broad():
    try:
        yield 1
    except Exception:
        broad_log.append('wrongly caught')
        yield 9
    finally:
        broad_log.append('finally')


b = broad()
assert next(b) == 1
assert b.close() is None
assert broad_log == ['finally']


# === a finally that raises during close reports its own error ===
def bad_exit():
    try:
        yield 1
    finally:
        raise RuntimeError('in finally')


x = bad_exit()
assert next(x) == 1
try:
    x.close()
    raise AssertionError('expected RuntimeError')
except RuntimeError as e:
    assert str(e) == 'in finally'


# === throw() raises at the yield, and the body may catch it and go on ===
def catcher():
    while True:
        try:
            yield 'ok'
        except ValueError as e:
            yield 'caught ' + str(e)


c = catcher()
assert next(c) == 'ok'
assert c.throw(ValueError('boom')) == 'caught boom'
assert next(c) == 'ok'


# === an uncaught throw propagates to the caller and exhausts the generator ===
def bare():
    yield 1
    yield 2


t = bare()
assert next(t) == 1
try:
    t.throw(KeyError('k'))
    raise AssertionError('expected KeyError')
except KeyError as e:
    assert str(e) == "'k'"
try:
    next(t)
    raise AssertionError('expected StopIteration')
except StopIteration:
    pass


# === throw() into one never started raises at the caller ===
fresh = bare()
try:
    fresh.throw(ValueError('early'))
    raise AssertionError('expected ValueError')
except ValueError as e:
    assert str(e) == 'early'
try:
    next(fresh)
    raise AssertionError('expected StopIteration')
except StopIteration:
    pass


# === a throw the body turns into a clean end ===
def stopper():
    try:
        yield 1
    except ValueError:
        return


p = stopper()
assert next(p) == 1
try:
    p.throw(ValueError('done'))
    raise AssertionError('expected StopIteration')
except StopIteration:
    pass


# === finally runs when the generator is closed mid-loop ===
seen = []


def looped():
    for i in range(10):
        try:
            yield i
        finally:
            seen.append(i)


loop = looped()
assert next(loop) == 0
assert next(loop) == 1
assert loop.close() is None
assert seen == [0, 1]


# === throw() binds the object it was given, so a sandbox exception class catches ===
class Refused(Exception):
    pass


class Worse(Refused):
    pass


def catcher():
    try:
        yield 1
    except Refused as exc:
        yield 'caught ' + str(exc)


c = catcher()
assert next(c) == 1
assert c.throw(Refused('why')) == 'caught why'

# a subclass of a sandbox class is caught by the class it descends from
w = catcher()
assert next(w) == 1
assert w.throw(Worse('worse')) == 'caught worse'

# one that never started raises where it was asked, and is still that class there
unstarted = catcher()
try:
    unstarted.throw(Refused('early'))
    raise AssertionError('expected Refused')
except Refused as exc:
    assert str(exc) == 'early'

# and so does one that is over
over = catcher()
assert list(over) == [1]
try:
    over.throw(Refused('gone'))
    raise AssertionError('expected Refused')
except Refused as exc:
    assert str(exc) == 'gone'
