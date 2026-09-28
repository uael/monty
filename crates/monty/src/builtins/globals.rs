//! The `globals()` builtin.

use crate::{args::ArgValues, bytecode::VM, exception_private::RunResult, value::Value};

/// `globals()`: the global namespace the calling frame resolves names through.
///
/// Under an `exec()` / `eval()` globals dict this is that dict itself. Module
/// globals live in slots rather than a dict, so there it is a fresh snapshot of
/// the bound ones: writes to it never reach the module. See
/// `limitations/eval_exec.md`.
pub(crate) fn builtin_globals(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    args.check_zero_args("globals", vm.heap)?;
    vm.globals_dict()
}
