from typing import Any, Generic, TypeVar

_T = TypeVar('_T')

# Only `partialmethod`. `partial`, `reduce`, `wraps`, `cache`/`lru_cache`,
# `cached_property` and `total_ordering` are absent, so using one fails type
# checking rather than only at runtime.

class partialmethod(Generic[_T]):
    func: Any
    args: tuple[Any, ...]
    keywords: dict[str, Any]
    def __init__(self, func: Any, /, *args: Any, **keywords: Any) -> None: ...
    def __get__(self, obj: Any, cls: type[Any] | None = None) -> Any: ...
