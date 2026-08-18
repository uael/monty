"""A fed snippet carrying a filename of the caller's choosing.

A session's snippets are otherwise named `<python-input-N>`, which is right
for a REPL and wrong for a host feeding chunks that have names of their own.
"""

import pytest

from pydantic_monty import MontyRuntimeError, MontySession


def test_a_fed_snippet_carries_the_name_it_was_given(session: MontySession):
    with pytest.raises(MontyRuntimeError) as caught:
        session.feed_run('raise ValueError("boom")', script_name='chunk-7.py')
    rendered = caught.value.display()
    assert 'chunk-7.py' in rendered, rendered
    assert '<python-input-' not in rendered, rendered


def test_a_function_keeps_its_defining_snippet_name(session: MontySession):
    """A traceback from a later snippet still names where the callee came from."""
    session.feed_run('def boom():\n    raise ValueError("inner")', script_name='defs.py')
    with pytest.raises(MontyRuntimeError) as caught:
        session.feed_run('boom()', script_name='caller.py')
    rendered = caught.value.display()
    assert 'defs.py' in rendered, rendered
    assert 'caller.py' in rendered, rendered


def test_no_name_takes_the_generated_one(session: MontySession):
    with pytest.raises(MontyRuntimeError) as caught:
        session.feed_run('raise ValueError("boom")')
    assert '<python-input-' in caught.value.display()
