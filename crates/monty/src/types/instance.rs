use std::{borrow::Cow, fmt::Write};

use super::{Dict, LazyHeapSet, PyTrait, Type, attribute_name_value};
use crate::{
    args::{ArgValues, KwargsValues},
    builtins::Builtins,
    bytecode::{CallResult, VM},
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunError, RunResult},
    hash::{HashValue, identity_hash},
    heap::{
        BorrowedHeapReadMut, DropGuard, DropWithContext, Heap, HeapData, HeapId, HeapItem, HeapRead,
        heap_read_ref_as_field_mut,
    },
    intern::Interns,
    modules::dataclasses::{self, DataclassHash},
    types::{
        allocate_string,
        class::{MAX_MRO_DEPTH, class_base_id, class_exc_base},
        native_class::native_default_member,
        property::MethodKind,
    },
    value::{EitherStr, Value},
};

/// An instance of a user-defined class.
///
/// Holds a reference to its [`Class`](super::Class) (whose `HeapId` is the type
/// identity used by `type()`/`isinstance`) and an `attrs` [`Dict`] — the instance
/// `__dict__`. Attribute reads fall through to the class namespace, and its
/// bases', for methods and class variables; writes touch `attrs` unless the
/// class binds the name to a `property`, whose setter runs instead.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Instance {
    /// The class this is an instance of (a `HeapData::Class`).
    class: HeapId,
    /// Instance attributes (`__dict__`).
    attrs: Dict,
}

impl Instance {
    /// Creates a new instance of `class` with the given initial attributes.
    #[must_use]
    pub fn new(class: HeapId, attrs: Dict) -> Self {
        Self { class, attrs }
    }

    /// Returns the `HeapId` of the instance's class object.
    #[must_use]
    pub fn class(&self) -> HeapId {
        self.class
    }

    /// Returns a reference to the instance's attribute dict (`__dict__`).
    #[must_use]
    pub fn attrs(&self) -> &Dict {
        &self.attrs
    }
}

/// A method bound to an instance, produced by `obj.method` (without calling it).
///
/// Calling a `BoundMethod` prepends `instance` to the argument list and invokes
/// `func`. The common `obj.method()` path skips this allocation by binding and
/// calling directly in [`Instance::py_call_attr`].
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct BoundMethod {
    /// The bound `self` (a `Value::Ref` to the instance).
    pub instance: Value,
    /// The underlying function (`DefFunction`/`Closure`/...).
    pub func: Value,
}

impl<'h> HeapRead<'h, Instance> {
    fn attrs_mut(&mut self) -> BorrowedHeapReadMut<'_, 'h, Dict> {
        heap_read_ref_as_field_mut!(self, Instance, attrs)
    }

    /// Sets an instance attribute, returning the previous value (if any) for the
    /// caller to drop. Takes ownership of both `name` and `value`.
    ///
    /// The one hook a `@dataclass(frozen=True)` needs on the write path; what
    /// counts as frozen and how it reads is [`dataclasses`]' business.
    pub fn set_attr(&mut self, name: Value, value: Value, vm: &mut VM<'h>) -> RunResult<Option<Value>> {
        let class_id = self.get(vm.heap).class();
        if let Some(exc) = dataclasses::set_attr_error(class_id, &name, vm) {
            [name, value].drop_with(vm);
            return Err(exc);
        }
        self.attrs_mut().set(name, value, vm)
    }

    /// Sets an attribute without the frozen check, for the synthesized
    /// `__init__` — which has to populate a `frozen=True` instance that its own
    /// `set_attr` would refuse, exactly as CPython's generated `__init__` goes
    /// through `object.__setattr__`.
    pub fn set_attr_unchecked(&mut self, name: Value, value: Value, vm: &mut VM<'h>) -> RunResult<Option<Value>> {
        self.attrs_mut().set(name, value, vm)
    }

    /// Removes an instance attribute, returning its key and value for the caller
    /// to drop, or `None` when the instance `__dict__` does not bind `name`.
    pub fn del_attr(&mut self, name: &Value, vm: &mut VM<'h>) -> RunResult<Option<(Value, Value)>> {
        self.attrs_mut().pop(name, vm)
    }
}

impl<'h> PyTrait<'h> for HeapRead<'h, Instance> {
    /// The class's `__contains__`, or `None` when it defines none — `in` then
    /// falls back to iteration, matching CPython's `sq_contains` before `tp_iter`.
    fn py_contains_impl(&self, self_id: HeapId, item: &Value, vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        instance_contains(self_id, item, vm)
    }

    fn py_type(&self, vm: &VM<'h>) -> Type {
        Type::Instance(self.get(vm.heap).class)
    }

    fn py_len(&self, _vm: &VM<'h>) -> Option<usize> {
        None
    }

    /// Writes straight to the instance `__dict__`.
    ///
    /// A `property` setter must run instead when the class defines one, which
    /// needs the instance's `HeapId`; `Value::py_set_attr` routes instances
    /// through [`instance_setattr`] for that, so this is only the floor under a
    /// heap-level write reached without a `Value`.
    fn py_set_attr(&mut self, name: &EitherStr, value: Value, vm: &mut VM<'h>) -> RunResult<()> {
        let mut value_guard = DropGuard::new(value, vm);
        // `set_attr` below is what refuses a `frozen` or `slots` write, and it
        // releases both halves when it does.
        let name = attribute_name_value(name, value_guard.ctx());
        let (value, vm) = value_guard.into_parts();
        let old_value = self.set_attr(name, value, vm)?;
        old_value.drop_with(vm);
        Ok(())
    }

    /// `del obj.attr` removes the instance attribute only. A class-level binding
    /// of the same name is not touched (and becomes visible again), matching
    /// CPython; deleting a name only the class binds raises `AttributeError`.
    fn py_del_attr(&mut self, name: &EitherStr, vm: &mut VM<'h>) -> RunResult<()> {
        let key = attribute_name_value(name, vm);
        defer_drop!(key, vm);
        if let Some((old_key, old_value)) = self.del_attr(key, vm)? {
            old_key.drop_with(vm);
            old_value.drop_with(vm);
            Ok(())
        } else {
            let class_id = self.get(vm.heap).class;
            Err(ExcType::attribute_error(
                class_name(class_id, vm.heap, vm.interns),
                name.as_str(vm.interns),
            ))
        }
    }

    /// Returns `NotImplemented`; comparisons dispatch at the `Value` level because
    /// user and synthesized dataclass equality require the instance's `HeapId`.
    fn py_eq_impl(&self, _other: &Value, _vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        Ok(None)
    }

    /// Hashes an instance, following CPython's precedence.
    ///
    /// A `@dataclass` decoration generating a `__hash__` wins, since CPython
    /// writes it into the class after the body has run. Otherwise the body
    /// decides: a `__hash__` is used, `__hash__ = None` is the unhashable
    /// opt-out, and defining `__eq__` is the same opt-out implicitly. What is
    /// left hashes by identity.
    fn py_hash(&self, self_id: HeapId, vm: &mut VM<'h>) -> RunResult<Option<HashValue>> {
        let class_id = self.get(vm.heap).class();
        match dataclasses::hash_action(class_id, vm) {
            // Takes the class rather than a field list: `dataclass_hash` selects
            // the fields itself, honouring each one's `hash`/`compare` flag.
            Some(DataclassHash::FieldWise) => dataclasses::dataclass_hash(self_id, class_id, vm),
            Some(DataclassHash::Unhashable) => Ok(None),
            None if class_defines(class_id, "__hash__", vm) => {
                if class_defines_not_none(class_id, "__hash__", vm) {
                    instance_user_hash(self_id, vm)
                } else {
                    Ok(None)
                }
            }
            // CPython's `type` sets `__hash__ = None` whenever `__eq__` is defined.
            None if class_defines(class_id, "__eq__", vm) => Ok(None),
            None => Ok(Some(identity_hash(self_id))),
        }
    }

    /// The best-effort `<ClassName object>` default, never the real `repr`.
    ///
    /// Every form that distinguishes instances needs the `HeapId` to pass `self`,
    /// so all of it lives in [`instance_repr_fmt`], which `Value::py_repr_fmt`
    /// routes every instance through; this is only the floor under a heap-level
    /// `repr` reached without a `Value`.
    fn py_repr_fmt(&self, f: &mut impl Write, vm: &mut VM<'h>, _heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        let class_id = self.get(vm.heap).class();
        Ok(write!(f, "<{} object>", class_name(class_id, vm.heap, vm.interns))?)
    }

    fn py_call_attr(
        &mut self,
        self_id: HeapId,
        vm: &mut VM<'h>,
        attr: &EitherStr,
        args: ArgValues,
    ) -> RunResult<CallResult> {
        let attr_str = attr.as_str(vm.interns);

        // 1. An instance attribute shadows class methods; call it as-is (unbound).
        if let Some(callable) = self
            .get(vm.heap)
            .attrs
            .get_by_str(attr_str, vm.heap, vm.interns)
            .map(|v| v.clone_with_heap(vm.heap))
        {
            defer_drop!(callable, vm);
            return vm.call_function(callable, args);
        }

        // 2. A class member (this class's or an inherited one): bind `self` for
        // methods, run a `property` and call its result, call the rest as-is.
        let class_id = self.get(vm.heap).class;
        if let Some(member) = class_member(class_id, attr_str, vm) {
            if user_property(&member, vm).is_some() {
                let value = match descriptor_instance_get(member, attr_str, self_id, class_id, vm) {
                    Ok(value) => value,
                    Err(e) => {
                        args.drop_with(vm);
                        return Err(e);
                    }
                };
                defer_drop!(value, vm);
                return vm.call_function(value, args);
            }
            defer_drop!(member, vm);
            return call_member_bound(member, self_id, class_id, args, vm);
        }

        // 3. `obj.__class__(...)` constructs a new instance — the callable form of
        // the `obj.__class__` special attribute (see `instance_getattr` step 3).
        // Checked after the dict/namespace lookups so a same-named member wins.
        // The class value is a fresh owned ref (inc_ref) dropped by the guard once
        // `call_function` has borrowed it; `instantiate_class` takes its own ref
        // for the new instance.
        if attr_str == "__class__" {
            vm.heap.inc_ref(class_id);
            let class_val = Value::Ref(class_id);
            defer_drop!(class_val, vm);
            return vm.call_function(class_val, args);
        }

        // 4. No such attribute.
        args.drop_with(vm);
        Err(ExcType::attribute_error(
            class_name(class_id, vm.heap, vm.interns),
            attr_str,
        ))
    }

    fn py_is_iterable(&self, vm: &VM<'h>) -> bool {
        // CPython also accepts a `__getitem__`-only class here; Monty has no
        // such fallback, so it reports not-iterable (see limitations/classes.md).
        class_defines_not_none(self.get(vm.heap).class, "__iter__", vm)
    }

    fn py_is_iterator(&self, vm: &VM<'h>) -> bool {
        // Plain existence, not [`class_defines_not_none`]: `__next__ = None` is
        // not an opt-out in CPython — the class stays an iterator and calling it
        // raises "'NoneType' object is not callable".
        class_defines(self.get(vm.heap).class, "__next__", vm)
    }

    /// Dispatches the class's `__iter__` and returns its result unchanged.
    ///
    /// No wrapper is interposed, so a self-iterator satisfies `iter(obj) is obj`
    /// exactly as in CPython. The result must itself be an iterator, mirroring
    /// `PyObject_GetIter`'s `PyIter_Check` — raising here rather than deferring
    /// the failure to the first `next()`.
    fn py_iter(&self, self_id: Option<HeapId>, vm: &mut VM<'h>) -> RunResult<Value> {
        let self_id = self_id.expect("heap values have an id");
        // `py_is_iterable` is the single source of truth for "does this class
        // iterate", so an opted-out `__iter__ = None` reports not-iterable here
        // instead of reaching the call and raising "'NoneType' object is not
        // callable" — CPython's `slot_tp_iter` rejects it the same way.
        let dispatched = if self.py_is_iterable(vm) {
            instance_call_dunder_sync(self_id, "__iter__", ArgValues::Empty, vm)?
        } else {
            None
        };
        let Some(iterator) = dispatched else {
            return Err(ExcType::type_error_not_iterable(&class_name(
                instance_class(self_id, vm),
                vm.heap,
                vm.interns,
            )));
        };
        if iterator.py_is_iterator(vm) {
            Ok(iterator)
        } else {
            let err = ExcType::type_error_iter_returned_non_iterator(&iterator.py_type_name(vm));
            iterator.drop_with(vm);
            Err(err)
        }
    }

    /// Dispatches the class's `__next__`, turning `StopIteration` into exhaustion.
    ///
    /// Every other exception propagates, including an `UncatchableExc` from a
    /// resource limit — see [`RunError::is_stop_iteration`]. No heap borrow is
    /// held across the call: `__next__` runs Python, which may re-enter this
    /// same instance through a nested `next()`.
    fn py_next(&mut self, self_id: Option<HeapId>, vm: &mut VM<'h>) -> RunResult<Option<Value>> {
        let self_id = self_id.expect("heap values have an id");
        // The absent case doubles as the not-an-iterator check, halving the
        // lookups — this runs for every item of every user iterator.
        match instance_call_dunder_sync(self_id, "__next__", ArgValues::Empty, vm) {
            Ok(Some(value)) => Ok(Some(value)),
            Ok(None) => Err(ExcType::type_error_not_iterator(&class_name(
                instance_class(self_id, vm),
                vm.heap,
                vm.interns,
            ))),
            Err(e) if e.is_stop_iteration() => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn py_is_context_manager(&self, vm: &VM<'h>) -> bool {
        // CPython names `__exit__` in the protocol TypeError, so that is the
        // dunder the `BeforeWith` gate checks; a class with `__exit__` but no
        // `__enter__` passes the gate and gets the "missed __enter__ method"
        // error from `py_enter` instead — matching CPython's check order.
        class_defines(self.get(vm.heap).class, "__exit__", vm)
    }

    fn py_enter(&mut self, self_id: HeapId, vm: &mut VM<'h>) -> RunResult<CallResult> {
        let class_id = self.get(vm.heap).class;
        let Some(enter) = class_member(class_id, "__enter__", vm) else {
            return Err(ExcType::type_error_not_context_manager(
                class_name(class_id, vm.heap, vm.interns),
                "__enter__",
            ));
        };
        defer_drop!(enter, vm);
        // A plain-function `__enter__` runs as a real pushed frame
        // (`CallResult::FramePushed`), so — unlike `__repr__`/`__str__` — it
        // can suspend on external/OS calls; the frame's return value becomes
        // the `as` target via the normal `ReturnValue` push.
        call_member_bound(enter, self_id, class_id, ArgValues::Empty, vm)
    }

    fn py_exit(&mut self, self_id: HeapId, vm: &mut VM<'h>, exc: Option<HeapId>) -> RunResult<CallResult> {
        let class_id = self.get(vm.heap).class;
        let Some(exit) = class_member(class_id, "__exit__", vm) else {
            // Defensive tripwire — unreachable via `with`. `py_is_context_manager`
            // gates on `__exit__` being present at entry, and Monty has no `del`,
            // so the member cannot be removed mid-body. A reassignment (e.g. to a
            // non-callable like `None`) keeps the member present, so this branch
            // is not taken — that case fails later in `call_member_bound` as a
            // `TypeError: 'NoneType' object is not callable`.
            return Err(ExcType::attribute_error(
                class_name(class_id, vm.heap, vm.interns),
                "__exit__",
            ));
        };
        defer_drop!(exit, vm);
        // Build CPython's `(exc_type, exc_value, traceback)` triple. The type
        // is constructed as `Builtins::ExcType` — the same value the bare
        // exception name resolves to — so the idiomatic `if typ is ValueError:`
        // works inside a user `__exit__`. Monty has no traceback objects, so
        // the third slot is always `None` (see limitations/with.md).
        let (typ, val) = match exc {
            Some(exc_id) => {
                // A sandbox-defined exception hands its own class object as the
                // type, so `if typ is MyError:` works inside `__exit__` the same
                // way `if typ is ValueError:` does for a builtin one.
                let typ = match vm.heap.get(exc_id) {
                    HeapData::Exception(e) => Value::Builtin(Builtins::ExcType(e.exc_type())),
                    HeapData::Instance(inst) => {
                        let class_id = inst.class;
                        vm.heap.inc_ref(class_id);
                        Value::Ref(class_id)
                    }
                    // Instances only receive `Some(exc)` from `WithExceptStart`,
                    // which always passes the in-flight exception object
                    // (explicit `obj.__exit__(...)` calls go through normal
                    // method dispatch, never this trait hook).
                    _ => unreachable!("Instance py_exit called with a non-exception heap id"),
                };
                vm.heap.inc_ref(exc_id);
                (typ, Value::Ref(exc_id))
            }
            None => (Value::None, Value::None),
        };
        let args = ArgValues::ArgsKargs {
            args: vec![typ, val, Value::None],
            kwargs: KwargsValues::Empty,
        };
        call_member_bound(exit, self_id, class_id, args, vm)
    }
}

impl HeapItem for Instance {
    fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        stack.push(self.class);
        self.attrs.py_dec_ref_ids(stack);
    }
}

impl<'h> PyTrait<'h> for HeapRead<'h, BoundMethod> {
    fn py_type(&self, _vm: &VM<'h>) -> Type {
        // Monty has no dedicated `method` type; bound methods report `function`.
        Type::Function
    }

    fn py_len(&self, _vm: &VM<'h>) -> Option<usize> {
        None
    }

    fn py_eq_impl(&self, _other: &Value, _vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        Ok(None)
    }

    fn py_hash(&self, self_id: HeapId, _vm: &mut VM<'h>) -> RunResult<Option<HashValue>> {
        // Bound methods hash by identity, consistent with their identity-only
        // equality (CPython hashes by `(instance, func)` — see limitations/classes.md).
        Ok(Some(identity_hash(self_id)))
    }

    fn py_repr_fmt(&self, f: &mut impl Write, _vm: &mut VM<'h>, _heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        Ok(write!(f, "<bound method>")?)
    }
}

impl HeapItem for BoundMethod {
    fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        self.instance.py_dec_ref_ids(stack);
        self.func.py_dec_ref_ids(stack);
    }
}

/// Reads an instance attribute for `obj.attr` (the `LoadAttr` path).
///
/// Mirrors Python's lookup order: the instance `__dict__` first, then the class
/// namespace, then the `__class__` special case. A class method becomes a
/// [`BoundMethod`] (binding `self`); a class variable is returned as-is. A missing
/// attribute raises `AttributeError` with the real class name. Takes `self_id`
/// (available at the `Value` level) because binding a method needs the instance's
/// `HeapId`.
pub(crate) fn instance_getattr(self_id: HeapId, attr: &EitherStr, vm: &mut VM<'_>) -> RunResult<CallResult> {
    let attr_str = attr.as_str(vm.interns);
    if let Some(value) = instance_attr(self_id, attr_str, vm)? {
        Ok(CallResult::Value(value))
    } else {
        let class_id = instance_class(self_id, vm);
        Err(ExcType::attribute_error(
            class_name(class_id, vm.heap, vm.interns),
            attr_str,
        ))
    }
}

/// The lookup half of [`instance_getattr`]: the instance `__dict__`, then the
/// class namespace, then the `__class__` special case; `None` when nothing binds
/// `attr`, leaving the `AttributeError` to the caller.
///
/// Split out so the synthesized dataclass `__repr__`/`__eq__` read their fields
/// exactly as `self.field` does, binding a function-valued class member as a
/// [`BoundMethod`].
pub(crate) fn instance_attr(self_id: HeapId, attr: &str, vm: &mut VM<'_>) -> RunResult<Option<Value>> {
    let class_id = instance_class(self_id, vm);
    // A data descriptor (a `property`) wins over the instance `__dict__`, as in
    // CPython's `_PyObject_GenericGetAttrWithDict`; non-data descriptors and
    // plain class variables lose to it.
    let class_member = class_member(class_id, attr, vm);
    let is_data_descriptor = class_member.as_ref().is_some_and(|m| user_property(m, vm).is_some());
    let from_dict = if is_data_descriptor {
        None
    } else {
        match vm.heap.get(self_id) {
            HeapData::Instance(inst) => inst
                .attrs
                .get_by_str(attr, vm.heap, vm.interns)
                .map(|v| v.clone_with_heap(vm.heap)),
            _ => None,
        }
    };
    if let Some(value) = from_dict {
        class_member.drop_with(vm);
        return Ok(Some(value));
    }
    match class_member {
        Some(member) => descriptor_instance_get(member, attr, self_id, class_id, vm).map(Some),
        // `obj.__class__` returns the class object itself (`obj.__class__ is Foo`).
        // Last, so an explicit member of the same name wins, mirroring the
        // `__name__` handling on class objects.
        None if attr == "__class__" => {
            vm.heap.inc_ref(class_id);
            Ok(Some(Value::Ref(class_id)))
        }
        // An exception instance reports the chaining slots as `None` until a
        // `raise ... from ...` or an implicit chain fills them in, matching
        // CPython, where they are always-present `BaseException` members.
        None if matches!(attr, "__cause__" | "__context__") && instance_exc_base(self_id, vm).is_some() => {
            Ok(Some(Value::None))
        }
        None => Ok(None),
    }
}

/// Writes `obj.attr = value`, running a class `property` setter when one is
/// defined and otherwise storing into the instance `__dict__`.
///
/// The `Value`-level counterpart of [`HeapRead::py_set_attr`], which cannot see
/// the class chain because it has no `HeapId`. Takes ownership of `value` on
/// every path.
pub(crate) fn instance_setattr(self_id: HeapId, name: &EitherStr, value: Value, vm: &mut VM<'_>) -> RunResult<()> {
    let attr = name.as_str(vm.interns);
    let Some(value) = descriptor_instance_set(self_id, attr, value, vm)? else {
        return Ok(());
    };
    vm.heap.read(self_id).py_set_attr(name, value, vm)
}

/// Resolves `attr` for `super().attr`, starting *after* `start_class` in the
/// receiver's base chain and binding the result to `self_id`.
///
/// `Ok(None)` means no remaining class defines it, which the `super()` proxy
/// turns into either the implicit root-class behaviour or an `AttributeError`.
pub(crate) fn instance_super_lookup(
    self_id: HeapId,
    start_class: HeapId,
    attr: &str,
    vm: &mut VM<'_>,
) -> RunResult<Option<Value>> {
    let Some(base_id) = class_base_id(start_class, vm) else {
        return Ok(None);
    };
    let Some(member) = class_member(base_id, attr, vm) else {
        return Ok(None);
    };
    // The owning class for a `classmethod` is the receiver's own class, as in
    // CPython, where `super().cm()` still passes the most derived class.
    let class_id = instance_class(self_id, vm);
    descriptor_instance_get(member, attr, self_id, class_id, vm).map(Some)
}

/// The nearest builtin exception ancestor of `self_id`'s class, or `None` when
/// `self_id` is not an instance of an exception class.
///
/// This is what makes a user instance raisable: `raise MyError(...)` is legal
/// exactly when its class chain reaches `BaseException`.
pub(crate) fn instance_exc_base(self_id: HeapId, vm: &VM<'_>) -> Option<ExcType> {
    match vm.heap.get(self_id) {
        HeapData::Instance(inst) => class_exc_base(inst.class, vm),
        _ => None,
    }
}

/// Produces `repr(instance)`, dispatching to a user `__repr__` if the class
/// defines one, otherwise the default `<ClassName object at 0x..>`.
pub(crate) fn instance_repr(self_id: HeapId, vm: &mut VM<'_>) -> RunResult<Value> {
    // Top of a repr, so the cycle set starts empty.
    let mut s = String::new();
    let mut heap_ids = LazyHeapSet::default();
    instance_repr_fmt(self_id, &mut s, vm, &mut heap_ids)?;
    Ok(allocate_string(s, vm.heap))
}

/// Writes an instance's `repr` into `f`: a user `__repr__` wins, then the
/// synthesized dataclass form, then the `<Foo object at 0x..>` default.
///
/// The dataclass form registers the instance in the *caller's* `heap_ids` for
/// its duration, so a field pointing back renders `...` rather than nesting.
pub(crate) fn instance_repr_fmt(
    self_id: HeapId,
    f: &mut impl Write,
    vm: &mut VM<'_>,
    heap_ids: &mut LazyHeapSet,
) -> RunResult<()> {
    if let Some(s) = instance_call_str_dunder(self_id, "__repr__", vm)? {
        defer_drop!(s, vm);
        return Ok(f.write_str(s.to_str(vm)?)?);
    }
    let class_id = instance_class(self_id, vm);
    // `BaseException.__repr__` renders the class name over the args tuple, so
    // an exception subclass with no `__repr__` shows `Halt('stopped')` rather
    // than the `<Halt object at 0x..>` default.
    if class_exc_base(class_id, vm).is_some() {
        let name = class_name(class_id, vm.heap, vm.interns).into_owned();
        let args = instance_args(self_id, vm);
        // Written inside the guarded region: the sink can refuse (the
        // assert-repr writer stops at its byte cap), and an early `?` here
        // would strand the cloned arguments.
        let result = f
            .write_str(&name)
            .map_err(RunError::from)
            .and_then(|()| write_call_repr(&args, f, vm, heap_ids));
        for arg in args {
            arg.drop_with(vm);
        }
        return result;
    }
    heap_ids.insert(self_id);
    let handled = dataclasses::dataclass_repr_fmt(self_id, class_id, f, vm, heap_ids);
    heap_ids.remove(&self_id);
    if handled? {
        return Ok(());
    }
    Ok(f.write_str(&default_repr(self_id, vm))?)
}

/// Produces `str(instance)`, dispatching to a user `__str__` if defined, else
/// falling back to `repr` (which itself falls back to the default).
pub(crate) fn instance_str(self_id: HeapId, vm: &mut VM<'_>) -> RunResult<Value> {
    if let Some(s) = instance_call_str_dunder(self_id, "__str__", vm)? {
        return Ok(s);
    }
    // `BaseException.__str__` is args-based, not repr-based: no args is the
    // empty string, one is that argument, several are the args tuple's repr.
    if instance_exc_base(self_id, vm).is_some() {
        let args = instance_args(self_id, vm);
        let text = exception_instance_str(&args, vm);
        for arg in args {
            arg.drop_with(vm);
        }
        return Ok(allocate_string(text?, vm.heap));
    }
    instance_repr(self_id, vm)
}

/// `str(e)` for an exception instance, over its `args`.
fn exception_instance_str(args: &[Value], vm: &mut VM<'_>) -> RunResult<String> {
    match args {
        [] => Ok(String::new()),
        [only] => {
            let text = only.py_str(vm)?;
            defer_drop!(text, vm);
            Ok(text.to_str(vm)?.to_owned())
        }
        many => {
            let mut s = String::new();
            let mut heap_ids = LazyHeapSet::default();
            write_call_repr(many, &mut s, vm, &mut heap_ids)?;
            Ok(s)
        }
    }
}

/// Writes `(a, b)`: the argument list of an exception's `repr`, and the whole
/// `str` of a multi-argument one.
fn write_call_repr(args: &[Value], f: &mut impl Write, vm: &mut VM<'_>, heap_ids: &mut LazyHeapSet) -> RunResult<()> {
    f.write_char('(')?;
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            f.write_str(", ")?;
        }
        arg.py_repr_fmt(f, vm, heap_ids)?;
    }
    Ok(f.write_char(')')?)
}

/// Evaluates `item in instance` through the class's `__contains__`.
///
/// `Ok(None)` means the class does not define it, leaving the caller to fall
/// back to iteration — CPython checks `sq_contains` before `tp_iter`, so a
/// class defining both is never iterated by `in`. `__contains__ = None` is an
/// opt-out rather than an absence: it errors here instead of falling back.
///
/// The result is coerced with `py_bool`, which consults the returned object's
/// own `__bool__`/`__len__` when it is an instance, matching CPython's
/// `PyObject_IsTrue`.
pub(crate) fn instance_contains(self_id: HeapId, item: &Value, vm: &mut VM<'_>) -> RunResult<Option<bool>> {
    let class_id = instance_class(self_id, vm);
    if matches!(class_lookup(class_id, "__contains__", vm), Some(Value::None)) {
        return Err(ExcType::type_error_object_not_container(&class_name(
            class_id, vm.heap, vm.interns,
        )));
    }
    // The callee owns its argument, so the borrowed `item` is cloned;
    // `instance_call_dunder_sync` drops it again if there is no `__contains__`.
    let item = item.clone_with_heap(vm.heap);
    match instance_call_dunder_sync(self_id, "__contains__", ArgValues::One(item), vm)? {
        Some(result) => {
            defer_drop!(result, vm);
            Ok(Some(result.py_bool(vm)?))
        }
        None => Ok(None),
    }
}

/// Calls a user-defined string dunder (`__repr__`/`__str__`) on the instance and
/// validates that it returned a `str`.
///
/// Returns `Ok(None)` if the class does not define the dunder (caller uses the
/// default). The method runs to completion synchronously via `evaluate_function`,
/// so — unlike `__init__` — it cannot suspend on external/OS calls (see
/// `limitations/classes.md`). Recursion (e.g. a `__repr__` that reprs `self`)
/// re-enters the VM on the *Rust* stack; `evaluate_function`'s re-entry guard
/// bounds it with a catchable `RecursionError` — lower than CPython's depth for
/// deep-but-finite chains, a documented divergence (`limitations/classes.md`).
fn instance_call_str_dunder(self_id: HeapId, dunder: &'static str, vm: &mut VM<'_>) -> RunResult<Option<Value>> {
    let Some(result) = instance_call_dunder_sync(self_id, dunder, ArgValues::Empty, vm)? else {
        return Ok(None);
    };
    // CPython requires `__repr__`/`__str__` to return a `str`; reject any other
    // type with the same TypeError, dropping the offending return value.
    if result.is_str(vm.heap) {
        Ok(Some(result))
    } else {
        let exc = ExcType::type_error(format!(
            "{dunder} returned non-string (type {})",
            result.py_type_name(vm)
        ));
        result.drop_with(vm);
        Err(exc)
    }
}

/// Calls a zero- or one-argument dunder (`__repr__`, `__next__`,
/// `__contains__`, ...) on the instance, binding `self` for a plain-function
/// member. `Ok(None)` if the class does not define it — `arg` is dropped here
/// in that case, so callers hand over ownership unconditionally.
///
/// Synchronous: the callee runs inside `evaluate_function` rather than as a
/// pushed frame, so — unlike `__enter__` via [`call_member_bound`] — it cannot
/// suspend on an external/OS call. That is forced by the callers' signatures,
/// which must hand a `Value` straight back; see `limitations/classes.md`.
fn instance_call_dunder_sync(
    self_id: HeapId,
    dunder: &'static str,
    args: ArgValues,
    vm: &mut VM<'_>,
) -> RunResult<Option<Value>> {
    let class_id = instance_class(self_id, vm);
    let Some(func) = class_member(class_id, dunder, vm) else {
        args.drop_with(vm);
        return Ok(None);
    };
    defer_drop!(func, vm);
    // Only a plain function binds `self` as a descriptor (CPython's method
    // lookup protocol, mirrored by `call_member_bound`); an already-bound
    // method, a `staticmethod`, or another callable value is invoked without it.
    if let Some((kind, unwrapped)) = method_descriptor(func, vm) {
        defer_drop!(unwrapped, vm);
        let args = match kind {
            MethodKind::Static => args,
            MethodKind::Class => {
                vm.heap.inc_ref(class_id);
                args.prepend(Value::Ref(class_id))
            }
        };
        return vm.evaluate_function(dunder, unwrapped, args).map(Some);
    }
    let args = if is_method_value(func, vm) {
        vm.heap.inc_ref(self_id);
        args.prepend(Value::Ref(self_id))
    } else {
        args
    };
    vm.evaluate_function(dunder, func, args).map(Some)
}

/// `obj[key]` on a user instance: dispatches to a class-defined `__getitem__`.
///
/// Synchronous like `__eq__`/`__repr__` dispatch (`instance_call_dunder_sync`),
/// so the method cannot suspend on an external/OS call. A class defining no
/// `__getitem__` falls through to the trait default, which raises the same
/// `TypeError` an instance raises today.
pub(crate) fn instance_subscript(self_id: HeapId, key: &Value, vm: &mut VM<'_>) -> RunResult<Value> {
    let key_owned = key.clone_with_heap(vm.heap);
    match instance_call_dunder_sync(self_id, "__getitem__", ArgValues::One(key_owned), vm)? {
        Some(value) => Ok(value),
        None => vm.heap.read(self_id).py_getitem(key, vm),
    }
}

/// `obj[key] = value` on a user instance, through a class-defined `__setitem__`.
///
/// Takes ownership of both operands; a class defining no `__setitem__` raises
/// CPython's `'C' object does not support item assignment`.
pub(crate) fn instance_setitem(self_id: HeapId, key: Value, value: Value, vm: &mut VM<'_>) -> RunResult<()> {
    let args = ArgValues::Two(key, value);
    match instance_call_dunder_sync(self_id, "__setitem__", args, vm)? {
        Some(result) => {
            result.drop_with(vm);
            Ok(())
        }
        None => Err(ExcType::type_error_not_sub_assignment(&class_name(
            instance_class(self_id, vm),
            vm.heap,
            vm.interns,
        ))),
    }
}

/// `obj(...)` on a user instance, through a class-defined `__call__`.
///
/// Unlike the dunders above this runs as a real pushed frame, so a `__call__`
/// may suspend on external/OS calls exactly as an ordinary method does.
pub(crate) fn instance_call(self_id: HeapId, args: ArgValues, vm: &mut VM<'_>) -> RunResult<CallResult> {
    let class_id = instance_class(self_id, vm);
    let Some(member) = class_member(class_id, "__call__", vm) else {
        args.drop_with(vm);
        return Err(ExcType::type_error_not_callable_object(&class_name(
            class_id, vm.heap, vm.interns,
        )));
    };
    defer_drop!(member, vm);
    call_member_bound(member, self_id, class_id, args, vm)
}

/// `len(obj)` on a user instance, through a class-defined `__len__`.
///
/// `Ok(None)` means the class defines none, leaving the caller to raise
/// CPython's `object of type 'C' has no len()`. A negative or non-integer
/// return is rejected exactly as CPython's `PyObject_Size` rejects it.
pub(crate) fn instance_len(self_id: HeapId, vm: &mut VM<'_>) -> RunResult<Option<usize>> {
    let Some(result) = instance_call_dunder_sync(self_id, "__len__", ArgValues::Empty, vm)? else {
        return Ok(None);
    };
    defer_drop!(result, vm);
    match result {
        Value::Int(i) if *i >= 0 => Ok(Some(usize::try_from(*i).map_err(|_| ExcType::overflow_c_ssize_t())?)),
        Value::Int(_) => Err(ExcType::value_error("__len__() should return >= 0")),
        Value::Bool(b) => Ok(Some(usize::from(*b))),
        other => Err(ExcType::type_error(format!(
            "'{}' object cannot be interpreted as an integer",
            other.py_type_name(vm)
        ))),
    }
}

/// `bool(obj)` on a user instance: `__bool__` first, then `__len__`, then the
/// always-truthy default, which is CPython's `PyObject_IsTrue` order.
///
/// A `__bool__` returning a non-`bool` is rejected as CPython rejects it.
pub(crate) fn instance_bool(self_id: HeapId, vm: &mut VM<'_>) -> RunResult<bool> {
    if let Some(result) = instance_call_dunder_sync(self_id, "__bool__", ArgValues::Empty, vm)? {
        defer_drop!(result, vm);
        return match result {
            Value::Bool(b) => Ok(*b),
            other => Err(ExcType::type_error(format!(
                "__bool__ should return bool, returned {}",
                other.py_type_name(vm)
            ))),
        };
    }
    Ok(instance_len(self_id, vm)?.is_none_or(|len| len > 0))
}

/// Whether `self_id` is an instance whose class has an `__iter__` member —
/// including a `None` one, which opts the class out of iteration.
///
/// The distinction is CPython's `tp_iter`-is-non-NULL test, which only the
/// unpack error message needs (see `unpack_type_error`); everywhere else the
/// question is [`HeapRead::py_is_iterable`].
pub(crate) fn instance_defines_iter(self_id: HeapId, vm: &VM<'_>) -> bool {
    match vm.heap.get(self_id) {
        HeapData::Instance(inst) => class_defines(inst.class, "__iter__", vm),
        _ => false,
    }
}

/// Whether `class_id` or one of its bases defines `dunder`, without cloning it out.
///
/// Special-method lookup goes through the class only, never the instance
/// `__dict__`, matching CPython's lookup for implicit invocations. A slot whose
/// `None` value opts the class out of the protocol wants
/// [`class_defines_not_none`] instead.
pub(crate) fn class_defines(class_id: HeapId, dunder: &str, vm: &VM<'_>) -> bool {
    class_has_member(class_id, dunder, vm)
}

/// Whether `class_id` or one of its bases defines `dunder` as something other
/// than `None`.
///
/// `__iter__ = None` and `__contains__ = None` are explicit protocol opt-outs:
/// CPython's `slot_tp_iter` / `slot_sq_contains` reject a `None` member with the
/// same error as an absent one rather than calling it. This is per-slot, not
/// general — `__next__ = None` keeps the class an iterator (see
/// [`HeapRead::py_is_iterator`]), so use [`class_defines`] there.
pub(crate) fn class_defines_not_none(class_id: HeapId, dunder: &str, vm: &VM<'_>) -> bool {
    match class_lookup(class_id, dunder, vm) {
        Some(member) => !matches!(member, Value::None),
        // A natively provided base can only supply a real member, never `None`.
        None => native_default_member(class_id, dunder, vm).is_some(),
    }
}

/// Borrows `name` out of the first class in `class_id`'s base chain that binds
/// it, or `None` if no class does.
///
/// The class's *own* namespace, never a base's: CPython's `has_explicit_hash`
/// reads `cls.__dict__`, so an inherited `__hash__` must not answer here.
/// Callers needing to tell a `None` member apart from an absent one want this
/// rather than either existence check.
pub(crate) fn class_dunder<'v>(class_id: HeapId, dunder: &str, vm: &'v VM<'_>) -> Option<&'v Value> {
    match vm.heap.get(class_id) {
        HeapData::Class(class) => class.namespace().get_by_str(dunder, vm.heap, vm.interns),
        _ => None,
    }
}

/// This *is* the method resolution order: with single inheritance the chain is
/// a list, walked derived-first so an override wins. Backs the existence checks
/// above without the `clone_with_heap` that [`class_member`] pays to hand out an
/// owned value.
fn class_lookup<'v>(class_id: HeapId, name: &str, vm: &'v VM<'_>) -> Option<&'v Value> {
    let mut current = Some(class_id);
    for _ in 0..MAX_MRO_DEPTH {
        let id = current?;
        if let HeapData::Class(class) = vm.heap.get(id)
            && let Some(value) = class.namespace().get_by_str(name, vm.heap, vm.interns)
        {
            return Some(value);
        }
        current = class_base_id(id, vm);
    }
    None
}

/// The default `repr` for an instance with no user `__repr__`.
fn default_repr(self_id: HeapId, vm: &mut VM<'_>) -> String {
    let class_id = instance_class(self_id, vm);
    format!(
        "<{} object at 0x{:x}>",
        class_name(class_id, vm.heap, vm.interns),
        self_id.index()
    )
}

/// Returns the `HeapId` of `self_id`'s class object.
fn instance_class(self_id: HeapId, vm: &VM<'_>) -> HeapId {
    instance_class_id(self_id, vm).expect("instance_class called on non-instance heap value")
}

/// The class of `self_id`, or `None` when it is not an instance.
///
/// The checked form of [`instance_class`], for callers that reached a heap id
/// without already knowing what it holds.
pub(crate) fn instance_class_id(self_id: HeapId, vm: &VM<'_>) -> Option<HeapId> {
    match vm.heap.get(self_id) {
        HeapData::Instance(inst) => Some(inst.class),
        _ => None,
    }
}

/// Owned copies of an exception instance's `args`, empty when it has none.
///
/// `BaseException.__new__` stores the constructor arguments there, so this is
/// where `str(e)` and the traceback message come from.
pub(crate) fn instance_args(self_id: HeapId, vm: &VM<'_>) -> Vec<Value> {
    let Some(args) = (match vm.heap.get(self_id) {
        HeapData::Instance(inst) => inst.attrs.get_by_str("args", vm.heap, vm.interns),
        _ => None,
    }) else {
        return Vec::new();
    };
    match args {
        Value::Ref(id) => match vm.heap.get(*id) {
            HeapData::Tuple(tuple) => tuple.as_slice().iter().map(|v| v.clone_with_heap(vm.heap)).collect(),
            _ => vec![args.clone_with_heap(vm.heap)],
        },
        other => vec![other.clone_with_heap(vm.heap)],
    }
}

/// Dispatches a user-defined `__eq__`, or `Ok(None)` when it is absent.
///
/// The user's value is preserved so direct equality can return it unchanged;
/// callers interpret `NotImplemented` according to their comparison mode.
pub(crate) fn instance_user_eq(self_id: HeapId, other: &Value, vm: &mut VM<'_>) -> RunResult<Option<Value>> {
    if !matches!(vm.heap.get(self_id), HeapData::Instance(_)) {
        return Ok(None);
    }
    let class_id = instance_class(self_id, vm);
    if !class_defines(class_id, "__eq__", vm) {
        return Ok(None);
    }
    let other = other.clone_with_heap(vm.heap);
    instance_call_dunder_sync(self_id, "__eq__", ArgValues::One(other), vm)
}

/// Dispatches the synthesized field-wise `__eq__` of a dataclass instance, or
/// `Ok(None)` when `self_id` is not one — or is one declared `eq=False` — which
/// leaves the caller on identity.
///
/// Not in `HeapRead<Instance>::py_eq_impl` because fields are read as
/// `self.field` is (see [`instance_attr`]), which needs the instance's `HeapId`.
pub(crate) fn instance_dataclass_eq(self_id: HeapId, other: &Value, vm: &mut VM<'_>) -> RunResult<Option<bool>> {
    if !matches!(vm.heap.get(self_id), HeapData::Instance(_)) {
        return Ok(None);
    }
    let class_id = instance_class(self_id, vm);
    if !dataclasses::is_dataclass_class(class_id, vm) {
        return Ok(None);
    }
    dataclasses::dataclass_eq(self_id, class_id, other, vm)
}

/// Dispatches a user-defined `__hash__`, enforcing CPython's integer-return
/// contract. Only reached once the class is known to define a non-`None`
/// `__hash__`, so an absent member is an internal error.
fn instance_user_hash(self_id: HeapId, vm: &mut VM<'_>) -> RunResult<Option<HashValue>> {
    let Some(result) = instance_call_dunder_sync(self_id, "__hash__", ArgValues::Empty, vm)? else {
        return Err(RunError::internal(
            "instance_user_hash: __hash__ vanished from the class",
        ));
    };
    defer_drop!(result, vm);
    // CPython accepts any int (bool included, as an int subclass) and rejects
    // everything else before the value is ever used as a hash.
    if matches!(result.py_type(vm), Type::Int | Type::Bool) {
        result.py_hash(vm)
    } else {
        Err(ExcType::type_error(format!(
            "__hash__ method should return an integer, not {}",
            result.py_type_name(vm)
        )))
    }
}

/// Looks up a member in a class and its bases, cloning it out; `None` if no
/// class in the chain binds `name`.
pub(crate) fn class_member(class_id: HeapId, name: &str, vm: &VM<'_>) -> Option<Value> {
    match class_lookup(class_id, name, vm) {
        Some(member) => Some(member.clone_with_heap(vm.heap)),
        // Nothing in the chain binds it, so a natively provided base gets its
        // turn: this is what makes `Iterator.__iter__` reach a subclass that
        // only defined `__next__`.
        None => native_default_member(class_id, name, vm),
    }
}

/// Whether `class_id` or one of its bases binds `name`, without taking a
/// reference to the value: the answer a structural check needs, and a clone
/// taken to be thrown away would strand its refcount.
pub(crate) fn class_has_member(class_id: HeapId, name: &str, vm: &VM<'_>) -> bool {
    class_lookup(class_id, name, vm).is_some() || native_default_member(class_id, name, vm).is_some()
}

/// The `staticmethod`/`classmethod` wrapper `member` is, with its wrapped
/// callable cloned out; `None` for anything else.
fn method_descriptor(member: &Value, vm: &VM<'_>) -> Option<(MethodKind, Value)> {
    match member {
        Value::Ref(id) => match vm.heap.get(*id) {
            HeapData::MethodDescriptor(md) => Some((md.kind, md.func.clone_with_heap(vm.heap))),
            _ => None,
        },
        _ => None,
    }
}

/// The `property`'s accessor triple, cloned out; `None` for anything else.
fn user_property(member: &Value, vm: &VM<'_>) -> Option<(Value, Value)> {
    match member {
        Value::Ref(id) => match vm.heap.get(*id) {
            HeapData::Property(p) => Some((p.fget.clone_with_heap(vm.heap), p.fset.clone_with_heap(vm.heap))),
            _ => None,
        },
        _ => None,
    }
}

/// Wraps `func` as a method bound to `owner_id`, consuming `func`'s reference.
fn bind_method(func: Value, owner_id: HeapId, vm: &mut VM<'_>) -> Value {
    vm.heap.inc_ref(owner_id);
    let bound = BoundMethod {
        instance: Value::Ref(owner_id),
        func,
    };
    Value::Ref(vm.heap.allocate(HeapData::BoundMethod(bound)))
}

/// Applies the descriptor protocol for *class* access (`Foo.attr`): a
/// `staticmethod` unwraps to the bare function and a `classmethod` binds the
/// class. A `property` is returned as-is, matching CPython, where the
/// descriptor's `__get__` receives `None` for the instance and hands back the
/// property object. Takes ownership of `member`.
pub(crate) fn descriptor_class_get(member: Value, class_id: HeapId, vm: &mut VM<'_>) -> Value {
    let Some((kind, func)) = method_descriptor(&member, vm) else {
        return member;
    };
    member.drop_with(vm);
    match kind {
        MethodKind::Static => func,
        MethodKind::Class => bind_method(func, class_id, vm),
    }
}

/// Applies the descriptor protocol for *instance* access (`obj.attr`): a
/// `property` runs its getter, a `staticmethod` unwraps, a `classmethod` binds
/// the class, and a plain function binds the instance. Takes ownership of
/// `member`.
///
/// A property getter runs through `evaluate_function`, so (like
/// `__repr__`/`__eq__`) it cannot suspend on an external/OS call (see
/// `limitations/classes.md`).
fn descriptor_instance_get(
    member: Value,
    attr: &str,
    self_id: HeapId,
    class_id: HeapId,
    vm: &mut VM<'_>,
) -> RunResult<Value> {
    if let Some((fget, _)) = user_property(&member, vm) {
        member.drop_with(vm);
        defer_drop!(fget, vm);
        return if matches!(fget, Value::None) {
            Err(ExcType::attribute_error_property(
                attr,
                &class_name(class_id, vm.heap, vm.interns),
                "getter",
            ))
        } else {
            vm.heap.inc_ref(self_id);
            vm.evaluate_function("property", fget, ArgValues::One(Value::Ref(self_id)))
        };
    }
    if let Some((kind, func)) = method_descriptor(&member, vm) {
        member.drop_with(vm);
        return Ok(match kind {
            MethodKind::Static => func,
            MethodKind::Class => bind_method(func, class_id, vm),
        });
    }
    Ok(if is_method_value(&member, vm) {
        bind_method(member, self_id, vm)
    } else {
        member
    })
}

/// Runs a `property` setter for `obj.attr = value` when the class chain binds
/// `attr` to a property.
///
/// Returns `Ok(None)` once the setter has consumed `value`, and `Ok(Some(value))`
/// when no property is involved, handing ownership back so the caller can write
/// to the instance `__dict__` as usual.
pub(crate) fn descriptor_instance_set(
    self_id: HeapId,
    attr: &str,
    value: Value,
    vm: &mut VM<'_>,
) -> RunResult<Option<Value>> {
    let class_id = instance_class(self_id, vm);
    let Some(member) = class_member(class_id, attr, vm) else {
        return Ok(Some(value));
    };
    defer_drop!(member, vm);
    let Some((_, fset)) = user_property(member, vm) else {
        return Ok(Some(value));
    };
    defer_drop!(fset, vm);
    if matches!(fset, Value::None) {
        value.drop_with(vm);
        Err(ExcType::attribute_error_property(
            attr,
            &class_name(class_id, vm.heap, vm.interns),
            "setter",
        ))
    } else {
        vm.heap.inc_ref(self_id);
        let result = vm.evaluate_function("property", fset, ArgValues::Two(Value::Ref(self_id), value))?;
        result.drop_with(vm);
        Ok(None)
    }
}

/// Returns a class object's name for error messages / repr.
///
/// Takes `heap` + `interns` rather than a `&VM` so heap-only contexts (e.g.
/// `Type::name`) can resolve names. The result borrows only the interner —
/// interned names are `Cow::Borrowed`, while heap-owned names (classes created
/// by the 3-arg `type()` form) are cloned into `Cow::Owned` here, so either way
/// the result survives subsequent heap mutation.
///
/// # Panics
/// If `class_id` does not refer to a `Class` heap entry — every producer of a
/// class id (`Instance.class`, class values) guarantees it does, so this is a
/// programmer-error tripwire.
pub(crate) fn class_name<'i>(class_id: HeapId, heap: &Heap, interns: &'i Interns) -> Cow<'i, str> {
    match heap.get(class_id) {
        HeapData::Class(class) => match class.name() {
            EitherStr::Interned(id) => Cow::Borrowed(interns.get_str(*id)),
            EitherStr::Heap(s) => Cow::Owned(s.clone()),
        },
        _ => unreachable!("class_name called with a non-class heap id"),
    }
}

/// Calls a class member with CPython's descriptor-binding semantics: a
/// plain user-defined function binds `self` (prepended to `args`), a
/// `classmethod` binds the owning class, a `staticmethod` unwraps to its bare
/// function, and any other callable value is called as-is. Shared by
/// `py_call_attr` and the context-manager hooks (`py_enter`/`py_exit`) so dunder
/// invocation and ordinary method calls dispatch identically.
fn call_member_bound(
    member: &Value,
    self_id: HeapId,
    class_id: HeapId,
    args: ArgValues,
    vm: &mut VM<'_>,
) -> RunResult<CallResult> {
    if let Some((kind, func)) = method_descriptor(member, vm) {
        defer_drop!(func, vm);
        return match kind {
            MethodKind::Static => vm.call_function(func, args),
            MethodKind::Class => {
                vm.heap.inc_ref(class_id);
                vm.call_function(func, args.prepend(Value::Ref(class_id)))
            }
        };
    }
    if is_method_value(member, vm) {
        vm.heap.inc_ref(self_id);
        vm.call_function(member, args.prepend(Value::Ref(self_id)))
    } else {
        vm.call_function(member, args)
    }
}

/// Whether a value is a user-defined function (so it should bind `self` when
/// accessed as a method). Class variables that are not functions are returned
/// unbound.
fn is_method_value(value: &Value, vm: &VM<'_>) -> bool {
    match value {
        Value::DefFunction(..) => true,
        // The default methods a natively provided base contributes stand in for
        // functions CPython writes in Python, so they bind like one.
        Value::ModuleFunction(func) => func.binds_as_method(),
        // A `partialmethod` binds `self` like a function does — that binding is
        // the whole of what makes it a method (see `types::partialmethod`).
        Value::Ref(id) => matches!(
            vm.heap.get(*id),
            HeapData::Closure(_) | HeapData::FunctionDefaults(_) | HeapData::PartialMethod(_)
        ),
        _ => false,
    }
}

/// The `property`'s delete accessor, cloned out; `None` for anything else.
fn property_fdel(member: &Value, vm: &VM<'_>) -> Option<Value> {
    match member {
        Value::Ref(id) => match vm.heap.get(*id) {
            HeapData::Property(p) => Some(p.fdel.clone_with_heap(vm.heap)),
            _ => None,
        },
        _ => None,
    }
}

/// Runs a `property` deleter for `del obj.attr` when the class chain binds
/// `attr` to a property.
///
/// Reports `Ok(true)` once the deleter has run, and `Ok(false)` when no property
/// is involved, so the caller unbinds the instance `__dict__` entry as usual. A
/// property with no deleter raises rather than falling through: the descriptor
/// owns the name, and the dict entry it shadows must not be deleted instead.
fn descriptor_instance_del(self_id: HeapId, attr: &str, vm: &mut VM<'_>) -> RunResult<bool> {
    let class_id = instance_class(self_id, vm);
    let Some(member) = class_member(class_id, attr, vm) else {
        return Ok(false);
    };
    defer_drop!(member, vm);
    let Some(fdel) = property_fdel(member, vm) else {
        return Ok(false);
    };
    defer_drop!(fdel, vm);
    if matches!(fdel, Value::None) {
        return Err(ExcType::attribute_error_property(
            attr,
            &class_name(class_id, vm.heap, vm.interns),
            "deleter",
        ));
    }
    vm.heap.inc_ref(self_id);
    let result = vm.evaluate_function("property", fdel, ArgValues::One(Value::Ref(self_id)))?;
    result.drop_with(vm);
    Ok(true)
}

/// Removes `obj.attr`, running a class `property`'s deleter when one is defined
/// and otherwise unbinding from the instance `__dict__`.
///
/// The `Value`-level counterpart of [`HeapRead::py_del_attr`], for the same
/// reason [`instance_setattr`] is one: a deleter re-enters the VM, which cannot
/// happen behind a live heap-read handle.
pub(crate) fn instance_delattr(self_id: HeapId, name: &EitherStr, vm: &mut VM<'_>) -> RunResult<()> {
    let attr = name.as_str(vm.interns);
    if descriptor_instance_del(self_id, attr, vm)? {
        return Ok(());
    }
    vm.heap.read(self_id).py_del_attr(name, vm)
}

/// `del obj[key]` on a user instance, through a class-defined `__delitem__`.
///
/// Takes ownership of `key`; a class defining no `__delitem__` raises CPython's
/// `'C' object doesn't support item deletion`.
pub(crate) fn instance_delitem(self_id: HeapId, key: Value, vm: &mut VM<'_>) -> RunResult<()> {
    match instance_call_dunder_sync(self_id, "__delitem__", ArgValues::One(key), vm)? {
        Some(result) => {
            result.drop_with(vm);
            Ok(())
        }
        None => Err(ExcType::type_error_no_item_deletion(&class_name(
            instance_class(self_id, vm),
            vm.heap,
            vm.interns,
        ))),
    }
}
