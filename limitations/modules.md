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
| `collections.abc` | ./typing.md   |
| `contextlib`   | ./contextlib.md  |
| `contextvars`  | ./contextvars.md |
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
`import os.path` gives the name `os`. That form needs the package to be a
module Monty implements: `os.path` qualifies, while `import string.templatelib`
is rejected at compile time because `string` itself is not importable here, and
points at `import string.templatelib as tl` or
`from string.templatelib import Template`, which name the submodule directly.
The `string` package has no other importable submodule (see
./string_templatelib.md). `collections.abc` is registered the same way, so
`from collections.abc import Mapping` and `import collections.abc as abc` work
while the unaliased `import collections.abc` is rejected.

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
implements just `count` and `repeat` so far, and `collections` only the four
types above. The absent names are missing from the module namespace rather than
stubbed, so they fail type checking as well as raising `AttributeError` at
runtime; see each module's page for the specifics.

## Modules the type checker resolves but the runtime does not

`abc`, `typing_extensions`, `_collections_abc`, `builtins` and `_typeshed` back
the vendored stubs (e.g. `@abstractmethod` on protocol members), so they have
to resolve during type checking. Importing them therefore type-checks clean but
still raises `ModuleNotFoundError` at runtime — which for `builtins` also means
`vars(builtins)` has no way to be written.
