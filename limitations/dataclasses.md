# `dataclasses` module

Native, in-sandbox `@dataclass`: sandboxed code can define its own dataclasses,
executed entirely inside the sandbox. Host-supplied dataclasses are a separate
mechanism, passed in and dispatching back to the host (see
./classes.md).

The module exposes `dataclass`, `field`, `fields`, `asdict`, `astuple`,
`replace`, `is_dataclass`, `MISSING` and `FrozenInstanceError`. The decorator
takes `init`, `repr`, `eq`, `order`, `unsafe_hash`, `frozen`, `match_args`,
`kw_only` and `slots`, in both the bare (`@dataclass`) and the called
(`@dataclass(frozen=True)`) form, and generates `__init__`, `__repr__`,
`__eq__`, the four ordering methods, `__hash__` and `__match_args__` by the same
rules CPython's do. `__post_init__` runs at the end of construction.

## Unsupported

Each raises `NotImplementedError`, marking a feature Monty has not built yet
rather than a mistake in the calling code, and raises it **at decoration time**
rather than producing a subtly wrong class. CPython accepts all of them, so the
exception type is a divergence in its own right: code catching `TypeError`
around a decoration will not catch these.

- **`InitVar[...]`** — raises `NotImplementedError: dataclass() does not yet
  support InitVar (field <name>), which would become an ordinary field`.
  Detected textually, since annotations are never evaluated: the name need not
  be imported to be rejected.
- **The `KW_ONLY` marker** (`_: KW_ONLY` in a class body) is rejected the same
  way, and for the same reason: it would otherwise become an ordinary field
  instead of making the fields after it keyword-only. `@dataclass(kw_only=True)`
  and `field(kw_only=True)` are both supported, so the marker is the only
  keyword-only spelling missing.
- **`weakref_slot=True`**: Monty has no weak references, so there is no slot to
  add and no `__weakref__` to expose. Without `slots=True` it fails first with
  CPython's own `TypeError: weakref_slot is True but slots is False`.
- **`Field._field_type`** is CPython's internal `_FIELD` / `_FIELD_CLASSVAR`
  marker. Monty has no field kinds (see the `__dataclass_fields__` divergence
  below), so reading it raises `NotImplementedError: Field._field_type is not
  yet supported, dataclasses._FIELD is not implemented`.
- **`make_dataclass`, `KW_ONLY`, `InitVar` and `Field` are not module
  attributes**, so importing or reading them raises `ImportError` /
  `AttributeError`.

Mutable defaults are rejected as CPython rejects them
(`ValueError: mutable default <class 'list'> for field xs is not allowed: use
default_factory`), whether written plainly or through `field(default=...)`, and
so is a non-default field after a defaulted one
(`TypeError: non-default argument 'b' follows default argument 'a'`).

## Divergences from CPython

- **Annotations are stringized.** Fields come from the class's
  `__annotations__`, which Monty stores as never-evaluated source text (always
  PEP 563); see ./typing.md. Field
  discovery and the generated methods are unaffected, the field *type* being
  inert metadata, but `C.__dataclass_fields__['x'].type` is the string `'int'`,
  not the `int` type object.
- **`slots=True` restricts assignment, it does not change the object.** CPython
  builds a *new* class with a real `__slots__`; Monty keeps the decorated class
  and refuses assignment to any name that is not a declared field, with
  CPython's message (`AttributeError: 'C' object has no attribute 'q' and no
  __dict__ for setting new attributes`). So `C` is still the class the body
  defined (CPython's is a replacement), and there is no memory saving. Neither
  interpreter exposes `__dict__` on an instance, so that difference is not
  observable. One message differs: assigning an *undeclared* name on a
  `frozen=True, slots=True` instance raises `FrozenInstanceError` here, where
  CPython 3.14 raises a `TypeError` from the recreated class's `__setattr__`.
- **`__dataclass_params__` is a tuple of flags**, in the order `(init, repr, eq,
  order, unsafe_hash, frozen, match_args, kw_only, slots, weakref_slot)`, where
  CPython stores a `_DataclassParams` object with those names as attributes.
  Both are read back by the generated methods; only the shape differs.
  Rebinding it, like rebinding `__dataclass_fields__`, changes what those
  methods do.
- **`Field.metadata` is a plain dict**, not a `types.MappingProxyType` wrapping
  one, so it is mutable and a field with no metadata hands back a fresh empty
  dict on each read rather than one shared proxy.
- **No object addresses.** `repr(MISSING)` is
  `<dataclasses._MISSING_TYPE object>` and `Field.__repr__` writes that same
  spelling, where CPython appends ` at 0x...`. `type(MISSING).__name__` matches
  CPython (`dataclasses._MISSING_TYPE`), and `MISSING` is a singleton, so
  `f.default is MISSING` works. Crossing to the host it is typed as the generic
  marker type (`typing._SpecialForm`), which is the nearest thing the host-side
  type mirror has.
- **`__dataclass_fields__` holds only real fields.** CPython keeps `ClassVar`
  (and `InitVar`) entries in the mapping, marked `_FIELD_CLASSVAR`, and filters
  them in `fields()`. Monty has no field kinds, so the mapping *is* the field
  list and class variables never appear in it.
- **Overwriting `__dataclass_fields__` un-marks the class.** Every dunder reads
  the mapping from the class namespace, so `C.__dataclass_fields__ = 5` makes
  `is_dataclass(C)` false and `C(...)` construct like a plain class. CPython
  keeps its generated methods and still calls `C` a dataclass.
- **`ClassVar` / `InitVar` / `KW_ONLY` detection is purely textual.** Monty
  matches the annotation text (bare, dotted, subscripted, or quoted) without
  checking that the name is actually imported, where CPython resolves a *string*
  annotation through the defining module's namespace. So `c: "ClassVar[int]"`
  without `ClassVar` in scope is excluded by Monty but is an ordinary field to
  CPython. Conversely any dotted spelling matches, so a same-named attribute on
  an unrelated module (`mymod.ClassVar`) is treated as `typing.ClassVar`.
- **`__post_init__`, `default_factory` and `replace()` run to completion.** They
  are dispatched synchronously, like every other hook Monty calls on the
  interpreter's own stack (`__repr__`, `__eq__`, `__hash__`), so one that calls
  an external or OS function raises `NotImplementedError` naming that call
  instead of suspending to the host. A plain-function `__init__` on an ordinary
  class *can* suspend, so this is a bound the synthesized construction adds.
  `__post_init__`'s return value is discarded, as CPython discards it.
- **`asdict` / `astuple` share leaf values instead of deep-copying them.**
  CPython passes anything that is not a dataclass, list, tuple or dict through
  `copy.deepcopy`; Monty has no `copy` module, so the result holds the same
  object. Lists, tuples and dicts are rebuilt around their converted items, as
  in CPython, so mutating the result's containers does not touch the instance.
  A `namedtuple` is one of the values passed through: CPython rebuilds it from
  converted items, so a dataclass nested inside one is converted there and not
  here.
- **A field holding a function or bound method reprs differently**, since
  Monty's own `repr` for those differs (see ./classes.md). Only the
  text differs; the value and its equality match CPython.
- **A class-body `__setattr__` never runs for the synthesized `__init__`**,
  which writes fields straight into the instance `__dict__`. This is the
  never-dispatched attribute hook described in ./classes.md rather than
  something dataclass-specific, so `@dataclass` does not reject it. The
  `frozen` and `slots` refusals are enforced by the interpreter, not by a
  generated `__setattr__`, so they hold regardless.
- **`@dataclass` on a non-class** (e.g. `dataclasses.dataclass(5)`) raises
  `TypeError: dataclass() should be called on a class, not '<type>'`. CPython
  instead raises an incidental `AttributeError` about `__module__` from its
  implementation. The `@deco` syntax only ever targets a class, so this affects
  only direct calls.
- **`dataclass(...)` returns a native callable, not a Python function.** CPython
  builds a closure, which Monty cannot: a native function has nowhere to keep
  the bound options but its own value. Applying it to a class is identical, and
  it reprs as `<function dataclass at 0x..>`, but `type()` says
  `builtin_function_or_method` where CPython says `function`, and CPython's repr
  names the closure (`dataclass.<locals>.wrap`). Having nowhere to live but the
  value, the options *are* the value: `dataclass(frozen=True) is
  dataclass(frozen=True)` is `True`, where each CPython call builds a fresh
  closure. Fixable only if Monty gains closures over native functions; nothing
  else depends on that, so it is not planned.
- **`del obj.field` on a frozen instance never raises `cannot delete field`**,
  because Monty's parser has no `del` statement at all. (Assignment matches
  CPython, message included, and `dataclasses.FrozenInstanceError` is
  importable.)
- **Re-decorating a dataclass rebuilds it.** `C = dataclass(frozen=True)(C)`
  gives Monty a fully frozen class, where CPython keeps the `__init__` its first
  decoration generated — one that writes fields through the *new* frozen
  `__setattr__`, so CPython's re-decorated class raises `FrozenInstanceError`
  the moment you construct it. Monty synthesizes from the current metadata, so
  it constructs normally.
- **`__dataclass_params__` reads back normalised.** `C.__dataclass_params__`
  exists, reprs like CPython's and answers all ten flags, but each is the `bool`
  Monty acted on: `@dataclass(frozen=1)` reports `frozen=True` where CPython
  echoes the `1` you passed. As in CPython the object only reports the options —
  the class acts on what it was decorated with — so assigning another one
  changes what you read back and nothing else.

## Architectural gaps (cannot match)

- **Fields are not inherited across two decorated classes.** `@dataclass class
  B(A)` collects only `B`'s own annotations, where CPython merges `A`'s fields
  in front of them, so `fields()` and the synthesized `__init__` see only `B`'s.
  A consequence: a frozen dataclass inheriting from a non-frozen one (or vice
  versa) is accepted where CPython raises.

  An **undecorated** subclass is a different case and does work: `class B(A)`
  with `A` a dataclass inherits `A`'s fields, so it constructs, compares,
  prints, hashes and refuses frozen assignment exactly as `A` does, and
  `is_dataclass(B)` is true — which is what CPython's inherited `__init__`
  gives. `repr` names the subclass (`B(x=1)`), as CPython's does.
- **No `__weakref__`**, so `weakref_slot=True` is refused rather than modelled.
