# `operator` module

Monty implements `operator.attrgetter` and nothing else from `operator`.

```python
from operator import attrgetter

chunks.sort(key=attrgetter('window'), reverse=True)
```

## `attrgetter`

Matches CPython, including the parts that follow from arguments being split
eagerly:

- One argument yields the attribute itself; two or more yield a tuple in
  argument order.
- A dotted argument is a path, walked one attribute at a time
  (`attrgetter('b.c')(o)` is `o.b.c`).
- A missing attribute raises exactly what `getattr` would, naming the type that
  lacked it — so `attrgetter('b.nope')` reports `Inner`, not `Outer`.
- Because the split happens at construction, an empty path component becomes an
  empty attribute name: `attrgetter('x.')(o)` looks up `''` on `o.x`.
- The getter is reusable and holds nothing from the objects it is applied to.
- `repr` rebuilds the constructor arguments
  (`operator.attrgetter('a', 'b.c')`).

### Divergences

- **An attribute that would need to reach the host raises.** A getter can only
  return a plain value, so an attribute backed by an external or OS call raises
  `TypeError: attrgetter(): attribute is not a simple value` rather than
  suspending. `getattr()` has the same limit.
- The getter is not picklable or comparable; CPython supports neither
  meaningfully here either (`attrgetter('a') == attrgetter('a')` is `False` in
  both).

## Not implemented

Everything else: `itemgetter`, `methodcaller`, the arithmetic and comparison
functions (`add`, `sub`, `mul`, `truediv`, `lt`, `le`, `eq`, `ne`, `gt`, `ge`,
`and_`, `or_`, `xor`, `not_`, `neg`, `pos`, `abs_`, `index`), the in-place
variants (`iadd`, ...), the sequence helpers (`concat`, `contains`,
`countOf`, `indexOf`, `getitem`, `setitem`, `delitem`), `truth`, `is_`,
`is_not`, and `call`. The names are absent from the module namespace rather
than stubbed, so they fail type checking as well as raising `AttributeError`.
