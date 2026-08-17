from typing import Any, final

# Only `attrgetter`. The arithmetic/comparison helpers (`add`, `lt`, ...) and
# the sibling factories (`itemgetter`, `methodcaller`) are absent, so using one
# fails type checking rather than only at runtime.

@final
class attrgetter:
    def __init__(self, attr: str, /, *attrs: str) -> None: ...
    def __call__(self, obj: Any, /) -> Any: ...
