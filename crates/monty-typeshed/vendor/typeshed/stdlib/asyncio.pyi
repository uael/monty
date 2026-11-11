from collections.abc import Awaitable, Generator
from typing import Any, Literal, TypeAlias, TypeVar, overload

_T = TypeVar('_T')
_T1 = TypeVar('_T1')
_T2 = TypeVar('_T2')
_T3 = TypeVar('_T3')
_T4 = TypeVar('_T4')
_T5 = TypeVar('_T5')
_T6 = TypeVar('_T6')

class _Future(Awaitable[_T]):
    """
    Minimal copy of Future from _typeshed/stdlib/_asyncio.pyi: what Monty's future answers, which is what a
    program that settles one from outside a task needs.
    """
    def __iter__(self) -> Generator[Any, None, _T]: ...
    def __await__(self) -> Generator[Any, None, _T]: ...
    def set_result(self, result: _T, /) -> None: ...
    def set_exception(self, exception: type | BaseException, /) -> None: ...
    def exception(self) -> BaseException | None: ...

class _Task(_Future[_T]):
    """A task, as `current_task` gives one: a future with a name."""
    def get_name(self) -> str: ...

class _EventLoop:
    """The running loop, as `get_running_loop` gives it: what makes a future to settle from outside a task."""
    def create_future(self) -> _Future[Any]: ...

_FutureLike: TypeAlias = _Future[_T] | Awaitable[_T]

class CancelledError(BaseException): ...

def run(main: Awaitable[_T], *, debug: bool | None = None, loop_factory: Any = None) -> _T: ...
def get_running_loop() -> _EventLoop: ...
def current_task(loop: _EventLoop | None = None) -> _Task[Any] | None: ...
@overload
def sleep(delay: float) -> _Future[None]: ...
@overload
def sleep(delay: float, result: _T) -> _Future[_T]: ...
@overload
def gather(
    coro_or_future1: _FutureLike[_T1], /, *, return_exceptions: Literal[False] = False
) -> _Future[tuple[_T1]]: ...
@overload
def gather(
    coro_or_future1: _FutureLike[_T1],
    coro_or_future2: _FutureLike[_T2],
    /,
    *,
    return_exceptions: Literal[False] = False,
) -> _Future[tuple[_T1, _T2]]: ...
@overload
def gather(
    coro_or_future1: _FutureLike[_T1],
    coro_or_future2: _FutureLike[_T2],
    coro_or_future3: _FutureLike[_T3],
    /,
    *,
    return_exceptions: Literal[False] = False,
) -> _Future[tuple[_T1, _T2, _T3]]: ...
@overload
def gather(
    coro_or_future1: _FutureLike[_T1],
    coro_or_future2: _FutureLike[_T2],
    coro_or_future3: _FutureLike[_T3],
    coro_or_future4: _FutureLike[_T4],
    /,
    *,
    return_exceptions: Literal[False] = False,
) -> _Future[tuple[_T1, _T2, _T3, _T4]]: ...
@overload
def gather(
    coro_or_future1: _FutureLike[_T1],
    coro_or_future2: _FutureLike[_T2],
    coro_or_future3: _FutureLike[_T3],
    coro_or_future4: _FutureLike[_T4],
    coro_or_future5: _FutureLike[_T5],
    /,
    *,
    return_exceptions: Literal[False] = False,
) -> _Future[tuple[_T1, _T2, _T3, _T4, _T5]]: ...
@overload
def gather(
    coro_or_future1: _FutureLike[_T1],
    coro_or_future2: _FutureLike[_T2],
    coro_or_future3: _FutureLike[_T3],
    coro_or_future4: _FutureLike[_T4],
    coro_or_future5: _FutureLike[_T5],
    coro_or_future6: _FutureLike[_T6],
    /,
    *,
    return_exceptions: Literal[False] = False,
) -> _Future[tuple[_T1, _T2, _T3, _T4, _T5, _T6]]: ...
@overload
def gather(*coros_or_futures: _FutureLike[_T], return_exceptions: Literal[False] = False) -> _Future[list[_T]]: ...
@overload
def gather(coro_or_future1: _FutureLike[_T1], /, *, return_exceptions: bool) -> _Future[tuple[_T1 | BaseException]]: ...
@overload
def gather(
    coro_or_future1: _FutureLike[_T1], coro_or_future2: _FutureLike[_T2], /, *, return_exceptions: bool
) -> _Future[tuple[_T1 | BaseException, _T2 | BaseException]]: ...
@overload
def gather(
    coro_or_future1: _FutureLike[_T1],
    coro_or_future2: _FutureLike[_T2],
    coro_or_future3: _FutureLike[_T3],
    /,
    *,
    return_exceptions: bool,
) -> _Future[tuple[_T1 | BaseException, _T2 | BaseException, _T3 | BaseException]]: ...
@overload
def gather(
    coro_or_future1: _FutureLike[_T1],
    coro_or_future2: _FutureLike[_T2],
    coro_or_future3: _FutureLike[_T3],
    coro_or_future4: _FutureLike[_T4],
    /,
    *,
    return_exceptions: bool,
) -> _Future[tuple[_T1 | BaseException, _T2 | BaseException, _T3 | BaseException, _T4 | BaseException]]: ...
@overload
def gather(
    coro_or_future1: _FutureLike[_T1],
    coro_or_future2: _FutureLike[_T2],
    coro_or_future3: _FutureLike[_T3],
    coro_or_future4: _FutureLike[_T4],
    coro_or_future5: _FutureLike[_T5],
    /,
    *,
    return_exceptions: bool,
) -> _Future[
    tuple[_T1 | BaseException, _T2 | BaseException, _T3 | BaseException, _T4 | BaseException, _T5 | BaseException]
]: ...
@overload
def gather(
    coro_or_future1: _FutureLike[_T1],
    coro_or_future2: _FutureLike[_T2],
    coro_or_future3: _FutureLike[_T3],
    coro_or_future4: _FutureLike[_T4],
    coro_or_future5: _FutureLike[_T5],
    coro_or_future6: _FutureLike[_T6],
    /,
    *,
    return_exceptions: bool,
) -> _Future[
    tuple[
        _T1 | BaseException,
        _T2 | BaseException,
        _T3 | BaseException,
        _T4 | BaseException,
        _T5 | BaseException,
        _T6 | BaseException,
    ]
]: ...
@overload
def gather(*coros_or_futures: _FutureLike[_T], return_exceptions: bool) -> _Future[list[_T | BaseException]]: ...
