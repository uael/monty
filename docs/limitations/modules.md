# Standard library modules

Monty ships a fixed set of built-in stdlib modules. `import` of a name that is
neither one of them nor bound in `sys.modules` raises `ModuleNotFoundError`:
there is no `sys.path`, no site-packages, and nothing reads a file, so a module
of your own reaches the sandbox only by being put in `sys.modules`.

`sys.modules` is the session's own table, and `import` answers from it first, as
in CPython. So a module is built once and every later import of that name finds
the same object, across feeds and across a dump. Binding a name in it is what
makes `import <name>` work for anything else: `sys.modules['x'] = obj` then
`import x` binds `obj`, whatever `obj` is, exactly as CPython does.

`import` is the only thing that reads the table. `__import__` and `importlib`
are absent, a dotted name is not split into parent packages, and nothing else
(a `from x import y`, a submodule, a reload) consults or writes it.
Module attributes still cannot be set (`sys.x = 1` raises `AttributeError`), so
state is shared through a module a program built itself rather than through a
stdlib one.

## Modules available

| Module               | See                                            |
| -------------------- | ---------------------------------------------- |
| `ast`                | [ast.md](ast.md)                               |
| `asyncio`            | [asyncio.md](asyncio.md)                       |
| `base64`             | [base64.md](base64.md)                         |
| `builtins`           | [builtins.md](builtins.md)                     |
| `binascii`           | [base64.md](base64.md)                         |
| `collections`        | [collections.md](collections.md)               |
| `collections.abc`    | [collections.md](collections.md)               |
| `contextvars`        | [contextvars.md](contextvars.md)               |
| `copy`               | [copy.md](copy.md)                             |
| `dataclasses`        | [dataclasses.md](dataclasses.md)               |
| `datetime`           | [datetime.md](datetime.md)                     |
| `functools`          | [functools.md](functools.md)                   |
| `itertools`          | [itertools.md](itertools.md)                   |
| `json`               | [json.md](json.md)                             |
| `math`               | [math.md](math.md)                             |
| `os`                 | [os.md](os.md)                                 |
| `pathlib`            | [pathlib.md](pathlib.md)                       |
| `random`             | [random.md](random.md)                         |
| `re`                 | [re.md](re.md)                                 |
| `string.templatelib` | [string_templatelib.md](string_templatelib.md) |
| `sys`                | [sys.md](sys.md)                               |
| `time`               | [time.md](time.md)                             |
| `typing`             | [typing.md](typing.md)                         |
| `unicodedata`        | [unicodedata.md](unicodedata.md)               |

`collections` is importable and exposes `deque`, `Counter`, `defaultdict`,
and `namedtuple`; `OrderedDict`, `ChainMap`, and the `UserDict` / `UserList`
/ `UserString` wrappers are missing (see [collections.md](collections.md)).

A `gc` module exposing `collect()` / `enable()` / `disable()` is compiled
in only under the `test-hooks` Cargo feature, for Monty's own test suite;
production sandboxes never see it.

## Notable modules NOT available

Common modules that are *not* importable in Monty (non-exhaustive):
`abc`, `argparse`, `array`, `bisect`, `contextlib`, `csv`,
`ctypes`, `decimal`, `enum`, `fractions`,
`hashlib`, `heapq`, `hmac`, `http`, `inspect`, `io`,
`logging`, `multiprocessing`, `operator`, `pickle`, `queue`,
`socket`, `string`, `struct`, `subprocess`, `tempfile`, `threading`,
`traceback`, `unittest`, `urllib`, `uuid`, `warnings`, `weakref`,
`zipfile`, `zlib`.

`socket`, `subprocess`, `multiprocessing`, `threading` and `ctypes` are
excluded because they would breach the sandbox. Others (`enum`, `operator`)
are unimplemented and may appear over time.

Some available modules cover only part of their CPython surface: `functools`
implements only `reduce` and `partial`, `copy` only `copy()` and `deepcopy()`,
`time` only `time()` and `sleep()`, and `collections` only the four types above.
The absent names are missing from
the module namespace rather than stubbed, so they fail type checking as well as
raising `AttributeError` at runtime; see each module's page for the specifics.

## Modules the type checker resolves but the runtime does not

`abc`, `types`, `typing_extensions`, `_collections_abc` and `_typeshed` back
the vendored stubs (e.g. `@abstractmethod` on protocol members), so they have
to resolve during type checking. Importing them therefore type-checks clean but
still raises `ModuleNotFoundError` at runtime.
