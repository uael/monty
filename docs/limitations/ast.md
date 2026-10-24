# `ast` module

The module holds one name: `PyCF_ALLOW_TOP_LEVEL_AWAIT`, the flag
[`compile()`](eval_exec.md) takes so a body that awaits at its top level
compiles as one that may.

Everything else raises `AttributeError`.
Monty parses with ruff and exposes no syntax tree, so the node classes
(`ast.Module`, `ast.Expr`, `ast.Name`, …), `parse()`, `unparse()`, `dump()`,
`walk()`, `literal_eval()`, `NodeVisitor` and `NodeTransformer` have nothing
behind them.

The other compile flags CPython exports here are absent too:
`PyCF_ONLY_AST`, `PyCF_TYPE_COMMENTS`, `PyCF_OPTIMIZED_AST`,
and the `__future__` feature flags.
`compile()` accepts `0` and `PyCF_ALLOW_TOP_LEVEL_AWAIT` and rejects every
other value with `ValueError: compile(): unrecognised flags`.
