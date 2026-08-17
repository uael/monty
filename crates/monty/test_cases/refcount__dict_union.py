# Reference counting for the dict union operators.
#
# `|` builds a new dict from both operands and `|=` merges into the left one, so
# every value that lands in a result is a fresh counted reference, every value
# displaced by a collision is released exactly once, and a rejected operand is
# left at the count it came in with. A defaultdict operand's `default_factory` is
# cloned into the result's kind; a class object is a heap-tracked callable
# (unlike a lambda), so it pins that one.

from collections import defaultdict


class Factory:
    pass


# `|` shares the winning value with the operand it came from: the left's own
# 'shared' value never reaches the result, the right's does.
left_value = [1]
right_value = [2]
shared_value = [3]
left = {'a': left_value, 'shared': shared_value}
right = {'shared': right_value}
merged = left | right
assert merged == {'a': [1], 'shared': [2]}

# `|=` replaces in place, so the displaced value drops back to its own binding.
kept_value = [4]
replaced_value = [5]
target = {'k': replaced_value}
target |= {'k': kept_value, 'extra': kept_value}
assert target == {'k': [4], 'extra': [4]}

# A defaultdict union carries the factory across as a second counted reference.
source = defaultdict(Factory, {'a': 1})
derived = source | {'b': 2}
assert derived.default_factory is Factory

# A non-dict right operand is rejected before anything is built.
or_rhs = [('b', 2)]
try:
    {'a': 1} | or_rhs
    assert False, 'expected a list operand to raise'
except TypeError:
    pass

# `|=` takes its own reference to the right operand before merging it, so a
# merge that raises part-way (a flat list has no key/value pairs) has to release
# that reference on the error path too.
ior_rhs = [1, 2]
ior_target = {'k': 0}
try:
    ior_target |= ior_rhs
    assert False, 'expected a flat list to raise'
except TypeError:
    pass

# ref-counts={'Factory': 3, 'left_value': 3, 'right_value': 3, 'shared_value': 2, 'left': 1, 'right': 1, 'merged': 1, 'kept_value': 3, 'replaced_value': 1, 'target': 1, 'source': 1, 'derived': 1, 'or_rhs': 1, 'ior_rhs': 1, 'ior_target': 1}
