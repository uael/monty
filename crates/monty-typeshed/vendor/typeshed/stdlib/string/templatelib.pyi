# Monty's surface of a template string: the module is a namespace for the two
# types, so `isinstance(t, Template)` and annotations resolve. A program reads a
# template and never makes one, so neither type has a constructor here.
from collections.abc import Iterator
from typing import Any, Literal, final

@final
class Template:
    strings: tuple[str, ...]
    interpolations: tuple[Interpolation, ...]

    def __iter__(self) -> Iterator[str | Interpolation]: ...
    @property
    def values(self) -> tuple[Any, ...]: ...

@final
class Interpolation:
    value: Any
    expression: str
    conversion: Literal["a", "r", "s"] | None
    format_spec: str
