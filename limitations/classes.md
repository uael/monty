# Classes

Sandboxed Python code in Monty can define simple classes. A `class`
statement with instance methods, `__init__`, `__eq__`, `__repr__`/`__str__`,
and class variables works. The class body has a real scope (like CPython's
class-body code object), so class variables may be arbitrary expressions
and may reference earlier class variables:

```python
class Foo:
    count = 0

    def __init__(self, a: int) -> None:
        self.a = a

    def bar(self) -> int:
        return self.a * 2

    def __repr__(self) -> str:
        return f'Foo(a={self.a})'
```

See `test_cases/class__basic.py` and `test_cases/class__repr.py`.

The host can also construct dataclass and namedtuple values (using the
`MontyObject` API) and pass them in; those are a separate mechanism whose
methods dispatch back to the host (see `test_cases/dataclass__basic.py`).

## Supported surface

Listed to bound what the divergences below apply to. Working,
CPython-matching features: instance methods, `__init__` (full parameter
shapes), instance and class attribute get/set (including `setattr(Foo, ...)`
and function-attributes-become-methods), bound methods, class variables
(arbitrary expressions, evaluated in a real suspendable class-body scope),
**class decorators** (`@deco class Foo`), **method decorators** taking any
callable in scope, **single inheritance** (`class B(A)`, inherited
methods/class variables/`__init__`, `super()`), **abstract bases**
(`typing.Protocol` and the `collections.abc` classes, which add no link to the
inheritance chain — see ./typing.md),
**descriptors** (`property`, `staticmethod`, `classmethod`),
`__repr__`/`__str__`/`__enter__`/`__exit__`/`__eq__`/`__hash__`/`__call__`/
`__getitem__`/`__setitem__`/`__len__`/`__bool__` dispatch,
`obj.__class__`, `Foo.__name__`, `Foo.__doc__`/`obj.__doc__`,
`Foo.__annotations__` (ordered; values stringized and provisional, see
./typing.md), `type(obj)`, `isinstance(obj, Foo)`, `issubclass(B, A)`, and the
3-arg `type()` constructor. The `__enter__`/`__exit__` divergences are in
./with.md; exception classes are in ./exceptions.md.

## Inheritance

`class B(A):` works, with one base. Attribute and method lookup walks the
chain derived-first, so an override wins and an inherited `__init__`,
`__repr__`, dunder or class variable is found; `isinstance` and `issubclass`
walk it too. Divergences:

- **Single inheritance only.** A second base raises
  `NotImplementedError: multiple inheritance is not supported` rather than
  being linearized: there is no C3 MRO, only a chain walk. `Foo.__mro__` does
  not exist.
- **A base must be a class defined in the sandbox or a builtin exception
  type.** Subclassing a builtin (`class MyList(list)`, `class C(object)`)
  raises `NotImplementedError: inheriting from '...' is not supported; a base
  must be a class defined in the sandbox or a builtin exception`. CPython
  allows both, and `object` is not even a name Monty defines.
- **Base expressions are evaluated in the enclosing scope**, as in CPython,
  but the chain is fixed at class creation: there is no `__bases__` attribute
  and no way to reassign it.
- **`super()` takes no arguments.** The explicit `super(C, obj)` form raises
  `NotImplementedError: super() with arguments is not supported`. The
  zero-argument form works, including from a middle class of a chain, but
  Monty recovers the defining class by finding which class in the receiver's
  chain binds the running function rather than from a compiler-injected
  `__class__` cell. The two agree for any class built by a `class` statement;
  they can differ if the *same* function object is bound in two classes of one
  chain (`type('B', (A,), {'m': A.m})`), where Monty resolves to the most
  derived of them.
- **`super()` outside a method** raises `RuntimeError: super(): no arguments`,
  matching CPython's wording for a missing `__class__` cell.
- **A generic base resolves to the class it subscripts.** A base that is a
  `types.GenericAlias` goes through `__mro_entries__` and stands for its
  `__origin__`, so `class Sub(Base[int])` inherits from `Base` and
  `class Held[T](Spawned[T])` from `Spawned`. Subscripting the base needs a
  `__class_getitem__` on it, which every PEP 695 generic class has; see
  ./typing.md for what a type parameter binds to.
- **The implicit root class is minimal.** `super().__init__()` falls back to
  `object.__init__` (zero arguments) or, in an exception class,
  `BaseException.__init__` (which stores `args`). No other `object` method
  (`__eq__`, `__str__`, `__reduce__`, ...) is reachable through `super()`;
  they raise `AttributeError`.

## Descriptors

`property`, `staticmethod` and `classmethod` are real objects, and class
attribute lookup invokes them. Both the decorator and the assignment form
work:

```python
class C:
    @property
    def x(self):
        return self._v

    def _set(self, value):
        self._v = value

    # `@x.setter` does not work; see below
    x = x.setter(_set)
```

Divergences:

- **`property()` takes positional arguments only.** `property(fget=f)` raises
  a `TypeError`; CPython accepts all four as keywords. The fourth argument
  (`doc`) is accepted and discarded, since there is no `property.__doc__`.
- **A property's accessors are reachable only as calls**, so `@x.setter` (and
  `@x.getter` / `@x.deleter`) raises `AttributeError: 'property' object has no
  attribute 'setter'`: reading a method off a property to hand to the decorator
  is what fails, while `x = x.setter(f)` in the class body returns the new
  property CPython's decorator form would have bound.
- **`repr(property_object)` is `<property object>`**, without CPython's
  `at 0x..` address.
- **A general user-defined descriptor protocol is not implemented.** Only
  these three built-in descriptors are invoked; a class defining `__get__` /
  `__set__` / `__delete__` and used as a class attribute is returned as-is.

## Dynamic class creation — `type(name, bases, dict)`

The 3-arg `type()` form creates classes at runtime with CPython's validation
order and error wording, but with these divergences:

- **`bases` accepts at most one entry, and it must be a sandbox class or a
  builtin exception type.** `(object,)` and `(int,)` raise
  `NotImplementedError`, and two bases raise
  `NotImplementedError: multiple inheritance is not supported`; see
  "Inheritance" above. This is the same code path a compiled `class`
  statement takes, so the two agree by construction.
- **Keywords are always rejected.** CPython forwards extra keywords to
  `__init_subclass__`; Monty has no `__init_subclass__`, but the error
  message matches what `object.__init_subclass__` produces
  (`A.__init_subclass__() takes no keyword arguments`).
- Only `__doc__` is synthesized into the namespace when absent (as `None`,
  matching CPython). CPython also sets `__module__`, `__dict__`,
  `__weakref__`, etc.; those attributes raise `AttributeError` in Monty, as for
  compiled classes. `__qualname__` is answered, but from the class name rather
  than the namespace.
- **Non-string namespace keys raise `TypeError`**
  (`non-string key (int) in the namespace of class 'A'`). CPython accepts
  them with only a `RuntimeWarning`; Monty has no warnings machinery, so it
  raises rather than silently accepting.

## Divergences from CPython

- **Default `repr`** (no user `__repr__`) is `<Foo object at 0x..>` using the
  **bare** class name, where CPython uses the qualified name
  `<module.Foo object at 0x..>`.
- **`__init__`/method argument-count errors** name the method without the
  class qualifier, e.g. `__init__() missing 1 required positional argument:
  'y'`, where CPython says `Foo.__init__() missing ...`.
- **`type(obj)`** returns the class object (so identity works), but its own
  `repr` is `<class 'Foo'>` with the bare name; CPython qualifies it.
- **`type(Foo) is type` is `False`** (CPython: `True`). The bare name `type`
  resolves to the builtin `type` *function*, not a type object, and there is no
  metaclass. `isinstance(Foo, type)` does answer `True`, as it does for every
  builtin type and exception type, since `type` as the second argument asks
  whether the first is a class rather than comparing objects.
- **Bound methods report `function`, not `method`.** `type(obj.method)` is
  `<class 'function'>` where CPython says `<class 'method'>`; Monty has no
  dedicated `method` type.
- **Ordering comparisons on instances raise, but a user `__lt__`/`__gt__`/… is
  not dispatched.** `a < b` on instances of a class with no comparison dunders
  raises `TypeError: '<' not supported between instances of 'Foo' and 'Foo'`
  (matching CPython). A class that *defines* `__lt__` etc. still raises: those
  dunders are not dispatched (see the not-dispatched dunder list below).
- **`__repr__`/`__str__` cannot suspend**: they are run to completion
  synchronously, so a `__repr__`/`__str__` that calls an external/OS function
  raises rather than yielding to the host. `__init__` and regular methods
  *can* suspend on external/OS calls.
- **Only a plain-function `__init__` can suspend.** When `__init__` is bound to
  something else (a builtin, another class, a bound method, ...), it is called
  with CPython's descriptor-binding semantics (no `self` prepended unless it is
  a plain function) and CPython's `None`-return contract is enforced, but it
  runs to completion synchronously, so it cannot yield to the host, and an
  external-function `__init__` raises `NotImplementedError` rather than
  suspending.
- **`__eq__`/`__hash__` cannot suspend**: like `__repr__`/`__str__` they run to
  completion synchronously, so one that calls an external/OS function raises
  rather than yielding to the host. An exception raised by `__eq__` terminates
  the run instead of being catchable by a `try` around the comparison.
- **`__getitem__`/`__setitem__`/`__len__`/`__bool__` and a `property`
  getter/setter cannot suspend either**, for the same reason: they run to
  completion synchronously and raise `NotImplementedError` on an external/OS
  call. `__call__` is the exception: it runs as a real pushed frame, so it can
  suspend exactly as an ordinary method does.
- **Ordering dunders are still not dispatched**; see the entry above.
- **Bound methods compare and hash by identity**: each `obj.method` access
  creates a fresh object, so `obj.method == obj.method` is `False` and two
  accesses hash differently. CPython compares/hashes bound methods by
  `(instance, func)`, making separate accesses equal.
- **Bound-method `repr`** is the bare `<bound method>`; CPython renders
  `<bound method Foo.m of <__main__.Foo object at 0x..>>`.
- **Assigning `Foo.__name__`** stores an ordinary class member. Unlike CPython,
  where `type.__name__` is a metaclass descriptor whose setter renames the
  class, it does not rename the class, so `Foo.__name__` reads and `repr(Foo)`
  keep the original name while instances see the member.
- **Assigning `obj.__class__`** stores an ordinary instance attribute rather
  than reassigning the object's class. `obj.__class__ = X` then reads back `X`,
  but `type(obj)` and `isinstance` still report the original class, leaving an
  internally inconsistent object. CPython either reassigns the class (for a
  compatible class) or raises `TypeError: __class__ must be set to a class, not
  '...' object`.
- **Recursive/deep `__repr__`/`__str__` raises `RecursionError` earlier than
  CPython.** A `__repr__` (or `__str__`) that reprs `self`, or a deep chain of
  instances whose reprs nest (e.g. a long linked list), re-enters the
  interpreter on the native Rust call stack once per nesting level, unlike
  ordinary Python-level recursion, which lives on a heap-allocated frame stack
  and is bounded at 1000 by the normal recursion limit. A native stack overflow
  would abort the process, which is fatal for the in-process/wasm API sharing
  the host process, so this native re-entry is capped independently at a much
  lower, fixed depth, raising a catchable `RecursionError` once exceeded. So
  infinite `__repr__` recursion raises `RecursionError` (matching CPython's
  outcome, though not its exact depth), but a deep-but-finite chain that
  CPython's default 1000-frame limit would still render may raise
  `RecursionError` in Monty. The same cap applies to synchronous callback
  evaluation such as `map()`, `filter()`, `sorted()`/`list.sort(key=...)`,
  `min()`/`max(key=...)`, and exotic `__init__` recursion (see the "Recursion"
  section of ./resource_limits.md).
- **Comprehensions in the class body** can see class variables, because Monty
  inlines comprehensions into the enclosing scope. In CPython a comprehension
  has its own scope that skips the class scope, so only the *leftmost iterable*
  is evaluated in class scope and the body cannot see class variables
  (`[n + offset for n in nums]` referencing a class variable `offset` raises
  `NameError` in CPython but succeeds in Monty).
- **Same-name collision is rejected, not resolved.** When an enclosing-function
  local and a class variable share a name *and* a method captures the enclosing
  one, CPython keeps the two distinct (a class-dict entry vs. a closure cell).
  Monty maps one name to a single slot and so cannot represent both; it raises
  `NotImplementedError` at compile time ("class member 'x' that shadows a
  captured variable of the same name from an enclosing scope") rather than
  miscompiling. Distinct names work fine.

## Crossing the host boundary (`pydantic_monty` / `@pydantic/monty`)

An **instance** of a user-defined class crosses as a `MontyInstance`
(`{ __monty_type__: 'Instance', ... }` in JS), carrying the class name, the
class's member names, and the instance's attributes:

```python
point = session.feed_run('class A:\n    def __init__(self):\n        self.x = 1\nA()')
point.class_name  # 'A'
point.attrs['x']  # 1
```

Passing it back into a session rebuilds it against *that session's* own class
of the same name and members, so an instance moves from one session to another
(typically one woken from the other's `dump()`) and stays usable there:
attribute access, `isinstance`, and method calls all work against the receiving
session's class object. A session that defines no class of that shape rejects
the instance rather than inventing one to hold it. Only classes bound in the
module namespace are matched; one defined inside a function is not part of the
session's vocabulary. What crosses is state, not behaviour: the methods are the
receiving session's, so two classes that share a name and member list but not
their method bodies are treated as the same class.

The **class object itself** has no host representation and surfaces as its repr
text (`"<class 'A'>"`), since a class is bound to the heap it was defined on. A
host that wants to construct instances asks the sandbox to.

## What does NOT exist for user code

- Multiple inheritance, an MRO, and `__mro__`/`__bases__`; see "Inheritance".
- Metaclasses (`class Foo(metaclass=Meta)` is rejected at parse time: "class
  metaclasses"), `__init_subclass__`, `__set_name__`, and any other
  metaclass-driven namespace customization.
- `__slots__`, and a general user descriptor protocol (`__get__` / `__set__` /
  `__delete__` on a user class); see "Descriptors" for the three built-in
  ones that do work.
- Abstract base classes (`abc.ABC`, `@abstractmethod`).
- `functools.wraps`-style metadata copying, for a method decorator or any
  other: a function exposes no attributes to copy (see ./language.md). The
  decorator itself is an ordinary call, so the member is bound to whatever it
  returns, and a plain wrapper receives `self` as its first argument like any
  other function.
- **Tracebacks from a method decorator that raises** point at the whole `class`
  statement, like class decorators below, rather than at the decorator line.
- **Classes are barely introspectable**: `__dict__`, `__bases__` and `dir()`
  are all unavailable (`cls.__name__` and `cls.__annotations__` work, the
  latter with stringized values, see ./typing.md). A class decorator
  can therefore discover fields and nothing else.
- **Tracebacks from decorator application point at the whole `class` statement**
  (a span from the first decorator through the body, with the body elided as
  `...<N lines>...`), where CPython pins the individual decorator that raised.
  Every decorator in a stack reports that same location; only the callee frame
  identifies which one raised.
- Dunder protocols other than `__init__`, `__repr__`, `__str__`,
  `__enter__`, `__exit__`, `__iter__`, `__next__`, `__contains__`, `__eq__`,
  `__hash__`, `__call__`, `__getitem__`, `__setitem__`, `__delitem__`,
  `__len__` and `__bool__`: `__new__`, `__add__`, `__ne__`, `__lt__`,
  `__getattr__`, etc. are not dispatched for user-defined instances. `__ne__` is always the negation of
  `__eq__`, as CPython derives it by default, so a custom `__ne__` is ignored.
- `__iter__` / `__next__` / `__contains__` **are** dispatched, but like
  `__repr__`/`__str__` they run synchronously, so one that calls an external or
  OS function cannot suspend and raises `NotImplementedError`. Two related
  protocols are still not dispatched, so a class relying on either is not
  iterable:
  - the legacy `__getitem__`-only fallback: CPython iterates a class defining
    `__getitem__` but not `__iter__` from index 0 until `IndexError`, while
    Monty reports it as not iterable. (`monty -t` accepts `iter(obj)` for
    such a class, so this fails only at runtime, see ./iter.md.)
  - `__reversed__`, so `reversed(obj)` on any user instance raises
    `TypeError: '{cls}' object is not reversible`. That matches CPython for a
    class defining neither `__reversed__` nor `__len__` + `__getitem__`, and
    diverges for one that does.
- `__next__` is looked up on the class only, never the instance `__dict__`, and
  a `StopIteration` raised anywhere inside it ends the iteration, including one
  that propagates out of a nested call. CPython's PEP 479 protections cover
  generators rather than a hand-written `__next__`, and Monty does not
  implement them for its own generators either (see ./iter.md).

- Attribute-access hooks are **never** dispatched: `__getattr__`,
  `__getattribute__`, `__setattr__`, `__delattr__`, and `__del__`. A missing
  attribute always raises the default `AttributeError` even when the class
  defines `__getattr__`, and attribute writes go straight to the instance
  `__dict__` unless the class binds the name to a `property`.
- Introspection attributes other than `__name__`, `__qualname__`, `__doc__`,
  `__annotations__` and `obj.__class__`: `Foo.__dict__`, `obj.__dict__`,
  `Foo.__bases__`, `Foo.__mro__`, `Foo.__module__`, and explicit
  `obj.__repr__()` / `obj.__str__()` calls when the class defines none, all
  raise `AttributeError`. `__qualname__` is always the same string as
  `__name__`: a class body may not contain a class, so nothing here is nested
  and there is nothing to qualify.
- Class-body statements other than a `def`, a simple `name [: T] = <expr>`
  variable assignment, a `type X = ...` alias, `pass`, `...`, or a docstring,
  e.g. `if`/`for`/`while`
  in the class body, or tuple/multiple assignment targets (rejected at parse
  time).
- Assignment expressions (`:=`) that bind in the class-body scope: in
  class-variable values, method parameter defaults, and lambda parameter
  defaults (rejected at parse time). In CPython the walrus target becomes a
  class member (`class C: x = (y := 5)` gives `C.y`); Monty's class-namespace
  assembly only records directly-assigned names, so the syntax is reserved
  rather than silently dropping the binding. A walrus inside a lambda *body*
  (`f = lambda: (z := 1)`) binds in the lambda's own scope and works. A walrus
  in a comprehension in the class body is also rejected (CPython rejects that
  too, but as a `SyntaxError` with different wording). A walrus in an
  *annotation* (`x: (y := int) = 5`) runs in Monty, since annotation
  expressions are captured as source text (stringized) and never evaluated, so
  the walrus never binds; CPython raises `SyntaxError`. This follows from
  annotations never being evaluated, so it would change if they ever are (see
  ./typing.md).
- `del Foo.attr` on a class object, which raises `AttributeError: 'type' object
  has no attribute '<name>' and no __dict__ for setting new attributes`.
  `del obj.attr` on an instance works; see ./language.md.

## `FrozenInstanceError`

Raised when assigning to a field of a frozen dataclass, host-supplied or a
sandbox `@dataclass(frozen=True)`, and when deleting an attribute of the
latter. Subclass of `AttributeError`, so `except AttributeError:` catches it,
as in CPython's `dataclasses` module. A host-supplied dataclass never raises it
for a deletion: `del` reaches instance attributes only, so it raises the plain
`AttributeError` there (see ./language.md).
