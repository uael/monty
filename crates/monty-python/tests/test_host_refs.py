"""A host object handed into a session by reference.

Everything else at the boundary is a copy, so an object whose identity or whose
live state is the point could not cross at all; a host had to write a proxy
class inside the guest and answer every method by name. A `MontyRef` replaces
that: the operations the sandbox performs on the proxy come back as calls that
are performed on the real object.
"""

from typing import Any

import pytest
from conftest import RunMonty

from pydantic_monty import MontyError, MontyRef


class Cursor:
    """A host object with state and identity, which is why it cannot be copied."""

    def __init__(self, cwd: str = '/') -> None:
        self.cwd = cwd
        self.seen: list[str] = []

    def bash(self, cmd: str, timeout: int = 120) -> str:
        self.seen.append(cmd)
        return f'{cmd} in {self.cwd} within {timeout}'

    def __call__(self, what: str) -> str:
        return f'called with {what}'

    def __enter__(self) -> str:
        self.seen.append('enter')
        return 'inside'

    def __exit__(self, kind: Any, value: Any, trace: Any) -> bool:
        self.seen.append(f'exit {value!r}')
        return False


def test_attribute_read_reaches_the_object(monty_run: RunMonty):
    cursor = Cursor(cwd='/work')
    assert monty_run('c.cwd', inputs={'c': MontyRef(cursor)}) == '/work'


def test_method_call_runs_on_the_object(monty_run: RunMonty):
    cursor = Cursor(cwd='/work')
    result = monty_run("c.bash('ls', timeout=5)", inputs={'c': MontyRef(cursor)})
    assert result == 'ls in /work within 5'
    assert cursor.seen == ['ls']


def test_the_reference_is_callable(monty_run: RunMonty):
    assert monty_run("f('x')", inputs={'f': MontyRef(Cursor())}) == 'called with x'


def test_the_reference_is_a_context_manager(monty_run: RunMonty):
    cursor = Cursor()
    result = monty_run('with c as inner:\n    x = inner\nx', inputs={'c': MontyRef(cursor)})
    assert result == 'inside'
    assert cursor.seen == ['enter', 'exit None']


def test_a_reference_read_back_is_the_object_itself(monty_run: RunMonty):
    """Identity round trips: what comes back resolves to the same object."""
    cursor = Cursor()
    returned = monty_run('c', inputs={'c': MontyRef(cursor)})
    assert returned is cursor


def test_the_sandbox_cannot_name_a_dunder_on_a_reference(monty_run: RunMonty):
    """`__class__` and everything reachable from it stay out of reach."""
    with pytest.raises(MontyError, match="'Cursor' object has no attribute '__class__'"):
        monty_run('c.__class__', inputs={'c': MontyRef(Cursor())})


def test_a_released_reference_is_dead(monty_run: RunMonty):
    """A host bounds what it has exposed by how long it keeps the wrapper."""
    ref = MontyRef(Cursor())
    ref.release()
    assert ref.value is None
    with pytest.raises(MontyError, match='no longer holds'):
        monty_run('c.cwd', inputs={'c': ref})


def test_a_raise_on_the_host_is_a_raise_in_the_sandbox(monty_run: RunMonty):
    class Refuses:
        @property
        def nope(self) -> str:
            raise ValueError('not today')

    result = monty_run(
        'try:\n    c.nope\nexcept ValueError as e:\n    x = str(e)\nx',
        inputs={'c': MontyRef(Refuses())},
    )
    assert result == 'not today'


def test_a_reference_carries_its_type_name_into_the_sandbox(monty_run: RunMonty):
    assert monty_run('repr(c)', inputs={'c': MontyRef(Cursor())}) == '<Cursor host object>'


def test_two_references_to_one_object_are_one_object(monty_run: RunMonty):
    """Wrapping the same object twice names it twice, not two objects."""
    cursor = Cursor()
    a, b = MontyRef(cursor), MontyRef(cursor)
    assert monty_run('(a == b, len({a, b}))', inputs={'a': a, 'b': b}) == (True, 1)


def test_a_reference_lives_while_any_wrapper_holds_it(monty_run: RunMonty):
    """Releasing one wrapper does not pull the object out from under another."""
    cursor = Cursor(cwd='/work')
    held, spare = MontyRef(cursor), MontyRef(cursor)
    spare.release()
    assert monty_run('c.cwd', inputs={'c': held}) == '/work'
