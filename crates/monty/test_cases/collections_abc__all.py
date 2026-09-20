# `collections.abc` exports seven names, which answer `isinstance` and nothing
# else. See `limitations/collections.md`.
from collections import Counter, abc, defaultdict, deque, namedtuple
from collections.abc import Callable, Coroutine, Generator, Iterable, Iterator, Mapping, Sequence

# === Every exported name resolves, from either import form ===
for name in (Callable, Coroutine, Generator, Iterable, Iterator, Mapping, Sequence):
    assert name is not None

assert abc.Callable is Callable
assert abc.Sequence is Sequence

# === Annotations name them without evaluating them ===


def takes(f: Callable[[int], int], m: Mapping) -> Iterable:
    return [f, m]


assert len(takes(1, 2)) == 2


# === Each one answers `isinstance` from what the value is ===
def gen_fn():
    yield 1


def plain():
    pass


assert isinstance(plain, Callable)
assert isinstance(len, Callable)
assert not isinstance(1, Callable)

assert isinstance(gen_fn(), Generator)
assert isinstance((x for x in []), Generator)
assert not isinstance([], Generator)

assert isinstance(iter([]), Iterator)
assert isinstance(gen_fn(), Iterator)
assert not isinstance([], Iterator)

for value in ('s', b's', [], (), {}, set(), range(2), iter([]), gen_fn()):
    assert isinstance(value, Iterable)
assert not isinstance(1, Iterable)

Point = namedtuple('Point', 'x y')
for value in ('s', b's', [], (), range(2), deque([1]), Point(1, 2)):
    assert isinstance(value, Sequence)
assert not isinstance({}, Sequence)

for value in ({}, Counter(), defaultdict(int)):
    assert isinstance(value, Mapping)
assert not isinstance([], Mapping)

# === A tuple of them works, as any classinfo tuple does ===
assert isinstance([], (Mapping, Sequence))
assert not isinstance(1, (Mapping, Sequence))
