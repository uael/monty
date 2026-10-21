# A local that a closure in an assert or a raise captures is a cell of its function.


def asserted(value, later):
    if value is not None:
        try:
            assert any(isinstance(value, x) for x in [int])
        except Exception:
            raise ValueError('the handler of the try')
    if later:
        raise KeyError('after the try')
    return 'done'


assert asserted(1, False) == 'done', 'an assert whose closure holds'
try:
    asserted('a', False)
except ValueError as e:
    assert e.args == ('the handler of the try',), 'an assert whose closure fails reaches its handler'
try:
    asserted(None, True)
except KeyError as e:
    assert e.args == ('after the try',), 'a raise after a try that did not run is not caught by its handler'
try:
    asserted(1, True)
except KeyError as e:
    assert e.args == ('after the try',), 'a raise after a try that ran is not caught by its handler'


def messaged(value):
    assert value, (lambda: value)()


try:
    messaged(0)
except AssertionError as e:
    assert str(e).split('\n')[0] == '0', 'the message of an assert reads a captured local'


def raised(value):
    raise ValueError((lambda: str(value))())


try:
    raised(3)
except ValueError as e:
    assert e.args == ('3',), 'a raise reads a captured local'


def lambdaed(value):
    assert (lambda: value)() == value
    return value


assert lambdaed(5) == 5, 'an assert reads a captured local'
