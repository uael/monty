# Monty's surface of a template string: the module is a namespace for the two
# types, so `isinstance(t, Template)` and annotations resolve, and each type
# builds one as CPython does.
from collections.abc import Iterator
from typing import Any, Literal, final

@final
class Template:
    strings: tuple[str, ...]
    interpolations: tuple[Interpolation, ...]

    def __new__(cls, *args: str | Interpolation) -> Template: ...
    def __iter__(self) -> Iterator[str | Interpolation]: ...
    @property
    def values(self) -> tuple[Any, ...]: ...

@final
class Interpolation:
    value: Any
    expression: str
    conversion: Literal["a", "r", "s"] | None
    format_spec: str

    def __new__(
        cls,
        value: Any,
        expression: str = "",
        conversion: Literal["a", "r", "s"] | None = None,
        format_spec: str = "",
    ) -> Interpolation: ...
