# `list`

## `extend` and `+=`

`xs += ys` is `xs.extend(ys)`, as in CPython: any iterable extends, the list
keeps its identity so an alias sees the update, and a non-iterable raises
`TypeError: 'int' object is not iterable` rather than `+`'s concatenation error.
`xs += xs` doubles.

- **A source that raises part-way leaves the list unchanged**, where CPython
  keeps the items it had already yielded. Both `extend` and `+=` drain the
  iterable into a temporary before appending any of it, so `xs = [0]; xs +=
  (i for i in gen_that_raises_after_one())` leaves `[0]` in Monty and `[0, 1]`
  in CPython. `collections.deque` appends as it reads and does not diverge here
  (see ./collections.md).
