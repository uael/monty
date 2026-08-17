# Refcount coverage for operator.attrgetter.
#
# A getter holds no heap references at all: its paths are plain strings. These
# cases pin that, and that nothing it walks over is retained afterwards.
from operator import attrgetter


class Holder:
    def __init__(self, inner: object) -> None:
        self.inner = inner


# The target is named separately, so its count is 1 only if the getter kept no
# reference to the object it was applied to.
target = Holder([1, 2])
getter = attrgetter('inner')
fetched = getter(target)
assert fetched is target.inner

# A dotted walk holds each intermediate only for the step that uses it.
nested = Holder(Holder([3]))
deep = attrgetter('inner.inner')
walked = deep(nested)
assert walked == [3]

# The multi-argument form builds a tuple; the tuple owns the results, and the
# getter still owns nothing.
pair = attrgetter('inner', 'inner')(target)
assert pair == ([1, 2], [1, 2])

# The freeing path: a dropped getter must release cleanly even though it holds
# no heap values, and a failed walk must not strand the intermediates.
gone = attrgetter('inner')
gone = None

failing = attrgetter('inner.missing')
try:
    failing(nested)
    raise AssertionError('expected AttributeError')
except AttributeError:
    pass

len('done')
# `Holder` is held by its binding plus each of the three live instances
# (target, nested, nested.inner). `fetched` is target.inner, named once here,
# once as the attribute, and twice more by the tuple `pair`, which the
# two-argument form built from the same attribute twice.
# ref-counts={'Holder': 4, 'target': 1, 'getter': 1, 'fetched': 4, 'nested': 1, 'deep': 1, 'walked': 2, 'pair': 1, 'failing': 1}
