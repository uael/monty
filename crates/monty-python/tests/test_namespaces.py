"""Several global namespaces over one session's heap.

A namespace is a name map, not a heap. Copying one from another copies the
names and shares the objects, so a rebinding through either is invisible to the
other while a mutation of an object both name is seen by both. Copying the heap
instead is `dump()` and `load_session()`, which shares nothing.
"""

import pytest

from pydantic_monty import FunctionSnapshot, Monty, MontyComplete, MontyError, MontySession


@pytest.fixture
def session(pool: Monty):
    with pool.checkout() as s:
        yield s


def test_a_copied_namespace_shares_objects_and_not_bindings(session: MontySession):
    session.feed_run('board = []')
    session.feed_run('x = 1')
    parent = session.select_namespace(0)
    child = session.copy_namespace(parent)

    # A mutation crosses, child to parent.
    session.select_namespace(child)
    session.feed_run("board.append('c')")
    session.select_namespace(parent)
    assert session.feed_run('board') == ['c']

    # And parent to child.
    session.feed_run("board.append('p')")
    session.select_namespace(child)
    assert session.feed_run('board') == ['c', 'p']

    # A rebinding crosses neither way.
    session.feed_run('x = 2')
    assert session.feed_run('x') == 2
    session.select_namespace(parent)
    assert session.feed_run('x') == 1


def test_a_name_only_the_child_binds_is_not_the_parents(session: MontySession):
    session.feed_run('shared = 1')
    child = session.copy_namespace(0)

    session.select_namespace(child)
    session.feed_run('only_child = 7')
    assert session.feed_run('only_child') == 7

    session.select_namespace(0)
    with pytest.raises(MontyError, match='only_child'):
        session.feed_run('only_child')


def test_an_empty_namespace_binds_nothing(session: MontySession):
    session.feed_run('x = 1')
    fresh = session.create_namespace()
    assert fresh != 0

    session.select_namespace(fresh)
    with pytest.raises(MontyError, match='x'):
        session.feed_run('x')
    session.feed_run("x = 'mine'")
    assert session.feed_run('x') == 'mine'

    session.select_namespace(0)
    assert session.feed_run('x') == 1


def test_a_function_reads_the_namespace_it_was_defined_in(session: MontySession):
    session.feed_run('scale = 10')
    session.feed_run('def scaled(v):\n    return v * scale')
    child = session.copy_namespace(0)

    session.select_namespace(child)
    session.feed_run('scale = 100')
    assert session.feed_run('scale') == 100
    # A rebinding does not cross, so the parent's function keeps reading the
    # parent's binding however far from home it is called.
    assert session.feed_run('scaled(2)') == 20


def test_probe_reads_a_named_namespace(session: MontySession):
    session.feed_run('who = "parent"')
    child = session.copy_namespace(0)
    session.select_namespace(child)
    session.feed_run('who = "child"')
    session.select_namespace(0)

    assert session.probe('who') == 'parent'
    assert session.probe('who', namespace=child) == 'child'
    # And probing leaves the selection where it was.
    assert session.probe('who') == 'parent'


def test_probe_reads_a_second_namespace_while_the_first_is_suspended(session: MontySession):
    session.feed_run('board = []')
    session.feed_run('who = "parent"')
    child = session.copy_namespace(0)
    session.select_namespace(child)
    session.feed_run('who = "child"')
    session.select_namespace(0)

    snapshot = session.feed_start('board.append(ask(1))')
    assert isinstance(snapshot, FunctionSnapshot)
    # Nothing is running, so every namespace is readable, not just the one
    # that asked.
    assert snapshot.probe('who') == 'parent'
    assert snapshot.probe('who', namespace=child) == 'child'
    # A write through the parked namespace reaches the object the suspended
    # snippet is about to append to.
    snapshot.probe("board.append('while suspended')", namespace=child)

    done = snapshot.resume({'return_value': 2})
    assert isinstance(done, MontyComplete)
    # `append` is what the snippet evaluated to, so the feed says None; the
    # list is what the two namespaces both wrote to.
    assert done.output is None
    assert session.feed_run('board') == ['while suspended', 2]


def test_namespaces_survive_dump_and_load(pool: Monty):
    with pool.checkout() as session:
        session.feed_run('board = []')
        session.feed_run('x = 1')
        child = session.copy_namespace(0)
        session.select_namespace(child)
        session.feed_run('x = 2')
        session.feed_run("board.append('before')")
        session.select_namespace(0)
        state = session.dump()

    with pool.checkout() as woken:
        woken.load_session(state)
        # Distinct bindings survive.
        assert woken.feed_run('x') == 1
        assert woken.probe('x', namespace=child) == 2
        # And the two maps still name one object, not two copies of it.
        woken.probe("board.append('after')", namespace=child)
        assert woken.feed_run('board') == ['before', 'after']


def test_a_fork_shares_nothing(pool: Monty):
    with pool.checkout() as session:
        session.feed_run('board = []')
        child = session.copy_namespace(0)
        state = session.dump()

    with pool.checkout() as left, pool.checkout() as right:
        left.load_session(state)
        right.load_session(state)

        left.probe("board.append('left')", namespace=child)
        assert left.feed_run('board') == ['left']
        assert right.feed_run('board') == []
        assert right.probe('board', namespace=child) == []


def test_releasing_a_namespace_keeps_what_another_still_names(session: MontySession):
    session.feed_run('board = []')
    child = session.copy_namespace(0)
    session.probe("board.append('written')", namespace=child)

    assert session.release_namespace(child) == child
    with pytest.raises(MontyError):
        session.release_namespace(child)

    # The list outlived the namespace that wrote to it, because the parent
    # still names it.
    assert session.feed_run('board') == ['written']


def test_the_selected_namespace_cannot_be_released(session: MontySession):
    session.feed_run('x = 1')
    with pytest.raises(MontyError):
        session.release_namespace(0)
    assert session.feed_run('x') == 1


def test_a_handle_naming_nothing_is_refused(session: MontySession):
    session.feed_run('x = 1')
    gone = session.create_namespace()
    session.release_namespace(gone)

    with pytest.raises(MontyError):
        session.select_namespace(gone)
    with pytest.raises(MontyError):
        session.probe('x', namespace=gone)
