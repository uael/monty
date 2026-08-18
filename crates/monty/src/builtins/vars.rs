//! Implementation of the `vars()` builtin.

use crate::{
    args::ArgValues,
    bytecode::VM,
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunError, RunResult, SimpleException},
    heap::{HeapData, HeapId},
    types::Dict,
    value::Value,
};

/// `vars(object)` — the object's `__dict__`.
///
/// Only a module has one in Monty, so this is how a program reads a module's
/// namespace as a `dict`. Everything else raises the `TypeError` CPython raises
/// for an object without a `__dict__`.
///
/// **The result is a copy**, not the live mapping: a module owns its namespace
/// inline, so there is no second reference to hand out. Writing to the result
/// therefore does not change the module. See `limitations/builtins.md`.
pub fn builtin_vars(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let Some(object) = args.get_zero_one_arg("vars", vm.heap)? else {
        // CPython's zero-argument form returns the calling frame's `locals()`,
        // which Monty cannot build; raising beats returning a misleading empty
        // dict.
        return Err(ExcType::not_implemented("vars() with no argument (locals() is not supported)").into());
    };
    defer_drop!(object, vm);
    let Value::Ref(id) = *object else {
        return Err(no_dict());
    };
    if !matches!(vm.heap.get(id), HeapData::Module(_)) {
        return Err(no_dict());
    }
    let namespace = copy_module_namespace(id, vm);
    Ok(Value::Ref(vm.heap.allocate(HeapData::Dict(namespace))))
}

/// Shallow-copies the module's attribute dict, taking a reference to each value.
fn copy_module_namespace(id: HeapId, vm: &mut VM<'_>) -> Dict {
    let HeapData::Module(module) = vm.heap.get(id) else {
        unreachable!("checked by the caller")
    };
    let pairs: Vec<(Value, Value)> = module
        .attrs()
        .iter()
        .map(|(key, value)| (key.clone_with_heap(vm.heap), value.clone_with_heap(vm.heap)))
        .collect();
    let mut namespace = Dict::new();
    for (key, value) in pairs {
        // Every key is a string a module attribute was set under, so hashing
        // cannot fail and no entry can collide with a different key.
        namespace
            .set(key, value, vm)
            .expect("module attribute names are hashable strings");
    }
    namespace
}

/// CPython's `vars()` rejection for an object with no `__dict__`.
fn no_dict() -> RunError {
    SimpleException::new_msg(ExcType::TypeError, "vars() argument must have __dict__ attribute").into()
}
