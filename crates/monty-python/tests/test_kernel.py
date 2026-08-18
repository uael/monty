"""The session surface a host drives an interpreter through: a per-feed step
budget, reading source without running it, a module-level `return` as an
outcome, and evaluating one expression against a live namespace."""

from __future__ import annotations

import pytest
from conftest import RunMonty
from inline_snapshot import snapshot

from pydantic_monty import (
    AsyncMonty,
    Monty,
    MontyComplete,
    MontyInstance,
    MontyRuntimeError,
    MontySession,
    MontySyntaxError,
)

SPIN = 'n = 0\nwhile n < 200_000:\n    n += 1\nn'


def test_a_feed_carries_its_own_budget(session: MontySession):
    with pytest.raises(MontyRuntimeError) as first:
        session.feed_run(SPIN, max_steps=5_000)
    assert str(first.value) == snapshot('RuntimeError: call step limit exceeded: 5120 instructions > 5000')

    # The count is per feed, so the same source under the same budget fails the
    # same way however much the session spent before it.
    with pytest.raises(MontyRuntimeError) as again:
        session.feed_run(SPIN, max_steps=5_000)
    assert str(again.value) == str(first.value)

    # The overrun ended a feed, not the session.
    assert session.feed_run(SPIN) == snapshot(200000)


def test_a_wide_budget_does_not_fire(session: MontySession):
    assert session.feed_run('sum(range(100))', max_steps=10_000_000) == snapshot(4950)


def test_the_session_budget_still_applies_under_a_wide_per_feed_one(monty_run: RunMonty):
    with pytest.raises(MontyRuntimeError) as exc:
        monty_run(SPIN, limits={'max_steps': 5_000}, max_steps=10_000_000)
    assert str(exc.value) == snapshot('RuntimeError: step limit exceeded: 5120 instructions > 5000')


def test_a_written_return_is_an_outcome_of_its_own(session: MontySession):
    closed = session.feed_start('x = 1\nreturn x + 41\nx = 99')
    assert isinstance(closed, MontyComplete)
    assert (closed.output, closed.returned) == snapshot((42, True))
    # the `return` cut the body short, so the line after it never ran
    assert session.feed_run('x') == snapshot(1)

    trailing = session.feed_start('x + 1')
    assert isinstance(trailing, MontyComplete)
    assert (trailing.output, trailing.returned) == snapshot((2, False))

    nothing = session.feed_start('y = 5')
    assert isinstance(nothing, MontyComplete)
    assert (nothing.output, nothing.returned) == snapshot((None, False))

    inside = session.feed_start('def f():\n    return 7\nf()')
    assert isinstance(inside, MontyComplete)
    assert (inside.output, inside.returned) == snapshot((7, False))


def test_parse_separates_unfinished_input_from_a_real_error(session: MontySession):
    unfinished = session.parse('values = [1,')
    assert (unfinished.complete, unfinished.error) == snapshot((False, None))

    wrong = session.parse('values = )', script_name='rung.py')
    assert wrong.complete is True
    assert wrong.error is not None
    assert wrong.error.display('type-msg') == snapshot('SyntaxError: Expected an expression')
    assert 'rung.py' in wrong.error.display()

    fine = session.parse('values = [1]')
    assert (fine.complete, fine.error, fine.binds_global, fine.stores) == snapshot((True, None, False, []))


def test_parse_reports_the_bindings_asked_about(session: MontySession):
    facts = session.parse('total = 1\ndef helper():\n    spare = 2', stores=['total', 'spare', 'helper'])
    assert facts.stores == snapshot(['total', 'helper'])
    assert facts.binds_global is False

    assert session.parse('def f():\n    global g').binds_global is True


def test_parse_runs_nothing(session: MontySession):
    session.feed_run('marker = 1')
    facts = session.parse('marker = 999\nraise RuntimeError("never")')
    assert facts.complete is True
    assert session.feed_run('marker') == snapshot(1)


def test_probe_reads_the_namespace_and_leaves_it_alone(session: MontySession):
    session.feed_run('items = [1, 2]\nfactor = 10')
    assert session.probe('sum(items) * factor') == snapshot(30)
    assert session.feed_run('items') == snapshot([1, 2])


def test_probe_refuses_anything_that_binds(session: MontySession):
    session.feed_run('value = 1')

    with pytest.raises(MontySyntaxError) as statement:
        session.probe('value = 2')
    assert statement.value.display('msg') == snapshot('a probe evaluates one expression, not a statement')

    with pytest.raises(MontySyntaxError) as walrus:
        session.probe('(spare := 2)')
    assert walrus.value.display('msg') == snapshot('a probe evaluates an expression that binds nothing')

    assert session.feed_run('value') == snapshot(1)


def test_probe_resolves_names_from_the_host(session: MontySession):
    def double(n: int) -> int:
        return n * 2

    assert session.probe('double(21)', external_lookup={'double': double}) == snapshot(42)


def test_probe_carries_its_own_budget(session: MontySession):
    session.feed_run('def spin():\n    n = 0\n    while n < 200_000:\n        n += 1\n    return n')
    with pytest.raises(MontyRuntimeError) as exc:
        session.probe('spin()', max_steps=5_000)
    assert str(exc.value) == snapshot('RuntimeError: call step limit exceeded: 5120 instructions > 5000')


def test_imports_hand_back_one_module_object(monty_run: RunMonty):
    assert monty_run('import sys\nimport sys as other\nsys is other') == snapshot(True)


async def test_async_session_parses_and_probes():
    async with AsyncMonty() as pool:
        async with pool.checkout() as s:
            await s.feed_run('scale = 3')
            assert await s.probe('scale * 14') == snapshot(42)
            facts = await s.parse('scale = 4', stores=['scale'])
            assert (facts.complete, facts.stores) == snapshot((True, ['scale']))
            with pytest.raises(MontyRuntimeError):
                await s.feed_run(SPIN, max_steps=5_000)


CLASSES = """
from dataclasses import dataclass

@dataclass
class Point:
    x: int
    y: int

    def total(self) -> int:
        return self.x + self.y


class Plain:
    def __init__(self, n: int):
        self.n = n

    def double(self) -> int:
        return self.n * 2


point = Point(1, 41)
plain = Plain(21)
"""


def test_an_instance_crosses_out_as_its_shape(session: MontySession):
    session.feed_run(CLASSES)

    point = session.feed_run('point')
    assert isinstance(point, MontyInstance)
    assert point.class_name == snapshot('Point')
    assert point.attrs == snapshot({'x': 1, 'y': 41})
    assert 'total' in point.members
    assert repr(point) == snapshot('Point(x=1, y=41)')

    plain = session.feed_run('plain')
    assert isinstance(plain, MontyInstance)
    assert (plain.class_name, plain.attrs) == snapshot(('Plain', {'n': 21}))


def test_an_instance_is_usable_in_a_session_woken_from_the_dump(pool: Monty):
    with pool.checkout() as origin:
        origin.feed_run(CLASSES)
        point = origin.feed_run('point')
        plain = origin.feed_run('plain')
        seed = origin.dump()

    with pool.checkout() as woken:
        woken.load_session(seed)
        carried = {'carried': point}
        assert woken.feed_run('carried.x + carried.y', inputs=carried) == snapshot(42)
        assert woken.feed_run('isinstance(carried, Point)', inputs=carried) is True
        assert woken.feed_run('carried.total()', inputs=carried) == snapshot(42)
        assert woken.feed_run('other.double()', inputs={'other': plain}) == snapshot(42)
        # and it crosses back out unchanged
        assert woken.feed_run('carried', inputs=carried).attrs == snapshot({'x': 1, 'y': 41})


def test_two_sessions_woken_from_one_dump_share_nothing(pool: Monty):
    with pool.checkout() as origin:
        origin.feed_run('items = [1]\ncount = 0')
        seed = origin.dump()

    with pool.checkout() as first, pool.checkout() as second:
        first.load_session(seed)
        second.load_session(seed)
        first.feed_run('items.append(2)\ncount = 1')
        assert first.feed_run('items') == snapshot([1, 2])
        assert second.feed_run('items') == snapshot([1])
        assert second.feed_run('count') == snapshot(0)


def test_a_session_without_the_class_refuses_the_instance(pool: Monty):
    with pool.checkout() as origin:
        origin.feed_run(CLASSES)
        point = origin.feed_run('point')

    with pool.checkout() as stranger:
        with pytest.raises(MontyRuntimeError) as exc:
            stranger.feed_run('carried', inputs={'carried': point})
        assert str(exc.value) == snapshot(
            'RuntimeError: invalid input type: Point names no class this session defines with those members'
        )
