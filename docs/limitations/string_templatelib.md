# `string.templatelib` module

Monty implements PEP 750 template strings: a `t"..."` literal evaluates to a
`string.templatelib.Template`, whose `strings`, `interpolations` and `values`
attributes, iteration, `repr()`/`str()`, and per-`Interpolation` `value`,
`expression`, `conversion` and `format_spec` match CPython 3.14 apart from the
notes below.

## Importing

`from string.templatelib import Template, Interpolation`,
`import string.templatelib as tl` and `import string.templatelib` all work, the
last binding `string` as CPython does.

- **`string` holds `templatelib` alone.** `import string` gives a package with
    no other attribute, so `string.ascii_letters` and the rest of CPython's
    `string` module raise `AttributeError`.
- **The import does not type-check.** The vendored typeshed carries no `string`
    package, so the module does not resolve during type checking even though it
    imports and runs.

## Module contents

The module exposes only the `Template` and `Interpolation` type objects.
CPython also provides `convert(obj, conversion)`, which is absent here: the name
is missing from the module namespace rather than stubbed, so it raises
`ImportError` on `from string.templatelib import convert` and `AttributeError`
on attribute access.

## Behavioural divergences

- **No concatenation.** CPython supports `Template + Template` and
    `Template + str`; Monty raises
    `TypeError: unsupported operand type(s) for +: 'string.templatelib.Template' and ...`.
- **The type objects are not subscriptable.** CPython's `__class_getitem__`
    makes `Template[Any]` a `types.GenericAlias`; in Monty it raises
    `TypeError: type 'string.templatelib.Template' is not subscriptable`.
- **Crossing the host boundary loses the object.** A `Template` or
    `Interpolation` returned to the host arrives as its `repr()` text
    (`Template(strings=('a',), interpolations=())`), not as a host
    `string.templatelib` object and not as anything structured.
