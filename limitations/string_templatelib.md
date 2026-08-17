# `string.templatelib` module

Monty implements PEP 750 template strings: a `t"..."` literal evaluates to a
`string.templatelib.Template`, whose `strings`, `interpolations` and `values`
attributes, iteration, `repr()`/`str()`, and per-`Interpolation` `value`,
`expression`, `conversion` and `format_spec` match CPython 3.14 apart from the
notes below.

## Importing

`from string.templatelib import Template, Interpolation` and
`import string.templatelib as tl` both work.

- **`import string` raises `ModuleNotFoundError`.** Monty has no `string`
  module; `templatelib` is registered under its full dotted name only.
- **`import string.templatelib` (no alias) is rejected** with
  `NotImplementedError: importing a submodule without an alias; use \`import
  string.templatelib as <name>\` or \`from string.templatelib import <name>\``.
  Monty interns a dotted module path as one name and has no package objects, so
  the plain form would bind a name no expression can spell; CPython binds
  `string` and reaches the submodule through it.
- **The import does not type-check.** The vendored typeshed carries no `string`
  package (see `crates/monty-typeshed/update.py`), so the module does not resolve
  during type checking even though it imports and runs.

## Module contents

The module exposes only the `Template` and `Interpolation` type objects.
CPython also provides `convert(obj, conversion)`, which is absent here: the name
is missing from the module namespace rather than stubbed, so it raises
`ImportError` on `from string.templatelib import convert` and `AttributeError`
on attribute access.

## Not constructible from Python

`Template` and `Interpolation` are exposed so `isinstance()` works, but calling
them raises `TypeError: cannot create 'string.templatelib.Template' instances`
(and the matching message for `Interpolation`). CPython builds both directly:
`Template('a', Interpolation(42, 'x'), 'b')`. In Monty, a template can only come
from a `t"..."` literal.

## Behavioural divergences

- **No concatenation.** CPython supports `Template + Template` and
  `Template + str`; Monty raises
  `TypeError: unsupported operand type(s) for +: 'string.templatelib.Template' and ...`.
- **The type objects are not subscriptable.** CPython's `__class_getitem__`
  makes `Template[Any]` a `types.GenericAlias`; in Monty it raises
  `TypeError: 'type' object is not subscriptable`.
- **`type(...).__name__` is the dotted name.** `type(t).__name__` is
  `'string.templatelib.Template'` where CPython reports the bare `'Template'`.
  This is Monty's general treatment of types whose CPython `tp_name` is dotted
  (`re.Pattern`, `collections.deque` behave the same way); `repr()` of the type
  and error messages naming it match CPython.
- **Crossing the host boundary loses the object.** A `Template` or
  `Interpolation` returned to the host arrives as its `repr()` text
  (`Template(strings=('a',), interpolations=())`), not as a host
  `string.templatelib` object and not as anything structured.
