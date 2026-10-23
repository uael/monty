# monty

`monty` is a module that CPython does not have. It holds what this interpreter
does for the host that embeds it. Its two functions take a value out of one
namespace and give it to another, which a host that runs code with `exec()` in a
globals dict of its own needs.

## `rebound(x, source, target, memo=None)`

`rebound` is `copy.deepcopy(x, memo)` with one rule more. A function or a class
made under the `exec()` / `eval()` globals dict `source` is made again under
`target`, so the copy reads and changes `target` and not `source`. CPython's
deep copy shares functions and classes, so a copy that it gives stays bound to
the dict of the originals.

- A closure made again gets cells of its own. A cell that two closures share is
    shared by their copies too.
- An instance of a class made again is an instance of the copy, and a bound
    method of such an instance binds the copy of its function.
- A `@property` over a getter made again is made again, and is shared over a
    shared getter.
- What a session has one of is shared: a module, an object or a function of the
    host, and the reports of a dataclass, its fields and its options.
    `copy.deepcopy` refuses these, as the pickler of CPython does.
- A function or a class made under another dict, or under none, is shared, as
    `copy.deepcopy` shares it.

## `instance(cls, fields)`

`instance` gives an instance of `cls`, a class of the session, that holds the
attributes in the dict `fields`. It runs no `__init__`. A host that took an
instance out with its fields uses it to give the instance back, since the
session has no `__new__` to make a bare instance with.

## Divergences

- **There is no type stub for `monty`.** A program that imports it fails a type
    check, so the type check of a session that uses it must be off.
