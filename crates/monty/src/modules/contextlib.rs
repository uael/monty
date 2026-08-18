//! Implementation of the `contextlib` module.
//!
//! `suppress` (a real context manager, see [`crate::types::suppress`]) and
//! `AbstractContextManager` (a name to inherit from, no behaviour of its own).
//!
//! `contextmanager`, `ExitStack`, `closing`, `redirect_stdout` and the async
//! halves are absent — see `limitations/contextlib.md`.

use std::fmt;

use crate::{
    args::ArgValues,
    builtins::Builtins,
    bytecode::VM,
    exception_private::RunResult,
    heap::{DropWithContext, HeapData, HeapId},
    intern::StaticStrings,
    types::{Module, NativeClass, Type},
    value::Value,
};

/// The methods `AbstractContextManager` contributes to a class that names it as
/// a base, which is the whole of what CPython's own definition provides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(crate) enum ContextlibFunctions {
    /// `AbstractContextManager.__enter__`, which returns the receiver.
    ContextManagerEnter,
    /// `AbstractContextManager.__exit__`, which returns `None` so the
    /// exception it was handed keeps propagating.
    ContextManagerExit,
}

impl fmt::Display for ContextlibFunctions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ContextManagerEnter => "__enter__",
            Self::ContextManagerExit => "__exit__",
        })
    }
}

/// Dispatches an inherited `AbstractContextManager` method.
pub(super) fn call(vm: &mut VM<'_>, func: ContextlibFunctions, args: ArgValues) -> RunResult<Value> {
    match func {
        // Bound like any other method, so the receiver arrives as the only
        // argument and is what `with ... as` must bind.
        ContextlibFunctions::ContextManagerEnter => args.get_one_arg("__enter__", vm.heap),
        // `(self, exc_type, exc, tb)`; nothing is read, and dropping them is
        // the whole body.
        ContextlibFunctions::ContextManagerExit => {
            args.drop_with(vm);
            Ok(Value::None)
        }
    }
}

/// Creates the `contextlib` module on the heap.
///
/// # Panics
/// Panics if the required strings have not been pre-interned during prepare phase.
pub fn create_module(vm: &mut VM<'_>) -> HeapId {
    let mut module = Module::new(StaticStrings::Contextlib);
    module.set_attr(
        StaticStrings::Suppress,
        Value::Builtin(Builtins::Type(Type::Suppress)),
        vm,
    );
    // A native class rather than a marker: a class statement may name it as a
    // base, and it answers `isinstance` for anything with the two methods.
    module.set_attr(
        StaticStrings::AbstractContextManager,
        Value::Builtin(Builtins::Type(Type::Native(NativeClass::AbstractContextManager))),
        vm,
    );
    vm.heap.allocate(HeapData::Module(Box::new(module)))
}
