from typing import Literal

# A `typing._LiteralGenericAlias` owns its `__args__` tuple, so a missing arm in
# `py_dec_ref_ids` / `for_each_child_id` only shows up under
# `ref-count-return` / `memory-model-checks`.
#
# `word` ends at 3: its binding and the `__args__` of `single` and of `twice`,
# which keeps one of the two equal values.
# `args` and `got` are the same tuple, ending at 3: the binding of each and
# the alias holding it as `__args__`.
word = 'w' * 30
single = Literal[word]
twice = Literal[word, word]
args = single.__args__
got = single.__args__

# Flattening clones the values of `inner` and holds no reference to `inner`
# itself, so `item` ends at 3 (its binding, `inner` and `outer`) and `inner` at 1.
item = [1]
inner = Literal[item]
outer = Literal[inner, 3]

# Nothing but the alias refers to this list, so the walk reaches it through
# `for_each_child_id` or not at all.
anon = Literal[[6, 7]]

# The cycle collector must reach the values of an alias: this list holds the
# only reference to an alias that in turn holds the list.
cyclic = []
cyclic.append(Literal[cyclic])
cyclic = None

# The freeing path: once the only binding goes, a `py_dec_ref_ids` that skips
# `args` leaves the list alive with no referrer.
dropped = Literal[[4, 5]]
dropped = None

len(got)
# ref-counts={'word': 3, 'single': 1, 'twice': 1, 'args': 3, 'got': 3, 'item': 3, 'inner': 1, 'outer': 1, 'anon': 1}
