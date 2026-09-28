//! Python property descriptor for computed attributes.
//!
//! Properties are descriptors whose value is computed when accessed.
//! When a Property is retrieved via `py_getattr`, its getter is invoked
//! rather than returning the Property itself.

use std::fmt::Write;

use monty_types::OsFunctionCall;

use crate::{
    args::{ArgValues, FromArgs},
    bytecode::{CallResult, VM},
    exception_private::{ExcType, ExcTypeExt, RunResult},
    heap::{DropGuard, DropWithContext, HeapId, HeapItem, HeapObjectRead},
    types::{LazyHeapSet, PyTrait, Type},
    value::Value,
};

/// Property descriptor for computed attributes (mirrors Python's descriptor
/// protocol — accessing the property invokes its getter).
///
/// Currently only supports zero-arg OS properties (e.g. `os.environ`).
/// Future variants will likely add `Callable(FunctionId)` for `@property`
/// and `External(StringId)` for external function getters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(crate) enum Property {
    Os(ZeroArgOsProperty),
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

/// A `@property` on a user-defined class: a getter the class binds by name,
/// called on every read of the attribute.
///
/// `fget` is an OWNED reference, released by [`HeapItem::py_dec_ref_ids`].
/// Only the getter is stored: `property(fget, fset, fdel, doc)`'s other three
/// arguments are refused, so an attribute with one is read-only (see
/// `limitations/classes.md`).
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct ClassProperty {
    /// Owned callable taking the instance and returning the attribute's value.
    fget: Value,
}

impl ClassProperty {
    /// A property over this getter, which it owns from now on.
    pub(crate) fn new(fget: Value) -> Self {
        Self { fget }
    }

    /// The getter, for the instance read that calls it. Borrowed, not owned.
    pub(crate) fn fget(&self) -> &Value {
        &self.fget
    }
}

/// `property(fget)`: builds the descriptor a `@property` binds.
///
/// CPython's signature is `property(fget=None, fset=None, fdel=None, doc=None)`.
/// Monty takes the getter alone and refuses the rest, because an attribute it
/// cannot write is better than one that silently ignores the setter it was
/// given.
pub(crate) fn property_init(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let PropertyArgs { fget, fset, fdel, doc } = PropertyArgs::from_args(args, vm)?;
    let mut guard = DropGuard::new([fget, fset, fdel, doc], vm);
    let (values, _) = guard.as_parts();
    let [_, fset, fdel, doc] = values;
    let unsupported = [("fset", fset), ("fdel", fdel), ("doc", doc)]
        .into_iter()
        .find(|(_, value)| !matches!(value, Value::None))
        .map(|(name, _)| name);
    let (values, vm) = guard.into_parts();
    let [fget, rest @ ..] = values;
    rest.drop_with(vm);
    match unsupported {
        Some(name) => {
            fget.drop_with(vm);
            Err(ExcType::not_implemented(format!("property() does not yet support the {name} argument")).into())
        }
        None if matches!(fget, Value::None) => Err(ExcType::type_error("property() takes a getter")),
        None => Ok(vm.heap.allocate_as(ClassProperty::new(fget)).into_value()),
    }
}

/// `property(fget=None, fset=None, fdel=None, doc=None)`, CPython's signature.
#[derive(FromArgs)]
#[from_args(style = def, name = "property")]
struct PropertyArgs {
    #[from_args(default = Value::None)]
    fget: Value,
    #[from_args(default = Value::None)]
    fset: Value,
    #[from_args(default = Value::None)]
    fdel: Value,
    #[from_args(default = Value::None)]
    doc: Value,
}

impl<'h> PyTrait<'h> for HeapObjectRead<'h, ClassProperty> {
    fn py_type(&self, _vm: &VM<'h>) -> Type {
        Type::Property
    }

    fn py_len(&self, _vm: &VM<'h>) -> Option<usize> {
        None
    }

    fn py_eq_impl(&self, _other: &Value, _vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        // CPython's `property` defines no `__eq__`; identity is resolved first.
        Ok(None)
    }

    fn py_repr_fmt(&self, f: &mut impl Write, vm: &mut VM<'h>, _heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        self.py_default_repr_fmt(f, vm)
    }
}

impl HeapItem for ClassProperty {
    fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        self.fget.py_dec_ref_ids(stack);
    }
}
