# List / set / dict comprehensions

Monty inlines list, set, and dict comprehensions into the surrounding code
object. The user-visible behaviour follows
[PEP 709](https://peps.python.org/pep-0709/): inlined comprehensions, no
synthetic frame in tracebacks, comprehension targets do not leak into the
enclosing scope.

## Divergences from CPython

- **`locals()` while a comprehension is running.** CPython exposes the
  comprehension's active targets in `locals()` during the comprehension body.
  Monty does not implement `locals()` introspection.
- **Generator expressions bind walrus targets in their own scope.** A
  comprehension is inlined into the enclosing frame, so `[(y := v) for v in xs]`
  leaves `y` bound there as CPython's PEP 572 requires. A generator expression
  is compiled as its own function, so `(y := v for v in xs)` binds `y` inside
  that function and the name is unbound outside it.
- **Targets must be names.** `[x for obj.a in xs]` and `[x for d['k'] in xs]`
  raise `NotImplementedError: The monty syntax parser does not yet support
  attribute or subscript targets in a comprehension`. CPython accepts both.
  Everywhere else (assignments, `for` statements, `with ... as`) those targets
  work; a comprehension's targets live in operand-stack slots, which have
  nowhere to store through an object.
- **Maximum number of `for` clauses.** Monty caps a single comprehension at
  255 `for` clauses; exceeding this raises `SyntaxError: comprehension has
  too many nested clauses (N); maximum is 255`. Per-clause operand-stack
  growth means real comprehensions hit a tighter `SyntaxError: comprehension
  target + iterator count exceeds u8 depth operand` well before that point.
  CPython has no equivalent compile-time limit. The cap bounds compiler
  recursion depth on attacker-controlled source.
