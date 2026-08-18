//! A host object the sandbox holds by reference.
//!
//! Everything else at the boundary is a copy, so an object whose identity or
//! whose live state is the point cannot cross at all. A [`HostRef`] crosses
//! instead: an opaque proxy holding only the host's own id, unforgeable from
//! inside (no constructor is exposed and the id is never spelled in the
//! sandbox), whose operations suspend back to the host rather than being
//! answered here.
//!
//! The suspension is a [`CallResult::MethodCall`], the same exit a dataclass
//! method already takes, carrying the reference itself as the first argument.
//! So the host resolves the receiver exactly once, where it resolves any
//! argument, and the boundary needs no new event and no new protocol version.
//! What varies is only the operation name: an ordinary attribute name for a
//! method call, and otherwise one of the reserved names in [`ops`].
//!
//! Each reserved name is a dunder, which is what closes the set. A dunder is
//! rejected as an attribute the sandbox asks for by name, so a dunder reaching
//! the host can only have been produced by the VM performing that operation --
//! sandboxed code cannot forge `__getattr__` into a read of `__class__` and
//! walk out through the type graph.

use std::fmt::Write;

use crate::{
    args::ArgValues,
    bytecode::{CallResult, VM},
    exception_private::{ExcType, ExcTypeExt, RunResult},
    hash::{HashValue, hash_one},
    heap::{DropWithContext, HeapData, HeapId, HeapRead},
    types::{LazyHeapSet, PyTrait, Type, str::allocate_string},
    value::{EitherStr, Value},
};

/// Operation names a host reference sends in place of a method name.
pub(crate) mod ops {
    /// Read one attribute: `(ref, name)`.
    pub(crate) const GETATTR: &str = "__getattr__";
    /// Call the reference itself: `(ref, *args, **kwargs)`.
    pub(crate) const CALL: &str = "__call__";
    /// Enter the reference as a context manager: `(ref,)`.
    pub(crate) const ENTER: &str = "__enter__";
    /// Leave a context manager: `(ref, exc)`, `exc` being the live exception or
    /// `None`. The type and traceback CPython also passes are not reproducible
    /// (monty has no traceback objects), which is how every other `__exit__` in
    /// the interpreter is already called.
    pub(crate) const EXIT: &str = "__exit__";
    /// Await the reference itself: `(ref,)`. The host answers with whatever it
    /// would await, and the sandbox awaits that.
    pub(crate) const AWAIT: &str = "__await__";
}

/// A reference to an object living on the host.
///
/// A leaf: it owns no heap values, so it cannot take part in a reference cycle
/// and is not GC-tracked. Two references name the same object exactly when
/// their ids are equal, and what an id means is the host's business.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct HostRef {
    /// The host's identifier for the object. Never readable from sandboxed
    /// code: it is frequently a real memory address.
    id: u64,
    /// `type(obj).__name__` on the host, carried so `repr()` and every error
    /// message can name what the proxy stands for without a round trip.
    type_name: String,
}

impl HostRef {
    /// Wraps a host id and the name of its type.
    pub(crate) fn new(id: u64, type_name: String) -> Self {
        Self { id, type_name }
    }

    /// The host's identifier for the referenced object.
    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    /// The name of the referenced object's type on the host.
    pub(crate) fn type_name(&self) -> &str {
        &self.type_name
    }
}

/// Builds the suspension asking the host to perform `op` on the reference at
/// `self_id`, with the reference itself as the first argument.
///
/// The reference becomes an argument, and an argument owns a count on the
/// entry it names, so this takes one.
fn dispatch(self_id: HeapId, op: impl Into<EitherStr>, args: ArgValues, vm: &mut VM<'_>) -> CallResult {
    vm.heap.inc_ref(self_id);
    CallResult::MethodCall(op.into(), args.prepend(Value::Ref(self_id)))
}

/// The type name of the reference at `self_id`, for an error message.
///
/// # Panics
/// If `self_id` does not name a host reference; every caller has matched on
/// [`HeapData::HostRef`] first.
fn type_name_of(self_id: HeapId, vm: &VM<'_>) -> String {
    let HeapData::HostRef(host_ref) = vm.heap.get(self_id) else {
        unreachable!("only reached for a host reference");
    };
    host_ref.type_name.clone()
}

/// Reads one attribute of the referenced object, on the host.
///
/// Intercepted at [`Value::py_getattr`](crate::value::Value::py_getattr)
/// rather than through [`PyTrait::py_getattr`], which is handed no heap id and
/// so could not make the reference an argument. Instances and class objects
/// are intercepted there for the same reason.
pub(crate) fn host_ref_getattr(self_id: HeapId, attr: &EitherStr, vm: &mut VM<'_>) -> RunResult<CallResult> {
    let name = attr.as_str(vm.interns);
    if name.starts_with('_') {
        return Err(ExcType::attribute_error(type_name_of(self_id, vm), name));
    }
    let name = allocate_string(name.to_owned(), vm.heap);
    Ok(dispatch(self_id, ops::GETATTR.to_owned(), ArgValues::One(name), vm))
}

/// Calls the referenced object itself, on the host.
pub(crate) fn host_ref_call(self_id: HeapId, args: ArgValues, vm: &mut VM<'_>) -> CallResult {
    dispatch(self_id, ops::CALL.to_owned(), args, vm)
}

impl<'h> PyTrait<'h> for HeapRead<'h, HostRef> {
    fn py_type(&self, _vm: &VM<'h>) -> Type {
        Type::HostRef
    }

    fn py_len(&self, _vm: &VM<'h>) -> Option<usize> {
        None
    }

    /// The identity hash every Python object without a `__hash__` of its own
    /// has, taken over the HOST's id rather than this proxy's heap entry:
    /// hashing the proxy would put two references to one object in different
    /// buckets while [`py_eq_impl`](PyTrait::py_eq_impl) calls them equal, and
    /// a set would then hold the same object twice.
    ///
    /// Asking the host for its own hash would need a suspension, which a hash
    /// is taken in too many places to be able to afford.
    fn py_hash(&self, _self_id: HeapId, vm: &mut VM<'h>) -> RunResult<Option<HashValue>> {
        Ok(Some(hash_one(self.get(vm.heap).id)))
    }

    /// Identity, for the same reason as [`py_hash`](PyTrait::py_hash) and
    /// consistent with it: two proxies are equal exactly when they name one
    /// host object, whether or not they are the same proxy.
    fn py_eq_impl(&self, other: &Value, vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        let Value::Ref(other_id) = other else {
            return Ok(Some(false));
        };
        let id = self.get(vm.heap).id;
        Ok(Some(match vm.heap.get(*other_id) {
            HeapData::HostRef(other_ref) => other_ref.id == id,
            _ => false,
        }))
    }

    /// Names the type rather than asking the host: a `repr` is taken in
    /// contexts that cannot suspend (an f-string, an error message). The id is
    /// omitted because it is an address in the host's own space.
    fn py_repr_fmt(&self, f: &mut impl Write, vm: &mut VM<'h>, _heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        write!(f, "<{} host object>", self.get(vm.heap).type_name).map_err(Into::into)
    }

    /// Every method call goes to the host, so a proxy needs no enumeration of
    /// what the object offers. A dunder does not: those are the sandbox's own
    /// vocabulary, and forwarding one would hand out `__class__` and
    /// everything reachable from it. `__await__` is the exception, because it
    /// is an operation on the reference rather than a way to name one.
    fn py_call_attr(
        &mut self,
        self_id: HeapId,
        vm: &mut VM<'h>,
        attr: &EitherStr,
        args: ArgValues,
    ) -> RunResult<CallResult> {
        let name = attr.as_str(vm.interns);
        if name == ops::AWAIT {
            return Ok(dispatch(self_id, ops::AWAIT.to_owned(), args, vm));
        }
        if name.starts_with('_') {
            let name = name.to_owned();
            let type_name = self.get(vm.heap).type_name.clone();
            args.drop_with(vm);
            return Err(ExcType::attribute_error(type_name, &name));
        }
        Ok(dispatch(self_id, attr.clone(), args, vm))
    }

    fn py_is_context_manager(&self, _vm: &VM<'h>) -> bool {
        true
    }

    fn py_enter(&mut self, self_id: HeapId, vm: &mut VM<'h>) -> RunResult<CallResult> {
        Ok(dispatch(self_id, ops::ENTER.to_owned(), ArgValues::Empty, vm))
    }

    fn py_exit(&mut self, self_id: HeapId, vm: &mut VM<'h>, exc: Option<HeapId>) -> RunResult<CallResult> {
        let exc = exc.map_or(Value::None, |id| {
            vm.heap.inc_ref(id);
            Value::Ref(id)
        });
        Ok(dispatch(self_id, ops::EXIT.to_owned(), ArgValues::One(exc), vm))
    }
}
