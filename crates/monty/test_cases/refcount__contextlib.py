# Refcount and GC-trace coverage for contextlib.suppress.
#
# Every case holds objects NOTHING else names, so the strict unreachable walk
# has to go through `for_each_child_id` to reach them.
from contextlib import suppress

# The exception classes are owned references reachable only through the manager.
live = suppress(ValueError, TypeError)

# The freeing path: `py_dec_ref_ids` only runs on release, so this must be
# DROPPED rather than held. A missed release leaves its arguments unaccounted.
gone = suppress(ValueError)
gone = None

# A manager inside a cycle: the list holds the only reference to a suppress
# that holds the list back, so only tracing through it can collect either.
cyclic = []
cyclic.append(suppress(cyclic))

# Entering and leaving must not leak: the normal exit, the suppressing exit,
# and the propagating exit are three separate paths out of __exit__.
quiet = suppress(ValueError)
with quiet:
    pass

with quiet:
    raise ValueError('suppressed')

declined = suppress(TypeError)
try:
    with declined:
        raise ValueError('propagates')
except ValueError:
    pass

# The __exit__ error path: a non-class argument raises out of __exit__ while an
# exception is already propagating, the narrowest exit from the method.
bad = suppress(1)
try:
    with bad:
        raise ValueError('x')
    raise AssertionError('expected TypeError')
except TypeError:
    pass

len('done')
# ref-counts={'live': 1, 'cyclic': 2, 'quiet': 1, 'declined': 1, 'bad': 1}
