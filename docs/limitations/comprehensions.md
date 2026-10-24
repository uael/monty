# List / set / dict comprehensions

Monty inlines list, set, and dict comprehensions into the surrounding code
object. The user-visible behaviour follows
[PEP 709](https://peps.python.org/pep-0709/): inlined comprehensions, no
synthetic frame in tracebacks, comprehension targets do not leak into the
enclosing scope.

A generator expression is not one of these: it is a generator, so it has a
frame of its own and a `<genexpr>` line in a traceback, as in CPython. See
[generators.md](generators.md).

## Divergences from CPython

- **A comprehension target must be a name.** CPython accepts
    `[i for obj.x in xs]` and `[i for d[k] in xs]`; Monty raises
    `SyntaxError: comprehension target must be a name, not an attribute` (or
    `not a subscript`). A comprehension's targets live in operand-stack slots,
    which a store to an object cannot reach. Attribute and subscript targets
    work in every other unpacking position, a generator expression's own
    targets included.
- **`locals()` while a comprehension is running.** CPython exposes the
    comprehension's active targets in `locals()` during the comprehension body.
    Monty does not implement `locals()` introspection.
- **Maximum number of `for` clauses.** Monty caps a single list, set or dict
    comprehension at 255 `for` clauses; exceeding this raises
    `SyntaxError: comprehension has too many nested clauses (N); maximum is 255`. Per-clause operand-stack
    growth means real comprehensions hit a tighter
    `SyntaxError: comprehension target + iterator count exceeds u8 depth operand` well before that point.
    CPython has no equivalent compile-time limit. The cap bounds compiler
    recursion depth on attacker-controlled source.
