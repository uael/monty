# Standard library modules

Monty ships a fixed set of built-in stdlib modules. `import` of anything
else raises `ModuleNotFoundError`: there is no `sys.path`, no site-packages,
and no way for sandboxed code to load additional modules.

## Modules available

| Module         | See              |
| -------------- | ---------------- |
| `asyncio`      | ./asyncio.md     |
| `builtins`     | ./builtins.md    |
| `collections`  | ./collections.md |
| `contextlib`   | ./contextlib.md  |
| `contextvars`  | ./contextvars.md |
| `collections.abc` | ./typing.md   |
| `dataclasses`  | ./dataclasses.md |
| `datetime`     | ./datetime.md    |
| `functools`    | ./functools.md   |
| `itertools`    | ./itertools.md   |
| `json`         | ./json.md        |
| `math`         | ./math.md        |
| `os`           | ./os.md          |
| `operator`     | ./operator.md    |
| `os.path`      | ./os.md          |
| `pathlib`      | ./pathlib.md     |
| `re`           | ./re.md          |
| `string.templatelib` | ./string_templatelib.md |
| `sys`          | ./sys.md         |
| `types`        | ./typing.md      |
| `typing`       | ./typing.md      |
| `unicodedata`  | ./unicodedata.md |

`collections` is importable and exposes `deque`, `Counter`, `defaultdict`,
and `namedtuple`; `OrderedDict`, `ChainMap`, and the `UserDict` / `UserList`
/ `UserString` wrappers are missing (see ./collections.md).

A submodule is registered under its full dotted name. `import a.b` binds the
package `a` and reaches the submodule as `a.b`, as in CPython, so
`import os.path` gives the name `os` and `import collections.abc` gives
`collections`. That form needs the package to be a module Monty implements:
`os.path` and `collections.abc` qualify, while `import string.templatelib` is
rejected at compile time because `string` itself is not importable here, and
points at `import string.templatelib as tl` or
`from string.templatelib import Template`, which name the submodule directly.
The `string` package has no other importable submodule (see
./string_templatelib.md).

Each module is built once and cached, as CPython's `sys.modules` does, so every
import of a name hands back that one object: `import sys` twice, or
`import sys` beside `import sys as s`, gives `sys is s`. The cache belongs to
the interpreter state rather than to one execution, so the identity holds
across the snippets of a REPL session and across a session dump and restore.
The `sys.modules` mapping itself is not exposed.

A `gc` module exposing `collect()` / `enable()` / `disable()` is compiled
in only under the `test-hooks` Cargo feature, for Monty's own test suite;
production sandboxes never see it.

## Notable modules NOT available

Common modules that are *not* importable in Monty (non-exhaustive):
`abc`, `argparse`, `array`, `base64`, `bisect`, `copy`, `csv`,
`ctypes`, `decimal`, `enum`, `fractions`,
`hashlib`, `heapq`, `hmac`, `http`, `inspect`, `io`,
`logging`, `multiprocessing`, `pickle`, `queue`, `random`,
`socket`, `struct`, `subprocess`, `tempfile`, `threading`,
`time`, `traceback`, `unittest`, `urllib`, `uuid`, `warnings`, `weakref`,
`zipfile`, `zlib`.

`string` is a special case: the package itself is not importable (no
`Template`/`Formatter`/`ascii_letters`), but its `string.templatelib`
submodule is; see above.

`socket`, `subprocess`, `multiprocessing`, `threading` and `ctypes` are
excluded because they would breach the sandbox. Others (`enum`, `copy`) are
unimplemented and may appear over time.

Some available modules cover only part of their CPython surface: `itertools`
implements eight of its adaptors, `collections` only the four types above,
`functools` only `partialmethod` and `operator` only `attrgetter`. For these
the absent names are missing from the module namespace *and* from the vendored
stub, so they fail type checking as well as raising `AttributeError` at
runtime; see each module's page for the specifics.

## Where the type checker is wider than the runtime

`abc`, `typing_extensions`, `_collections_abc` and `_typeshed` back the
vendored stubs (e.g. `@abstractmethod` on protocol members), so they have to
resolve during type checking. Importing one therefore type-checks clean and
still raises `ModuleNotFoundError` at runtime.

The same gap exists a level down, per name. `math`, `re`, `json`, `datetime`,
`pathlib`, `dataclasses`, `typing` and `types` are vendored from upstream
typeshed verbatim rather than narrowed, so every name CPython's module has
type-checks here: `math.fsum`, `re.subn`, `typing.cast` and
`dataclasses.make_dataclass` all pass `monty -t` and then raise
`AttributeError` or `ImportError` when the code runs. Each module's page below
lists what is actually implemented; the stub is not that list. The narrowed
modules (`asyncio`, `collections`, `contextlib`, `contextvars`, `functools`,
`itertools`, `operator`, `os`, `sys`, `unicodedata`) are cut down to the
implemented surface, and `crates/monty-typeshed/custom/` is where a module
joins them.

What survives the narrowing is a handful of *type* names a stub needs in order
to describe something, but which are not module attributes at runtime:
`asyncio.Timeout` (what `asyncio.timeout()` returns), `contextvars.Token`
(what `ContextVar.set()` returns), `os.PathLike` and `os.stat_result`, and the
`collections.abc` names `collections`'s own stub pulls in. Three of them name
something a program really holds (`type(asyncio.timeout(5)).__name__` is
`'Timeout'`, a `Token` comes back from `set()`, a `StatResult` from `stat()`),
and reaching it *through the module* is what raises `AttributeError`;
`os.PathLike` describes a protocol Monty does not dispatch at all (see
./os.md). Two builtin names diverge the same way, `object` and `UnicodeError`;
see ./builtins.md.
