# Refcount and GC-trace coverage for ContextVar and its Token.
#
# Every case holds objects NOTHING else names, so the strict unreachable walk
# has to go through `for_each_child_id` on each type to reach them — a fixture
# that names them separately passes even with the hook removed.
from contextvars import ContextVar

# The default is an owned reference reachable only through the variable.
defaulted = ContextVar('defaulted', default=[1, 2])

# So is the current value, which is a second owned field, not the default.
both = ContextVar('both', default=[3])
both.set([4])

# A token owns the variable AND the value it displaced, so only tracing both
# reaches either list.
held = ContextVar('held')
held.set([5])
token = held.set([6])

# The freeing path: `py_dec_ref_ids` only runs when the object is released, so
# these must be DROPPED rather than merely held. A missed field leaves its list
# alive with no referrer.
gone_var = ContextVar('gone_var', default=[7])
gone_var.set([8])
gone_var = None

gone_token = ContextVar('gone_token')
gone_token.set([9])
gone_token = None

# A variable that holds itself: nothing outside the cycle names either object,
# so only tracing through the value field can collect it.
cyclic = ContextVar('cyclic')
cyclic.set(cyclic)

# A variable inside a list that the variable itself holds — the cycle runs
# through the container rather than directly.
ring = []
ring_var = ContextVar('ring_var')
ring_var.set(ring)
ring.append(ring_var)

# reset() releases the value it replaced. `replaced` is named separately, so its
# count is 1 only if the reset let go of it; a retained value would leave 2.
replaced = [10]
resettable = ContextVar('resettable')
first = resettable.set(replaced)
resettable.set([11])
resettable.reset(first)

# Resetting back past the first set returns the variable to unset, which must
# release the value it was holding rather than keep it as a stale default.
cleared = ContextVar('cleared')
cleared_token = cleared.set([12])
cleared.reset(cleared_token)

# The refused resets take an early return out of `reset`, the path where a
# missed release is easiest to introduce. Both variables stay usable after.
other = ContextVar('other')
other_token = other.set([13])
try:
    cleared.reset(other_token)
    raise AssertionError('expected ValueError')
except ValueError:
    pass

try:
    other.reset(other_token)
    other.reset(other_token)
    raise AssertionError('expected RuntimeError')
except RuntimeError:
    pass

# A get() that raises LookupError renders the variable's repr on the way out,
# which clones the default; that clone must not outlive the error.
unset = ContextVar('unset')
try:
    unset.get()
    raise AssertionError('expected LookupError')
except LookupError:
    pass

len('done')
# A variable a live token still names is held twice: once by the binding, once
# by the token's own reference to it. `defaulted` and `both` have no live token
# and so stay at 1, which is what makes the token's reference visible here.
# ref-counts={'defaulted': 1, 'both': 1, 'held': 2, 'token': 1, 'cyclic': 2, 'ring': 2, 'ring_var': 2, 'replaced': 1, 'resettable': 2, 'first': 1, 'cleared': 2, 'cleared_token': 1, 'other': 2, 'other_token': 1, 'unset': 1}
