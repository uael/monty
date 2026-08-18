# Refcount and GC-trace coverage for functools.partialmethod.
#
# Each case holds objects NOTHING else names, so the strict unreachable walk has
# to go through `for_each_owned_value` to reach them.
from functools import partialmethod


def take(a: object, b: object = None, c: object = None) -> tuple[object, object, object]:
    return (a, b, c)


# The stored positional and keyword are owned references reachable only through
# the descriptor.
stored = partialmethod(take, [1], c=[2])

# The freeing path: `py_dec_ref_ids` only runs on release, so this must be
# DROPPED. A missed field leaves its list alive with no referrer.
gone = partialmethod(take, [3], c=[4])
gone = None

# A descriptor inside a cycle: the list holds the only reference to a
# partialmethod that holds the list back.
cyclic = []
cyclic.append(partialmethod(take, cyclic))


class Holder:
    def keep(self, first: object = None, second: object = None) -> tuple[object, object]:
        return (first, second)

    fixed = partialmethod(keep, [5])


# Binding builds a bound method that owns both the instance and the descriptor;
# calling it must release everything it built.
holder = Holder()
result = holder.fixed()
assert result == ([5], None)

# The keyword-override path replaces a stored pair, which must release the pair
# it replaced rather than leaking it.
override = partialmethod(take, [6], c=[7])
applied = override(None, c=[8])
assert applied == (None, [6], [8])

# The call error path leaves through an early return with the merged keywords
# already built.
try:
    override(None, [9], c=[10])
    raise AssertionError('expected TypeError')
except TypeError:
    pass

len('done')
# ref-counts={'take': 4, 'stored': 1, 'cyclic': 2, 'Holder': 2, 'holder': 1, 'result': 1, 'override': 1, 'applied': 1}
