# `collections.abc` is annotations only: the names resolve so a program can
# write them, and nothing else. See `limitations/collections.md`.
from collections import abc
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
