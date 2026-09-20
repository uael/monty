# cpython-main-module
# `__module__` is the name of the module a class is written in, which the
# harness must seed for CPython because it runs a case with no `__name__`.
class Plain:
    pass


assert Plain.__module__ == '__main__'
assert Plain().__module__ == '__main__'
assert __name__ == '__main__'
