calls = []


def tag(fn):
    calls.append('tag')

    def wrapper(*args, **kwargs):
        calls.append('wrapped')
        return fn(*args, **kwargs)

    return wrapper


def double(fn):
    def wrapper(self, x):
        return fn(self, x) * 2

    return wrapper


def constant(value):
    def deco(fn):
        return lambda self: value

    return deco


class Sample:
    # A decorator resolved from the enclosing (module) scope.
    @tag
    def greet(self, name):
        return 'hi ' + name

    # Decorators stack bottom-up, so `double` runs first and `tag` wraps it.
    @tag
    @double
    def scaled(self, x):
        return x + 1

    # A decorator factory: the call runs at class-definition time.
    @constant(42)
    def fixed(self):
        return 0

    # A decorator defined earlier in the class body resolves there.
    def local_deco(fn):
        def wrapper(self):
            return 'local:' + fn(self)

        return wrapper

    @local_deco
    def uses_local(self):
        return 'inner'


# Both decorated methods ran their decorator once, at class-definition time.
assert calls == ['tag', 'tag']

sample = Sample()
assert sample.greet('bob') == 'hi bob'
assert calls == ['tag', 'tag', 'wrapped']

calls.clear()
assert sample.scaled(3) == 8
assert calls == ['wrapped']

assert sample.fixed() == 42
assert sample.uses_local() == 'local:inner'


# === async methods take decorators too ===
def sync_result(fn):
    def wrapper(self):
        return 'decorated'

    return wrapper


class Async:
    @sync_result
    async def work(self):
        return 'raw'


assert Async().work() == 'decorated'


# === A decorator on a method of a decorated class ===
def add_marker(cls):
    return cls


@add_marker
class Both:
    @tag
    def m(self):
        return 'm'


calls.clear()
assert Both().m() == 'm'
assert calls == ['wrapped']
