# References across the boundary (`MontyRef`, `MontySessionRef`)

The boundary between a host and a session carries **copies**. A value crosses
by being taken apart and rebuilt on the other side, so an object whose identity
is the point, or whose state is live, cannot cross at all: a host object dies
with a conversion error on the way in, and a session value with no copy
representation (a type object, a class, a template, a generator) arrives on the
host as its `repr` string, which can be printed and nothing else.

Two references answer that, one in each direction. Both are opaque tokens: the
holder cannot read anything out of one, and is not meant to. It hands the token
back and asks the side that owns the value.

CPython has no counterpart to either, so everything here is Monty's own
vocabulary rather than a divergence from a Python behaviour.

## A host object the sandbox holds (`MontyRef`)

```python
from pydantic_monty import Monty, MontyRef

cursor = Cursor()
held = MontyRef(cursor)              # keep this alive: see "Lifetime" below
with Monty().checkout() as session:
    session.feed_run("c.bash('ls')", inputs={'c': held})
```

Inside the sandbox `c` is a proxy. What it supports:

- **Attribute read** (`c.cwd`), performed as `getattr(obj, 'cwd')` on the host.
- **Method call** (`c.bash('ls', timeout=5)`), one round trip, not two.
- **Call** (`c(...)`), the host object's own `__call__`.
- **`with c as x:`**, both halves.
- **`await c.method(...)`** when the host method is `async`: the coroutine is
  awaited on the host and the sandbox gets a future, so other sandbox tasks
  keep running meanwhile.
- **`==`, `!=`, `hash`, `repr`**, answered locally: two proxies are equal
  exactly when they name one host object, `hash` agrees with that, and `repr`
  is `<TypeName host object>`.

What it does not support, and raises for:

- **Attribute assignment and deletion** (`c.x = 1`, `del c.x`). The opcode
  that stores an attribute has no way to suspend, so this cannot reach the
  host at all. Use a method.
- **Subscription** (`c[k]`, `c[k] = v`), for the same reason. Use a method.
- **Iteration** (`for x in c`, `iter(c)`): one suspension per step, which
  `__next__` has no room to take. Ask the host for a list.
- **`await c` on the reference itself.** The dispatch reaches the host's
  `__await__`, but what that returns is a coroutine wrapper with no boundary
  representation, so it fails on the way back (`Cannot convert
  builtins.coroutine_wrapper to Monty value`). Await a method instead.
- **Arithmetic and every other operator**: not forwarded.
- **`len(c)`, `bool(c)`**: `len` raises; `bool` is always true, the default
  for a Python object that defines neither.
- **Any dunder the sandbox names** (`c.__class__`, `c.__dict__`,
  `c.__getattr__`): `AttributeError`. This is deliberate and is what keeps a
  reference from being a way out of the sandbox: only operations the
  interpreter itself performs reach the host under those names.
- **A private attribute** (`c._x`): `AttributeError`, the same rule a host
  dataclass method call already follows.

The type inside the sandbox is `hostref` for every reference, not the host
class's name: the sandbox cannot name a host class, so a per-object type would
only make `type(a) is type(b)` answer False for two objects of one class, which
is worse than answering nothing. What a reference stands for is on the
reference: its `repr` is `<Cursor host object>`, and an attribute error names
the host type (`'Cursor' object has no attribute '_x'`). The errors raised by
the operators above come from the generic type machinery and say `'hostref'`
instead.

### Lifetime

**A reference lives exactly as long as the `MontyRef` wrapper does.** The
sandbox's copy of the token is not something Python's garbage collector can
see, so the wrapper is the only thing keeping the object registered. A
temporary is therefore a bug:

```python
session.feed_run("c.bash('ls')", inputs={'c': MontyRef(cursor)})   # wrong
```

The wrapper is collected as soon as the call returns, and the next operation on
the proxy raises `the host no longer holds the Cursor object this reference
names`. Bind it to something that outlives the session. `release()` ends it
early, and is idempotent; wrapping one object twice shares a token, and the
object stays reachable until the last wrapper goes.

### What a returned value becomes

A value the host returns from an operation crosses **by copy**, exactly as any
external function's return value does. A host method returning another
unconvertible object therefore fails, unless the host returns a `MontyRef`
wrapping it. There is no automatic wrapping: it would pin every such object for
the life of the process with nothing to release it.

## A session value the host holds (`MontySessionRef`)

Off by default. A session checked out with `cross_by_reference=True` hands back
a reference instead of a `repr` for any value with no copy representation:

```python
with Monty().checkout(cross_by_reference=True) as session:
    session.feed_run('class Chunk: pass')
    contract = session.feed_run('list[Chunk]')       # MontySessionRef
    session.probe('_t.__args__[0].__name__', bindings={'_t': contract})  # 'Chunk'
    session.release_refs(contract.id)
```

The host reads nothing out of the reference itself; it hands it back as an
input and asks the session, in the session's own language. `export_global`
(Rust) takes one by name for a value that *does* have a copy representation,
when identity rather than a copy is what the host wants.

Notes and limits:

- **Every reference must be released.** It pins its value in the session's
  heap, and nothing inside the sandbox can drop it, because the holder is
  outside. `release_refs` is idempotent for a token already released or never
  minted.
- **A token belongs to one session.** It is a place in that session's heap.
  Handing it to a different session is refused (`names an export this session
  has released or never made`), except for a session woken from the same
  session's dump, which carries the same heap and the same table.
- **A reference survives dump and load.** This is what lets a value outlive
  the turn that produced it, and lets two sessions woken from one dump both
  resolve the token, each against its own copy of the value.
- **An instance keeps crossing by copy.** `MontyObject::Instance` has a copy
  representation, so the mode does not change it, and a crossed instance
  therefore still carries no identity. Export it by name to get one.
- **The JavaScript binding refuses both references.** It keeps no table to
  resolve one in, and says so rather than handing back a string.

## Reading a session while it is suspended

`snapshot.probe(expr, bindings=...)` evaluates one expression against a session
that is suspended inside a call it made, and leaves the suspension resumable.
The host is usually deciding what to answer, and some answers depend on what
the frame asking for them contains.

- **It runs to completion.** The suspension is already the one turn in flight,
  so a second one would have nowhere to go: a name `bindings` does not supply
  raises `NameError` rather than reaching back out to the host. Idle, the same
  `probe` may still suspend, as a feed does.
- **`bindings` are scoped to the one expression.** Unlike a feed's inputs, and
  unlike a name resolved through the host, a binding does not stay in the
  namespace, where a later snippet would find it and a failed definition would
  be silently answered by it.
- **It binds nothing itself.** The same rule as `probe`: source that is not a
  single expression, or that could bind through `:=`, is refused. What the
  expression *calls* can still mutate what it reaches.
- **A probe's names claim namespace slots.** The slots are left unbound, so
  reading one raises `NameError`, but the slot count grows with the number of
  distinct names probed.

## Protocol

Both references cross the worker wire, and the crossing mode, the bindings, and
`ReleaseRefs` are protocol version 3. A version 2 child ignores what it does
not know, which for the bindings means answering the expression without them,
so the version floor is enforced rather than negotiated.

## A snippet's filename

A session names its snippets `<python-input-N>`, which is right for a REPL and
wrong for a host feeding chunks that have names of their own. `script_name=`
on a feed gives one, and the frames of everything that snippet defines carry it
from then on, so a traceback says where each function came from.

The name is used verbatim, including when an earlier snippet already used it.
The source kept for rendering tracebacks is keyed by this name, so reusing one
makes the older snippet's frames render against the newer text; keeping names
distinct is the caller's job. An empty name takes the generated one.

A session restored from a dump keeps the *session* name the dump carries; only
the per-snippet name is the caller's to set.
