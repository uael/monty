from types import TracebackType
from typing import Generic, TypeVar

_T_co = TypeVar('_T_co', covariant=True)

# `contextmanager`, `ExitStack`, `closing`, `redirect_stdout`/`redirect_stderr`
# and every async counterpart are absent: Monty has no generators to build a
# `@contextmanager` from.

class AbstractContextManager(Generic[_T_co]):
    def __enter__(self) -> _T_co: ...
    def __exit__(
        self, exc_type: type[BaseException] | None, exc_value: BaseException | None, traceback: TracebackType | None, /
    ) -> bool | None: ...

class suppress:
    def __init__(self, *exceptions: type[BaseException]) -> None: ...
    def __enter__(self) -> None: ...
    def __exit__(
        self, exc_type: type[BaseException] | None, exc_value: BaseException | None, traceback: TracebackType | None, /
    ) -> bool: ...
