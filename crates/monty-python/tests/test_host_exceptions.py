"""What happens to a host exception on its way into the sandbox.

The rule and the reasoning are in `limitations/exceptions.md`; these pin the
behaviour it describes, since the rule is the deliverable and a rule nothing
checks drifts.
"""

from conftest import RunMonty


class Halt(Exception):
    """A host class the sandbox has no counterpart for."""


def _boom() -> None:
    raise Halt('stopped')


def _boom_builtin() -> None:
    raise ValueError('plain')


def test_a_builtin_exception_crosses_as_itself(monty_run: RunMonty):
    code = "try:\n    boom()\nexcept ValueError as e:\n    r = ('ValueError', str(e))\nr"
    assert monty_run(code, external_lookup={'boom': _boom_builtin}) == ('ValueError', 'plain')


def test_a_host_class_arrives_as_a_bare_exception(monty_run: RunMonty):
    code = 'try:\n    boom()\nexcept Exception as e:\n    r = (type(e).__name__, str(e))\nr'
    assert monty_run(code, external_lookup={'boom': _boom}) == ('Exception', 'stopped')


def test_a_sandbox_class_of_the_same_name_does_not_catch_it(monty_run: RunMonty):
    """A sandbox class and a host class are different classes."""
    code = (
        'class Halt(Exception):\n    pass\n'
        'try:\n    boom()\n'
        'except Halt:\n    r = "caught Halt"\n'
        'except Exception as e:\n    r = type(e).__name__\n'
        'r'
    )
    assert monty_run(code, external_lookup={'boom': _boom}) == 'Exception'
