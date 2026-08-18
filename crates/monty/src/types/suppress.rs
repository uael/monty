//! `contextlib.suppress`: a context manager that swallows the exceptions it was
//! built with.
//!
//! CPython's is a pure-Python class holding the constructor's argument tuple as
//! `_exceptions` and calling `issubclass` on it in `__exit__`. That ordering is
//! observable and reproduced here: the arguments are not validated at
//! construction, so `suppress(1)` builds fine and only raises if an exception
//! actually reaches `__exit__`.

use std::fmt::Write;

use crate::{
    args::ArgValues,
    bytecode::{CallResult, VM},
    exception_private::{ExcType, ExcTypeExt, RunResult},
    heap::{HeapData, HeapId, HeapItem, HeapRead},
    types::{PyTrait, Type},
    value::Value,
};

/// A `contextlib.suppress` instance.
///
/// `exceptions` holds the constructor's arguments verbatim, owned references
/// released by [`HeapItem::py_dec_ref_ids`]. They are whatever was passed, not
/// necessarily exception classes — see the module doc for why that matters.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Suppress {
    exceptions: Vec<Value>,
}

impl Suppress {
    /// Runs `f` on every owned reference. Backs the GC child walker, and MUST
    /// report the same references as [`HeapItem::py_dec_ref_ids`].
    pub(crate) fn for_each_owned_value(&self, mut f: impl FnMut(&Value)) {
        for exception in &self.exceptions {
            f(exception);
        }
    }
}

/// `contextlib.suppress(*exceptions)`.
///
/// Uses `into_pos_only` because the derive cannot express unbounded `*args`
/// with no keywords, as `itertools.chain` does. CPython's `__init__` is a
/// pure-Python `def` taking only `*exceptions`, so any keyword is rejected.
pub(crate) fn init(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let exceptions: Vec<Value> = args.into_pos_only("suppress.__init__", vm.heap)?.collect();
    Ok(Value::Ref(
        vm.heap.allocate(HeapData::Suppress(Suppress { exceptions })),
    ))
}

/// Writes the `repr` of the `suppress` at `id`.
///
/// CPython's is the default object repr, which carries both the defining
/// module and the address — neither available to `PyTrait::py_repr_fmt`, so
/// this is dispatched from `Value::py_repr_fmt`'s `Ref` arm as the contextvars
/// types are. `Type::Suppress` renders bare because that is what CPython's
/// `tp_name` says in error messages; the qualified form belongs here.
pub(crate) fn repr_fmt(id: HeapId, f: &mut impl Write) -> RunResult<()> {
    Ok(write!(f, "<contextlib.suppress object at 0x{:x}>", id.index())?)
}

impl<'h> PyTrait<'h> for HeapRead<'h, Suppress> {
    fn py_type(&self, _vm: &VM<'h>) -> Type {
        Type::Suppress
    }

    fn py_len(&self, _vm: &VM<'h>) -> Option<usize> {
        None
    }

    fn py_eq_impl(&self, _other: &Value, _vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        // A plain object with no `__eq__`, so identity decides, as in CPython.
        Ok(None)
    }

    fn py_is_context_manager(&self, _vm: &VM<'h>) -> bool {
        true
    }

    /// `suppress.__enter__` returns `None`, so `with suppress(...) as x` binds
    /// `None` rather than the manager.
    fn py_enter(&mut self, _self_id: HeapId, _vm: &mut VM<'h>) -> RunResult<CallResult> {
        Ok(CallResult::Value(Value::None))
    }

    /// Truthy exactly when a propagating exception is one of the suppressed
    /// classes, which is what makes the `with` block swallow it.
    fn py_exit(&mut self, _self_id: HeapId, vm: &mut VM<'h>, exc: Option<HeapId>) -> RunResult<CallResult> {
        // No exception: return early without validating `exceptions`, matching
        // CPython's `if exctype is None: return` before its `issubclass` call.
        let Some(exc) = exc else {
            return Ok(CallResult::Value(Value::None));
        };
        // `issubclass` stops at the first match, so a non-class *after* one is
        // never reached and never raises: `suppress(ValueError, 1)` swallows a
        // ValueError, while `suppress(1, ValueError)` raises. This is the one
        // place it differs from an `except` clause, which validates the whole
        // tuple (see `VM::check_exc_match`).
        for candidate in &self.get(vm.heap).exceptions {
            match vm.exc_id_matches_class(exc, candidate) {
                Some(true) => return Ok(CallResult::Value(Value::Bool(true))),
                Some(false) => {}
                None => {
                    return Err(ExcType::type_error(
                        "issubclass() arg 2 must be a class, a tuple of classes, or a union",
                    ));
                }
            }
        }
        Ok(CallResult::Value(Value::Bool(false)))
    }
}

impl HeapItem for Suppress {
    fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        // Mirrors `for_each_owned_value`.
        for exception in &mut self.exceptions {
            exception.py_dec_ref_ids(stack);
        }
    }
}
