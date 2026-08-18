//! Implementation of the len() builtin function.

use crate::{
    args::ArgValues,
    bytecode::VM,
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunResult, SimpleException},
    heap::HeapData,
    types::{PyTrait, instance_len},
    value::Value,
};

/// Implementation of the len() builtin function.
///
/// Returns the length of an object (number of items in a container).
///
/// A user instance dispatches to its class's `__len__`, which re-enters the VM
/// and so cannot go through `PyTrait::py_len` (a `&VM` read).
pub fn builtin_len(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let value = args.get_one_arg("len", vm.heap)?;
    defer_drop!(value, vm);
    let len = match value {
        Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::Instance(_)) => instance_len(*id, vm)?,
        other => other.py_len(vm),
    };
    if let Some(len) = len {
        Ok(Value::Int(
            i64::try_from(len).map_err(|_| ExcType::overflow_c_ssize_t())?,
        ))
    } else {
        let type_name = value.py_type_name(vm);
        Err(SimpleException::new_msg(ExcType::TypeError, format!("object of type '{type_name}' has no len()")).into())
    }
}
