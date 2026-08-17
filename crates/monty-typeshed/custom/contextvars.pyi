from typing import Any, Generic, TypeVar, final, overload

_T = TypeVar('_T')
_D = TypeVar('_D')

# `Context`, `copy_context` and `ContextVar.get`'s per-context lookup are absent:
# Monty has a single context, so the value lives on the variable itself.

@final
class Token(Generic[_T]):
    @property
    def var(self) -> ContextVar[_T]: ...
    # Monty exposes no `Token.MISSING`, so an unset previous value reads as None.
    @property
    def old_value(self) -> Any: ...

@final
class ContextVar(Generic[_T]):
    @overload
    def __init__(self, name: str) -> None: ...
    @overload
    def __init__(self, name: str, *, default: _T) -> None: ...
    @property
    def name(self) -> str: ...
    @overload
    def get(self) -> _T: ...
    @overload
    def get(self, default: _D, /) -> _D | _T: ...
    def set(self, value: _T, /) -> Token[_T]: ...
    def reset(self, token: Token[_T], /) -> None: ...
