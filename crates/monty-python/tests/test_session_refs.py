"""A session value the host holds by reference, and reading a suspended session.

A type object, a class, a template have no copy representation, so they reach
the host as text, which can be printed and nothing else. With
`cross_by_reference=True` each crosses as a `MontySessionRef` the host hands
back to the session, whose own semantics say what it is made of.

The moment a host most needs that is while it is answering a call the sandbox
made: nothing runs then, so the frame is readable, and `snapshot.probe` reads
it without disturbing the suspension.
"""

from typing import Any

import pytest

from pydantic_monty import FunctionSnapshot, Monty, MontyComplete, MontyError, MontyRef, MontySession, MontySessionRef


@pytest.fixture
def refs(pool: Monty):
    """A session that hands back references rather than reprs."""
    with pool.checkout(cross_by_reference=True) as session:
        yield session


def test_a_type_crosses_as_a_reference(refs: MontySession):
    refs.feed_run('class Chunk:\n    pass')
    contract = refs.feed_run('list[Chunk]')
    assert isinstance(contract, MontySessionRef)
    assert contract.value_repr == 'list[Chunk]'
    refs.release_refs(contract.id)


def test_the_host_asks_the_session_what_a_type_is_made_of(refs: MontySession):
    refs.feed_run('class Chunk:\n    pass')
    contract = refs.feed_run('list[Chunk]')
    assert refs.probe('_t.__origin__.__name__', bindings={'_t': contract}) == 'list'
    assert refs.probe('_t.__args__[0].__name__', bindings={'_t': contract}) == 'Chunk'
    refs.release_refs(contract.id)


def test_a_reference_handed_back_is_the_value_itself(refs: MontySession):
    refs.feed_run("class Chunk:\n    kind = 'chunk'")
    held = refs.feed_run('Chunk')
    assert refs.probe('_c is Chunk', bindings={'_c': held})
    assert refs.probe('_c().kind', bindings={'_c': held}) == 'chunk'
    refs.release_refs(held.id)


def test_a_template_crosses_as_a_reference_and_answers_its_parts(refs: MontySession):
    """A t-string reaches the host as its repr otherwise, and had to be
    rebuilt by hand from the text."""
    said = refs.feed_run('x = 41\nt"answer {x + 1}"')
    assert isinstance(said, MontySessionRef)
    first = refs.probe('_s.interpolations[0]', bindings={'_s': said})
    assert refs.probe('(_i.expression, _i.value)', bindings={'_i': first}) == ('x + 1', 42)
    refs.release_refs(first.id)
    refs.release_refs(said.id)


def test_a_reference_survives_the_session_being_put_away_and_woken(pool: Monty):
    with pool.checkout(cross_by_reference=True) as session:
        session.feed_run('class Chunk:\n    pass\ncontract = list[Chunk]')
        contract = session.feed_run('contract')
        state = session.dump()

    with pool.checkout() as woken:
        woken.load_session(state)
        assert woken.probe('_t.__args__[0].__name__', bindings={'_t': contract}) == 'Chunk'
        woken.release_refs(contract.id)


def test_a_released_reference_is_refused(refs: MontySession):
    refs.feed_run('class Chunk:\n    pass')
    contract = refs.feed_run('list[Chunk]')
    refs.release_refs(contract.id)
    with pytest.raises(MontyError, match='released or never made'):
        refs.probe('_t', bindings={'_t': contract})


def test_a_probe_binding_does_not_become_a_name_the_session_has(refs: MontySession):
    assert refs.probe('supplied * 2', bindings={'supplied': 21}) == 42
    with pytest.raises(MontyError, match="name 'supplied' is not defined"):
        refs.feed_run('supplied')


def test_the_host_reads_the_frame_that_is_asking(refs: MontySession):
    """The blocking case: every proof happens while a call is open."""
    suspended = refs.feed_start('class Chunk:\n    pass\nwanted = list[Chunk]\ndecide(1)')
    assert isinstance(suspended, FunctionSnapshot)
    assert suspended.function_name == 'decide'

    assert suspended.probe('wanted.__args__[0].__name__') == 'Chunk'
    # and the rung was untouched by the reading
    done = suspended.resume({'return_value': 42})
    assert isinstance(done, MontyComplete)
    assert done.output == 42


def test_a_contract_reaches_a_host_call_as_a_reference(pool: Monty):
    """The driver's whole shape: sandbox code calls a host object with a
    contract the session defined, and the host reads it before answering."""

    class Cursor:
        def fill(self, want: Any, prompt: str) -> str:
            raise AssertionError('the host answers the call itself, without running this')

    # Held for the whole session: a reference lives exactly as long as the
    # wrapper does, so a temporary here would die before the call it enabled.
    cursor = MontyRef(Cursor())
    with pool.checkout(cross_by_reference=True) as session:
        suspended = session.feed_start(
            "class Chunk:\n    pass\nc.fill(list[Chunk], 'go')",
            inputs={'c': cursor},
        )
        assert isinstance(suspended, FunctionSnapshot)
        assert suspended.function_name == 'fill'
        receiver, contract, prompt = suspended.args
        assert isinstance(receiver, Cursor)
        assert prompt == 'go'
        assert isinstance(contract, MontySessionRef), f'the contract crossed as {contract!r}'
        assert contract.value_repr == 'list[Chunk]'

        assert suspended.probe('_t.__args__[0].__name__', bindings={'_t': contract}) == 'Chunk'
        suspended.release_refs(contract.id)
        done = suspended.resume({'return_value': 'done'})
        assert isinstance(done, MontyComplete)
        assert done.output == 'done'


def test_a_probe_of_a_suspended_session_cannot_suspend_again(refs: MontySession):
    suspended = refs.feed_start('decide(1)')
    assert isinstance(suspended, FunctionSnapshot)
    with pytest.raises(MontyError, match="name 'nobody_supplies_this' is not defined"):
        suspended.probe('nobody_supplies_this')
    # the failure left the suspension resumable
    done = suspended.resume({'return_value': 1})
    assert isinstance(done, MontyComplete)
    assert done.output == 1
