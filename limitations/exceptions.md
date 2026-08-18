# Exceptions

Monty implements the fixed set of builtin exception classes listed below, and
sandboxed code can subclass them (see "Custom subclasses"). `raise MyClass()`
on a class that does *not* derive from one raises
`TypeError: exceptions must derive from BaseException`, as in CPython.

## Implemented exception classes

`BaseException`, `Exception`, `SystemExit`, `KeyboardInterrupt`,
`ArithmeticError`, `OverflowError`, `ZeroDivisionError`, `LookupError`,
`IndexError`, `KeyError`, `RuntimeError`, `NotImplementedError`,
`RecursionError`, `AttributeError`, `FrozenInstanceError`, `NameError`,
`UnboundLocalError`, `ValueError`, `UnicodeDecodeError`, `UnicodeEncodeError`,
`ImportError`, `ModuleNotFoundError`, `OSError`, `FileNotFoundError`, `FileExistsError`,
`IsADirectoryError`, `NotADirectoryError`, `PermissionError`,
`AssertionError`, `MemoryError`, `StopIteration`, `StopAsyncIteration`,
`GeneratorExit`, `SyntaxError`, `TimeoutError`, `TypeError`.

Module-specific: `json.JSONDecodeError` (subclass of `ValueError`),
`re.PatternError` / `re.error`, `io.UnsupportedOperation` (catchable as
both `OSError` and `ValueError`, matching CPython's dual parentage), and
`asyncio.CancelledError` (a direct `BaseException` subclass, so
`except Exception:` does not catch it), `asyncio.InvalidStateError`,
`asyncio.QueueEmpty`, `asyncio.QueueFull`. None of these are builtins: each
is reachable only through its module, as in CPython. `asyncio.TimeoutError`
is the builtin `TimeoutError` re-exported, which is what it has been since
3.11.

## Exception classes NOT implemented

`Warning` and all its subclasses (`DeprecationWarning`, etc.),
`BufferError`, `EOFError`, `FloatingPointError`, `GeneratorExit`,
`ConnectionError` and subclasses (`ConnectionAbortedError`,
`ConnectionRefusedError`, `ConnectionResetError`,
`BrokenPipeError`), `BlockingIOError`, `ChildProcessError`,
`InterruptedError`, `ProcessLookupError`, `ReferenceError`,
`SystemError`, `TabError`, `IndentationError`,
`UnicodeError` (parent), `UnicodeTranslateError`,
`EncodingWarning`, `EnvironmentError` / `IOError` aliases,
`ExceptionGroup` / `BaseExceptionGroup` (see ./language.md).

## Constructor signature

Exception constructors take any number of positional arguments, which become
`exc.args`. **Keyword arguments are rejected** with
`TypeError: <Type>() takes no keyword arguments`; CPython's `BaseException`
rejects them too, but its subclasses with real signatures (e.g.
`UnicodeDecodeError`) accept more shapes than Monty models. The builtin types
that carry structured fields in CPython (`OSError`'s `errno`/`strerror`/
`filename`, `UnicodeDecodeError`'s five fields) still store their arguments as
plain `args` here: those attributes are not exposed.

## Attributes

- `exc.args`: the constructor's positional arguments, always a `tuple`.
- `str(exc)`: `""` for no arguments, the sole argument for one, the args
  tuple's repr for more, matching `BaseException.__str__`.
- `repr(exc)`: `ClassName(arg_reprs)` matching CPython, **except**
  `UnicodeDecodeError`/`UnicodeEncodeError` raised by Monty's own codec paths:
  CPython reprs these from their real 5-field constructor
  (`UnicodeDecodeError('ascii', b'\xff', 0, 1, 'ordinal not in range(128)')`),
  which Monty doesn't track, so Monty's `repr()` uses the generic
  single-message form instead.
- `exc.__cause__` / `exc.__context__`: the explicit `raise X from Y` cause and
  the implicit chain, both `None` when unset.

**Not implemented:** `__suppress_context__`, `__traceback__`, `__notes__`,
`add_note()`. `raise X from Y` records `__cause__`, but the traceback printed
when the exception escapes shows only the final exception, never CPython's
"The above exception was the direct cause of ..." chain.

## Custom subclasses

`class MyError(Exception): pass` works, as do deeper chains
(`class Halt(Fault)` where `class Fault(Exception)`). Such a class can be
raised, caught by its own name or by any base including `Exception`, given a
custom `__init__` (with or without `super().__init__(...)`), extra instance
attributes, and methods. `isinstance`/`issubclass` walk the chain.

Divergences:

- **The exception hierarchy above the sandbox class is Monty's fixed one.** A
  subclass inherits from exactly one builtin ancestor, and `except` matching
  against builtin classes goes through that ancestor rather than a real MRO.
- **A sandbox exception crossing the host boundary is reported under its own
  name but with its nearest builtin ancestor as the type a binding
  reconstructs.** `class Halt(Exception)` escaping the sandbox gives a
  `MontyException` whose `user_type()` is `"Halt"` and whose `exc_type()` is
  `Exception`; the traceback's final line reads `Halt: message`, where CPython
  writes `__main__.Halt: message` (Monty never qualifies class names, see
  ./classes.md).
- **A host binding rebuilds it as the builtin ancestor.** The name survives the
  wire (`MontyException::user_type`) and the rendered traceback, but
  `pydantic_monty` / `@pydantic/monty` raise the native Python exception for
  `exc_type`, so a caller catching it sees `Exception('done')`, not a `Halt`
  class. There is no sandbox class object to reconstruct host-side.
- **The traceback message is `str(exc)` computed at raise time**, not at print
  time. A custom `__str__` is used, but one that mutates state or depends on
  attributes set after the raise renders differently from CPython. A `__str__`
  that itself raises falls back to `<unprintable Foo object>`.
- The rest of the class-level divergences (single inheritance, `super()`
  resolution, no `__mro__`) are in ./classes.md.

## Control flow in `finally`

`break`/`continue`/`return` inside a `finally` block follows CPython
semantics (the finally body runs exactly once and a `return`/`break`/
`continue` that exits it discards any in-flight exception), but Monty does
not emit CPython 3.14's PEP 765 `SyntaxWarning` for such statements, having
no warnings machinery.

## Traceback behaviour

Tracebacks are formatted to match CPython, including the
`File "...", line N, in <function>` lines and `~` caret markers (Monty
uses `~` where CPython uses `^`; the test harness normalizes between
them). Frame names use `<module>` for top-level code.

Known caret divergences:

- CPython suppresses carets on a frame whose location is exactly the call in a
  simple `name = f(...)` assignment or `return f(...)` statement (a noise
  heuristic in `traceback._should_show_carets`); Monty always draws carets for
  the frame's range.
- For a frame whose location spans multiple lines (e.g. a caller frame covering
  a whole multi-line `class` statement), Monty renders the CPython-style source
  block (all lines when the range covers at most three, otherwise
  `...<N lines>...` elision) but never draws caret markers under it, where
  CPython draws multi-line carets for partial-line ranges such as a multi-line
  binary expression.

Monty never emits CPython's `Did you mean: '...'?` suggestions on
`NameError`/`AttributeError`. This divergence is invisible to the test
suite: `scripts/run_traceback.py` strips the suggestions from CPython's output
before comparison, so traceback tests cannot catch it.

An exception raised inside a Python callable that native code invokes
*synchronously* — the `key=`/predicate/function argument of `map`, `filter`,
`sorted`/`min`/`max`, and a user-defined
`__iter__`/`__next__`/`__contains__`/`__repr__`/`__str__` — omits the **calling**
frame from its traceback; the callee frame is present.
CPython shows both. The re-entrant call path (`evaluate_function`) does not
splice the host call site into the traceback. The exception type and message
are unaffected.
