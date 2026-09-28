//! The `vars()` builtin.

use crate::{
    args::ArgValues,
    bytecode::VM,
    defer_drop,
    exception_private::{ExcType, RunError, RunResult, SimpleException},
    heap::{HeapData, HeapReadOutput},
    types::Dict,
    value::Value,
};

/// `vars(object=...)`: the `__dict__` of a module, a class or an instance.
///
/// With no argument this is `locals()`, as in CPython. With one it is a fresh
/// dict of that object's namespace rather than the namespace itself, so writes
/// to the result never reach the object, the way `globals()` and `locals()`
/// already answer here. See `limitations/builtins.md`.
pub(crate) fn builtin_vars(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let Some(value) = args.get_zero_one_arg("vars", vm.heap)? else {
        return vm.locals_dict();
    };
    defer_drop!(value, vm);
    let Value::Ref(id) = *value else {
        return Err(no_dict());
    };
    // The pairs come out first because building the copy needs the heap
    // mutably, which the read handle holds.
    let pairs = match vm.heap.read(id) {
        HeapReadOutput::Module(module) => held(module.get(vm.heap).attrs(), vm),
        HeapReadOutput::Class(class) => held(class.get(vm.heap).namespace(), vm),
        HeapReadOutput::Instance(instance) => held(instance.get(vm.heap).attrs(), vm),
        _ => return Err(no_dict()),
    };
    // `from_pairs` releases the pairs itself on every path that fails.
    let made = Dict::from_pairs(pairs, vm)?;
    let dict_id = vm.heap.allocate(HeapData::Dict(made));
    Ok(Value::Ref(dict_id))
}

/// Every pair of one namespace, cloned, ready to build a dict of its own.
fn held(namespace: &Dict, vm: &VM<'_>) -> Vec<(Value, Value)> {
    namespace
        .iter()
        .map(|(key, one)| (key.clone_with_heap(vm), one.clone_with_heap(vm)))
        .collect()
}

/// What `vars()` raises for an object that carries no namespace.
///
/// CPython names no type in it, so neither does this.
fn no_dict() -> RunError {
    SimpleException::new_msg(
        ExcType::TypeError,
        "vars() argument must have __dict__ attribute".to_owned(),
    )
    .into()
}
