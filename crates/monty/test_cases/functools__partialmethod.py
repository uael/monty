# Tests for functools.partialmethod as a class member.
from functools import partialmethod


class Control:
    def __init__(self, tag: str) -> None:
        self.tag = tag

    def ctl(self, kind: str, extra: object = None) -> tuple[str, str, object]:
        return (self.tag, kind, extra)

    # The engine's shape: a bare verb bound to one fixed argument.
    abort = partialmethod(ctl, 'abort')
    pause = partialmethod(ctl, 'pause')
    # A stored keyword, which a call may override.
    resume = partialmethod(ctl, 'resume', extra=9)


obj = Control('c')

# === the stored argument is inserted after the receiver ===
assert obj.abort() == ('c', 'abort', None)
assert obj.pause() == ('c', 'pause', None)

# === the call's own arguments come after the stored ones ===
assert obj.abort(5) == ('c', 'abort', 5)
assert obj.abort(extra=5) == ('c', 'abort', 5)

# === stored keywords apply, and a call overrides them ===
assert obj.resume() == ('c', 'resume', 9)
assert obj.resume(extra=1) == ('c', 'resume', 1)

# A stored keyword still occupies its parameter, so passing it positionally as
# well is the ordinary duplicate-argument error. Only the tail is asserted:
# Monty names the method without the class qualifier (see limitations/classes.md).
try:
    obj.resume(1)
    raise AssertionError('expected TypeError')
except TypeError as e:
    assert str(e).endswith("ctl() got multiple values for argument 'extra'")

# === each instance binds its own receiver ===
other = Control('o')
assert other.abort() == ('o', 'abort', None)
assert obj.abort() == ('c', 'abort', None)

# === reached through the class, the receiver is the first argument ===
assert Control.abort(obj) == ('c', 'abort', None)
assert Control.abort(other, 7) == ('o', 'abort', 7)

# === the descriptor itself, outside a class body ===
plain = partialmethod(len, 1)
assert plain.args == (1,)
assert plain.keywords == {}
assert plain.func is len
assert repr(plain) == 'functools.partialmethod(<built-in function len>, 1)'

keyed = partialmethod(len, 1, k=2)
assert keyed.args == (1,)
assert keyed.keywords == {'k': 2}

# === construction errors ===
try:
    partialmethod()  # pyright: ignore[reportCallIssue]
    raise AssertionError('expected TypeError')
except TypeError as e:
    assert str(e) == "_partial_new() missing 1 required positional argument: 'func'"

# The first argument is checked for callability at construction, not at use.
try:
    partialmethod(1)
    raise AssertionError('expected TypeError')
except TypeError as e:
    assert str(e) == 'the first argument 1 must be a callable or a descriptor'

try:
    partialmethod('nope')
    raise AssertionError('expected TypeError')
except TypeError as e:
    assert str(e) == "the first argument 'nope' must be a callable or a descriptor"

# === errors from the wrapped function surface unchanged ===
try:
    obj.abort(1, 2)  # pyright: ignore[reportCallIssue]
    raise AssertionError('expected TypeError')
except TypeError:
    pass
