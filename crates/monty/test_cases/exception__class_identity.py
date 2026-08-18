# The class of an exception instance is the same object its name evaluates to.
try:
    raise ValueError('x')
except ValueError as exc:
    caught = exc

assert type(caught) is ValueError
assert type(caught) == ValueError
assert repr(type(caught)) == "<class 'ValueError'>"
assert type(caught).__name__ == 'ValueError'

# It also survives being passed around, which is how `__exit__` receives it.
seen = []


class Ctx:
    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc, tb):
        seen.append(exc_type)
        return True


with Ctx():
    raise KeyError('k')

assert seen == [KeyError]
assert seen[0] is KeyError

# Distinct exception classes stay distinct.
assert type(caught) is not KeyError
assert ValueError is not KeyError

# A class with no instance to hand still compares to itself.
assert ValueError is ValueError
assert type(TypeError('t')) is TypeError
