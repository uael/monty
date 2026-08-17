from os import PathLike
from typing import AnyStr, overload

# Only the pure lexical `normpath` is implemented; every other `posixpath`
# member (join, dirname, exists, ...) is absent so its use fails type checking
# rather than at runtime.
@overload
def normpath(path: str) -> str: ...
@overload
def normpath(path: PathLike[AnyStr]) -> str: ...
