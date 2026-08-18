//! Python property descriptor for computed attributes.
//!
//! Properties are descriptors whose value is computed when accessed.
//! When a Property is retrieved via `py_getattr`, its getter is invoked
//! rather than returning the Property itself.
//!
//! Two shapes live here. [`Property`] is the `Copy`, heap-free marker for
//! interpreter-owned zero-arg properties (`os.environ`), carried inline in a
//! `Value`. [`UserProperty`] and [`MethodDescriptor`] are the heap objects
//! `property()`, `staticmethod()` and `classmethod()` build: they hold user
//! `Value`s, so they cannot be `Copy` and must be reference-counted.

use std::{fmt::Write, mem};

use monty_types::OsFunctionCall;

use super::{LazyHeapSet, PyTrait, Type};
use crate::{
    args::{ArgValues, FromArgs},
    bytecode::{CallResult, VM},
    exception_private::{ExcType, ExcTypeExt, RunResult},
    hash::{HashValue, identity_hash},
    heap::{DropWithContext, HeapData, HeapId, HeapItem, HeapRead},
    value::{EitherStr, Value},
};

/// Property descriptor for computed attributes (mirrors Python's descriptor
/// protocol — accessing the property invokes its getter).
///
/// Only covers zero-arg OS properties (e.g. `os.environ`); the `property()`
/// builtin builds a [`UserProperty`] on the heap instead, since it must hold
/// arbitrary callables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(crate) enum Property {
    Os(ZeroArgOsProperty),
}

/// A `property(fget, fset, fdel)` object: the data descriptor behind
/// `@property` and its assignment form `x = property(_get)`.
///
/// Absent accessors are [`Value::None`], matching what `property()` stores, so
/// a missing setter is distinguishable from a setter that happens to be `None`
/// only by intent, not by representation: the same conflation CPython has.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct UserProperty {
    /// Getter invoked by `obj.x`; `None` makes the attribute write-only.
    pub fget: Value,
    /// Setter invoked by `obj.x = v`; `None` raises `AttributeError`.
    pub fset: Value,
    /// Deleter for `del obj.x`. Monty rejects `del` at parse time, so this is
    /// stored (so `property(g, s, d)` round-trips) but never invoked.
    pub fdel: Value,
}

impl UserProperty {
    /// Builds a property from the three accessor slots, each `Value::None`
    /// when the corresponding accessor was omitted.
    #[must_use]
    pub fn new(fget: Value, fset: Value, fdel: Value) -> Self {
        Self { fget, fset, fdel }
    }
}

impl HeapItem for UserProperty {
    fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        self.fget.py_dec_ref_ids(stack);
        self.fset.py_dec_ref_ids(stack);
        self.fdel.py_dec_ref_ids(stack);
    }
}

impl<'h> PyTrait<'h> for HeapRead<'h, UserProperty> {
    fn py_type(&self, _vm: &VM<'h>) -> Type {
        Type::Property
    }

    fn py_len(&self, _vm: &VM<'h>) -> Option<usize> {
        None
    }

    fn py_eq_impl(&self, _other: &Value, _vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        Ok(None)
    }

    fn py_hash(&self, self_id: HeapId, _vm: &mut VM<'h>) -> RunResult<Option<HashValue>> {
        Ok(Some(identity_hash(self_id)))
    }

    fn py_repr_fmt(&self, f: &mut impl Write, _vm: &mut VM<'h>, _heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        // CPython appends the object address; Monty has no stable one to show
        // here (the heap id is only available at the `Value` level).
        Ok(f.write_str("<property object>")?)
    }

    /// `p.getter(f)` / `p.setter(f)` / `p.deleter(f)` return a *new* property
    /// with that accessor replaced, which is what the `@x.setter` decorator
    /// form relies on.
    fn py_call_attr(
        &mut self,
        _self_id: HeapId,
        vm: &mut VM<'h>,
        attr: &EitherStr,
        args: ArgValues,
    ) -> RunResult<CallResult> {
        let slot = match attr.as_str(vm.interns) {
            "getter" => 0,
            "setter" => 1,
            "deleter" => 2,
            other => {
                let other = other.to_owned();
                args.drop_with(vm);
                return Err(ExcType::attribute_error("property", &other));
            }
        };
        let accessor = args.get_one_arg("property", vm.heap)?;
        let mut accessors = [
            self.get(vm.heap).fget.clone_with_heap(vm.heap),
            self.get(vm.heap).fset.clone_with_heap(vm.heap),
            self.get(vm.heap).fdel.clone_with_heap(vm.heap),
        ];
        let replaced = mem::replace(&mut accessors[slot], accessor);
        replaced.drop_with(vm);
        let [fget, fset, fdel] = accessors;
        Ok(CallResult::Value(allocate_property(fget, fset, fdel, vm)))
    }
}

/// The `property(fget, fset, fdel, doc)` constructor.
///
/// Positional-only: CPython also accepts these as keywords, which Monty does
/// not (see `limitations/classes.md`). `doc` is accepted and discarded, since
/// Monty exposes no `property.__doc__`.
#[derive(FromArgs)]
#[from_args(name = "property", style = unpack)]
struct PropertyArgs {
    #[from_args(pos_only, default = Value::None)]
    fget: Value,
    #[from_args(pos_only, default = Value::None)]
    fset: Value,
    #[from_args(pos_only, default = Value::None)]
    fdel: Value,
    #[from_args(pos_only, default = Value::None)]
    doc: Value,
}

/// Builds a `property` object; the `Type::Property` constructor's body.
pub(crate) fn property_init(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let PropertyArgs { fget, fset, fdel, doc } = PropertyArgs::from_args(args, vm)?;
    doc.drop_with(vm);
    Ok(allocate_property(fget, fset, fdel, vm))
}

/// Allocates a [`UserProperty`], taking ownership of all three accessors.
fn allocate_property(fget: Value, fset: Value, fdel: Value, vm: &mut VM<'_>) -> Value {
    Value::Ref(
        vm.heap
            .allocate(HeapData::Property(UserProperty::new(fget, fset, fdel))),
    )
}

/// The `staticmethod(f)` constructor.
pub(crate) fn staticmethod_init(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    allocate_method_descriptor(MethodKind::Static, args, vm)
}

/// The `classmethod(f)` constructor.
pub(crate) fn classmethod_init(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    allocate_method_descriptor(MethodKind::Class, args, vm)
}

/// Shared body of the two one-argument descriptor constructors.
fn allocate_method_descriptor(kind: MethodKind, args: ArgValues, vm: &mut VM<'_>) -> RunResult<Value> {
    let name: &'static str = kind.type_().into();
    let func = args.get_one_arg(name, vm.heap)?;
    Ok(Value::Ref(vm.heap.allocate(HeapData::MethodDescriptor(
        MethodDescriptor::new(kind, func),
    ))))
}

/// Which of the two non-data method descriptors a [`MethodDescriptor`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(crate) enum MethodKind {
    /// `staticmethod(f)`: attribute access yields `f` with nothing bound.
    Static,
    /// `classmethod(f)`: attribute access binds the owning class as first arg.
    Class,
}

impl MethodKind {
    /// The Python type name, used for `repr` and `type()`.
    pub fn type_(self) -> Type {
        match self {
            Self::Static => Type::StaticMethod,
            Self::Class => Type::ClassMethod,
        }
    }
}

/// A `staticmethod(f)` / `classmethod(f)` wrapper.
///
/// Both are non-data descriptors: they only affect attribute *reads*, so an
/// instance attribute of the same name still shadows them.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct MethodDescriptor {
    /// Which wrapper this is.
    pub kind: MethodKind,
    /// The wrapped callable.
    pub func: Value,
}

impl MethodDescriptor {
    /// Wraps `func` as `kind`.
    #[must_use]
    pub fn new(kind: MethodKind, func: Value) -> Self {
        Self { kind, func }
    }
}

impl HeapItem for MethodDescriptor {
    fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        self.func.py_dec_ref_ids(stack);
    }
}

impl<'h> PyTrait<'h> for HeapRead<'h, MethodDescriptor> {
    fn py_type(&self, vm: &VM<'h>) -> Type {
        self.get(vm.heap).kind.type_()
    }

    fn py_len(&self, _vm: &VM<'h>) -> Option<usize> {
        None
    }

    fn py_eq_impl(&self, _other: &Value, _vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        Ok(None)
    }

    fn py_hash(&self, self_id: HeapId, _vm: &mut VM<'h>) -> RunResult<Option<HashValue>> {
        Ok(Some(identity_hash(self_id)))
    }

    fn py_repr_fmt(&self, f: &mut impl Write, vm: &mut VM<'h>, heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        let kind: &'static str = self.get(vm.heap).kind.type_().into();
        let func = self.get(vm.heap).func.clone_with_heap(vm.heap);
        write!(f, "<{kind}(")?;
        let result = func.py_repr_fmt(f, vm, heap_ids);
        func.drop_with(vm);
        result?;
        Ok(f.write_str(")>")?)
    }
}

/// Discriminant for zero-arg OS-backed [`Property`]s. Kept `Copy` so
/// `Property` stays `Copy + Hash`; the matching [`OsFunctionCall`] (which
/// is not `Copy`) is built on access in [`Property::get`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(crate) enum ZeroArgOsProperty {
    /// `os.environ` — returns the host environment as a dict.
    GetEnviron,
}

impl Property {
    /// Invokes the getter, returning the `CallResult` the VM should act on.
    pub fn get(self) -> CallResult {
        match self {
            Self::Os(ZeroArgOsProperty::GetEnviron) => CallResult::OsCall(OsFunctionCall::GetEnviron),
        }
    }
}
