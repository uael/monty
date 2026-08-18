use std::fmt::Write;

use super::{Dict, LazyHeapSet, PyTrait, Type, attribute_name_value};
use crate::{
    args::ArgValues,
    bytecode::{CallResult, VM},
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunResult},
    hash::{HashValue, identity_hash},
    heap::{
        BorrowedHeapReadMut, DropGuard, DropWithContext, HeapData, HeapId, HeapItem, HeapRead,
        heap_read_ref_as_field_mut,
    },
    types::{
        instance::{class_member, class_name, descriptor_class_get},
        str::allocate_string,
    },
    value::{EitherStr, Value},
};

/// One `@dataclass(...)` keyword, and its bit in [`DataclassOptions`].
///
/// The order is CPython's `_DataclassParams` field order, which is also the
/// order `__dataclass_params__` reports them in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Opt {
    Init,
    Repr,
    Eq,
    Order,
    UnsafeHash,
    Frozen,
    MatchArgs,
    KwOnly,
    Slots,
    WeakrefSlot,
}

impl Opt {
    /// Every option, in `__dataclass_params__` order.
    pub(crate) const ALL: [Self; 10] = [
        Self::Init,
        Self::Repr,
        Self::Eq,
        Self::Order,
        Self::UnsafeHash,
        Self::Frozen,
        Self::MatchArgs,
        Self::KwOnly,
        Self::Slots,
        Self::WeakrefSlot,
    ];

    /// This option's bit.
    const fn bit(self) -> u16 {
        1 << (self as u16)
    }

    /// CPython's spelling of this keyword, which is both the `dataclass(...)`
    /// argument name and the `_DataclassParams` attribute reporting it.
    pub(crate) const fn keyword(self) -> &'static str {
        match self {
            Self::Init => "init",
            Self::Repr => "repr",
            Self::Eq => "eq",
            Self::Order => "order",
            Self::UnsafeHash => "unsafe_hash",
            Self::Frozen => "frozen",
            Self::MatchArgs => "match_args",
            Self::KwOnly => "kw_only",
            Self::Slots => "slots",
            Self::WeakrefSlot => "weakref_slot",
        }
    }
}

/// The `@dataclass(...)` options a class was decorated with.
///
/// Packed into one `u16` so it stays `Copy` and doubles as the payload of the
/// *configured decorator* (`dataclass(frozen=True)`) without a heap allocation
/// between the two calls that syntax makes. A decorated class holds these bits
/// directly, and `__dataclass_params__` reports them back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(crate) struct DataclassOptions(u16);

impl Default for DataclassOptions {
    /// CPython's defaults: `init=True, repr=True, eq=True, match_args=True`,
    /// everything else false.
    fn default() -> Self {
        Self(Opt::Init.bit() | Opt::Repr.bit() | Opt::Eq.bit() | Opt::MatchArgs.bit())
    }
}

impl DataclassOptions {
    /// Whether `option` is on.
    pub(crate) const fn get(self, option: Opt) -> bool {
        self.0 & option.bit() != 0
    }

    /// Turns `option` on or off.
    pub(crate) const fn set(&mut self, option: Opt, on: bool) {
        if on {
            self.0 |= option.bit();
        } else {
            self.0 &= !option.bit();
        }
    }
}
/// Ceiling on how far a base-class walk follows `__bases__`.
///
/// Bases are fixed at class creation and a base must already exist to be named,
/// so a live chain is finite and acyclic; the cap only bounds the damage from a
/// corrupted snapshot, where stopping early loses a lookup rather than hanging.
pub(crate) const MAX_MRO_DEPTH: usize = 100;

/// A user-defined class object created by a `class Foo: ...` statement.
///
/// Holds the class name and a `namespace` [`Dict`] mapping member names to values:
/// methods (stored as `DefFunction`/`Closure` values) and class variables. The
/// class's own [`HeapId`] is its type identity — `type(x) is Foo` and `isinstance`
/// work via reference identity, so there is no separate type-id counter.
///
/// Calling a class (`Foo(...)`) constructs an [`Instance`](super::Instance); see
/// `instantiate_class` in the VM's call module.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Class {
    /// Class name (e.g. `Foo`), used for `repr` and `__name__`. Interned for
    /// compiled `class` statements; heap-owned for classes created at runtime
    /// via the 3-arg `type(name, bases, dict)` form, whose name cannot be
    /// interned because the intern table is frozen after prepare.
    name: EitherStr,
    /// Members: method name / class-variable name -> value.
    namespace: Dict,
    /// The `@dataclass(...)` options this class was decorated with, left at
    /// CPython's defaults for a class that was not. Stands in for the dunders
    /// CPython generates and Monty cannot yet install: baked in at decoration
    /// so `__dataclass_params__` stays a report, not a rewritable control.
    options: DataclassOptions,
    /// Owned base-class values in declaration order: a `Value::Ref` to another
    /// `Class`, or a `Value::Builtin(Builtins::ExcType(..))` for a builtin
    /// exception root. Monty implements single inheritance, so this holds at
    /// most one entry (`create_class` rejects more).
    bases: Vec<Value>,
    /// Nearest builtin exception ancestor, resolved once at creation because
    /// bases never change. `Some` is what makes instances of this class
    /// raisable and catchable by `except <builtin>:`.
    exc_base: Option<ExcType>,
}

impl Class {
    /// Creates a class object from its name, member namespace and (owned) bases.
    ///
    /// Dataclass options start at their defaults; `@dataclass` sets them with
    /// [`HeapRead::set_dataclass_options`] once it has built the class.
    ///
    /// `exc_base` must already be the nearest builtin exception ancestor
    /// reachable through `bases`; the 3-arg `type()` constructor (`builtins::type_`)
    /// is the single place that derives it.
    #[must_use]
    pub fn new(name: EitherStr, namespace: Dict, bases: Vec<Value>, exc_base: Option<ExcType>) -> Self {
        Self {
            name,
            namespace,
            options: DataclassOptions::default(),
            bases,
            exc_base,
        }
    }

    /// The `@dataclass(...)` options in force for this class.
    ///
    /// Meaningful only once the class is known to be a dataclass — a plain one
    /// reports the defaults it was never decorated with.
    #[must_use]
    pub fn dataclass_options(&self) -> DataclassOptions {
        self.options
    }

    /// Returns the class name (interned or heap-owned).
    #[must_use]
    pub fn name(&self) -> &EitherStr {
        &self.name
    }

    /// Returns a reference to the class member namespace.
    #[must_use]
    pub fn namespace(&self) -> &Dict {
        &self.namespace
    }

    /// The declared base values, in order.
    #[must_use]
    pub fn bases(&self) -> &[Value] {
        &self.bases
    }

    /// The nearest builtin exception ancestor, or `None` for a plain class.
    #[must_use]
    pub fn exc_base(&self) -> Option<ExcType> {
        self.exc_base
    }
}

/// The single user-defined base class of `class_id`, or `None` at the top of
/// the chain (or when the only base is a builtin exception type).
pub(crate) fn class_base_id(class_id: HeapId, vm: &VM<'_>) -> Option<HeapId> {
    match vm.heap.get(class_id) {
        HeapData::Class(class) => class.bases().iter().find_map(|base| match base {
            Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::Class(_)) => Some(*id),
            _ => None,
        }),
        _ => None,
    }
}

/// The nearest builtin exception ancestor of `class_id`, or `None` when it is
/// not an exception class. Cached on the class, so this is a single read.
pub(crate) fn class_exc_base(class_id: HeapId, vm: &VM<'_>) -> Option<ExcType> {
    match vm.heap.get(class_id) {
        HeapData::Class(class) => class.exc_base(),
        _ => None,
    }
}

/// Whether `sub_id` is `base_id` or inherits from it, walking the base chain.
pub(crate) fn class_is_subclass(sub_id: HeapId, base_id: HeapId, vm: &VM<'_>) -> bool {
    let mut current = Some(sub_id);
    for _ in 0..MAX_MRO_DEPTH {
        match current {
            Some(id) if id == base_id => return true,
            Some(id) => current = class_base_id(id, vm),
            None => return false,
        }
    }
    false
}

impl<'h> HeapRead<'h, Class> {
    fn namespace_mut(&mut self) -> BorrowedHeapReadMut<'_, 'h, Dict> {
        heap_read_ref_as_field_mut!(self, Class, namespace)
    }

    /// Sets a class attribute (`Foo.x = 1`), returning the previous value (if any)
    /// for the caller to drop. Takes ownership of both `name` and `value`.
    ///
    /// Existing instances observe the change immediately: instance attribute reads
    /// fall through to this namespace.
    pub fn set_attr(&mut self, name: Value, value: Value, vm: &mut VM<'h>) -> RunResult<Option<Value>> {
        self.namespace_mut().set(name, value, vm)
    }

    /// Records what `@dataclass(...)` decorated this class with.
    ///
    /// Called once per decoration, so re-decorating replaces the options as it
    /// replaces the fields. Assigning to `__dataclass_params__` afterwards does
    /// not reach here, which is what makes that object a report rather than a
    /// control.
    pub fn set_dataclass_options(&mut self, options: DataclassOptions, vm: &mut VM<'h>) {
        self.get_mut(vm.heap).options = options;
    }

    /// Unbinds a class attribute, returning the removed `(name, value)` pair for
    /// the caller to release, or `None` when the name was not bound.
    ///
    /// The sandbox has no `del`, so this exists only for `@dataclass`, which
    /// must remove the `field()` spec a class body left behind when that field
    /// has no plain default: `C.x` then raises, as it does in CPython.
    pub fn del_attr(&mut self, name: &Value, vm: &mut VM<'h>) -> RunResult<Option<(Value, Value)>> {
        self.namespace_mut().pop(name, vm)
    }
}

impl<'h> PyTrait<'h> for HeapRead<'h, Class> {
    fn py_type(&self, _vm: &VM<'h>) -> Type {
        // The type of a class object is `type` (matching `type(Foo) is type`).
        Type::Type
    }

    fn py_len(&self, _vm: &VM<'h>) -> Option<usize> {
        None
    }

    fn py_set_attr(&mut self, name: &EitherStr, value: Value, vm: &mut VM<'h>) -> RunResult<()> {
        let mut value_guard = DropGuard::new(value, vm);
        let name = attribute_name_value(name, value_guard.ctx());
        let (value, vm) = value_guard.into_parts();
        let old_value = self.set_attr(name, value, vm)?;
        old_value.drop_with(vm);
        Ok(())
    }

    fn py_eq_impl(&self, _other: &Value, _vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        // Classes return `NotImplemented`; rich equality's final identity
        // fallback makes a class equal only to itself.
        Ok(None)
    }

    fn py_hash(&self, self_id: HeapId, _vm: &mut VM<'h>) -> RunResult<Option<HashValue>> {
        // Class objects hash by identity (like CPython type objects).
        Ok(Some(identity_hash(self_id)))
    }

    fn py_repr_fmt(&self, f: &mut impl Write, vm: &mut VM<'h>, _heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        Ok(write!(f, "<class '{}'>", self.get(vm.heap).name.as_str(vm.interns))?)
    }

    fn py_call_attr(
        &mut self,
        self_id: HeapId,
        vm: &mut VM<'h>,
        attr: &EitherStr,
        args: ArgValues,
    ) -> RunResult<CallResult> {
        let attr_str = attr.as_str(vm.interns);
        // `__name__` is a synthesized string, not a namespace member (see
        // [`class_getattr`]), so calling it goes through the normal callable
        // dispatch and raises CPython's `TypeError: 'str' object is not
        // callable` rather than a spurious `AttributeError`.
        if attr_str == "__name__" {
            let name = self.get(vm.heap).name.as_str(vm.interns).to_owned();
            let name_val = allocate_string(name, vm.heap);
            defer_drop!(name_val, vm);
            return vm.call_function(name_val, args);
        }
        // `Foo.method(args)` calls the raw (unbound) member with the given args —
        // no `self` is inserted, the caller passes the instance explicitly. A
        // `classmethod` is the exception: it binds the class, as in CPython.
        let member = class_member(self_id, attr_str, vm);
        if let Some(member) = member {
            let member = descriptor_class_get(member, self_id, vm);
            defer_drop!(member, vm);
            vm.call_function(member, args)
        } else {
            args.drop_with(vm);
            Err(ExcType::attribute_error_type(
                self.get(vm.heap).name.as_str(vm.interns),
                attr_str,
            ))
        }
    }
}

/// Reads a class attribute for `Foo.attr`: the `Value`-level counterpart of
/// [`instance_getattr`](super::instance::instance_getattr), taking the class's
/// own `HeapId` because both the base-class walk and descriptor binding need it.
///
/// Lookup order is `__name__`, then this class's namespace, then each base's.
pub(crate) fn class_getattr(class_id: HeapId, attr: &EitherStr, vm: &mut VM<'_>) -> RunResult<CallResult> {
    let attr_str = attr.as_str(vm.interns);
    // `Foo.__name__` returns the class name, checked before the namespace lookup
    // because in CPython `type.__name__` is a metaclass data descriptor that
    // shadows a same-named class-dict member (`class Foo: __name__ = 'bar'`
    // still reads `'Foo'`; only instances see the member).
    if attr_str == "__name__" {
        let name = class_name(class_id, vm.heap, vm.interns).into_owned();
        return Ok(CallResult::Value(allocate_string(name, vm.heap)));
    }
    match class_member(class_id, attr_str, vm) {
        Some(member) => Ok(CallResult::Value(descriptor_class_get(member, class_id, vm))),
        None => Err(ExcType::attribute_error_type(
            &class_name(class_id, vm.heap, vm.interns),
            attr_str,
        )),
    }
}

impl HeapItem for Class {
    fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        self.namespace.py_dec_ref_ids(stack);
        for base in &mut self.bases {
            base.py_dec_ref_ids(stack);
        }
    }
}
