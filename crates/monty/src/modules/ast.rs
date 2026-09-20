//! The `ast` module, which here is one constant and nothing else.
//!
//! `PyCF_ALLOW_TOP_LEVEL_AWAIT` is the flag a REPL passes to `compile()` so a
//! snippet that awaits at its top level compiles as a body that may, and
//! `eval()` hands back a coroutine to drive rather than running it where it
//! stands. Nothing else of `ast` is here: Monty parses with ruff and exposes no
//! syntax tree, so the node classes, `parse()` and `unparse()` would have
//! nothing behind them. See `limitations/ast.md`.

use crate::{bytecode::VM, heap::HeapId, intern::StaticStrings, types::Module, value::Value};

/// CPython's `ast.PyCF_ALLOW_TOP_LEVEL_AWAIT`, whose value is part of its
/// public API: a program writes the flag rather than the number, but a program
/// that writes the number must get the same answer.
pub const PY_CF_ALLOW_TOP_LEVEL_AWAIT: i64 = 0x2000;

/// Creates the `ast` module and allocates it on the heap.
pub fn create_module(vm: &mut VM<'_>) -> HeapId {
    let mut module = Module::new(StaticStrings::Ast, vm.interns);
    module.set_attr(
        StaticStrings::PyCfAllowTopLevelAwait,
        Value::Int(PY_CF_ALLOW_TOP_LEVEL_AWAIT),
        vm,
    );
    vm.heap.allocate_as(module).into_id()
}
