import copy
from contextvars import ContextVar, Token

# === A variable with no default is unset until something sets it ===
plain = ContextVar('plain')
try:
    plain.get()
    assert False, 'expected a LookupError'
except LookupError as exc:
    # CPython's message is the variable's repr, which carries an address.
    assert str(exc) == repr(plain)

assert plain.get('fallback') == 'fallback'
assert plain.name == 'plain'

# === A default answers get() until a set() overrides it ===
holder = ContextVar('holder', default=7)
assert holder.get() == 7
# The call's own default wins over the variable's.
assert holder.get(9) == 9
token = holder.set(1)
assert holder.get() == 1
assert holder.get(9) == 1
assert token.var is holder

# === reset() puts back what the set() displaced ===
holder.reset(token)
assert holder.get() == 7

second = holder.set(2)
third = holder.set(3)
assert holder.get() == 3
holder.reset(third)
assert holder.get() == 2
holder.reset(second)
assert holder.get() == 7

# === A default of None is a default, not an absence ===
nothing = ContextVar('nothing', default=None)
assert nothing.get() is None
# Unset, so the call's own default still wins over it.
assert nothing.get('fallback') == 'fallback'
nothing.set(None)
# Set, so it does not.
assert nothing.get('fallback') is None

# === A token spends exactly once, and only on its own variable ===
spent = plain.set('a')
plain.reset(spent)
try:
    plain.reset(spent)
    assert False, 'expected a RuntimeError'
except RuntimeError as exc:
    assert str(exc) == f'{spent!r} has already been used once'

other = ContextVar('other')
borrowed = other.set('b')
try:
    plain.reset(borrowed)
    assert False, 'expected a ValueError'
except ValueError as exc:
    assert str(exc) == f'{borrowed!r} was created by a different ContextVar'

try:
    plain.reset(1)
    assert False, 'expected a TypeError'
except TypeError as exc:
    assert str(exc) == 'expected an instance of Token, got 1'

# === Types and identity ===
assert type(plain).__name__ == 'ContextVar'
assert type(borrowed).__name__ == 'Token'
assert isinstance(plain, ContextVar) == True
assert isinstance(borrowed, Token) == True
assert repr(ContextVar[str]) == '_contextvars.ContextVar[str]'
assert str(plain) == repr(plain)

# Two variables of the same name are two variables.
twin = ContextVar('plain')
assert (plain == twin) == False
assert len({plain, twin}) == 2
twin.set('elsewhere')
assert plain.get('unset') == 'unset'

# === A token is a context manager over its own set() ===
scoped = ContextVar('scoped', default='base')
with scoped.set('inner') as held:
    assert scoped.get() == 'inner'
    assert held.var is scoped

assert scoped.get() == 'base'

try:
    with scoped.set('raising'):
        raise ValueError('boom')
    assert False, 'expected a ValueError'
except ValueError as exc:
    assert str(exc) == 'boom'

assert scoped.get() == 'base'

# === A token is not constructible, hashable or copyable ===
try:
    hash(held)
    assert False, 'expected a TypeError'
except TypeError as exc:
    assert str(exc) == "unhashable type: '_contextvars.Token'"


try:
    Token()
    assert False, 'expected a RuntimeError'
except RuntimeError as exc:
    assert str(exc) == 'Tokens can only be created by ContextVars'

for uncopyable, name in ((plain, 'ContextVar'), (borrowed, 'Token')):
    for clone in (copy.copy, copy.deepcopy):
        try:
            clone(uncopyable)
            assert False, 'expected a TypeError'
        except TypeError as exc:
            assert str(exc) == f"cannot pickle '_contextvars.{name}' object"

# === Argument errors ===
try:
    ContextVar()
    assert False, 'expected a TypeError'
except TypeError as exc:
    assert str(exc) == 'ContextVar() takes exactly 1 positional argument (0 given)'

try:
    ContextVar('a', 1)
    assert False, 'expected a TypeError'
except TypeError as exc:
    assert str(exc) == 'ContextVar() takes at most 1 positional argument (2 given)'

try:
    ContextVar(name='a')
    assert False, 'expected a TypeError'
except TypeError as exc:
    assert str(exc) == 'ContextVar() takes exactly 1 positional argument (0 given)'

try:
    ContextVar('a', default=1, other=2)
    assert False, 'expected a TypeError'
except TypeError as exc:
    assert str(exc) == 'ContextVar() takes at most 2 arguments (3 given)'

try:
    ContextVar(1)
    assert False, 'expected a TypeError'
except TypeError as exc:
    assert str(exc) == 'context variable name must be a str'

try:
    plain.get(1, 2)
    assert False, 'expected a TypeError'
except TypeError as exc:
    assert str(exc) == 'get expected at most 1 argument, got 2'

try:
    plain.set()
    assert False, 'expected a TypeError'
except TypeError as exc:
    assert str(exc) == 'ContextVar.set() takes exactly one argument (0 given)'

try:
    plain.reset()
    assert False, 'expected a TypeError'
except TypeError as exc:
    assert str(exc) == 'ContextVar.reset() takes exactly one argument (0 given)'

try:
    plain.missing
    assert False, 'expected an AttributeError'
except AttributeError as exc:
    assert str(exc) == "'_contextvars.ContextVar' object has no attribute 'missing'"
