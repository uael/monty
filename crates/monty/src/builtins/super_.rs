//! Implementation of the super() builtin function.

use crate::{
    args::ArgValues,
    bytecode::VM,
    exception_private::{ExcType, ExcTypeExt, RunResult},
    heap::{DropWithContext, HeapData},
    types::{SuperObject, super_object::defining_class},
    value::Value,
};

/// Implementation of the zero-argument `super()` builtin.
///
/// The two-argument form `super(C, obj)` is not supported: Monty resolves the
/// starting class from the running frame instead (see `types::super_object`),
/// so there is nothing for an explicit class argument to select. Outside a
/// method (or in one whose first local is not an instance), it raises the same
/// `RuntimeError` CPython raises for a missing `__class__` cell.
pub fn builtin_super(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    if !matches!(args, ArgValues::Empty) {
        args.drop_with(vm);
        return Err(ExcType::not_implemented("super() with arguments is not supported").into());
    }
    let Some((func_id, receiver)) = vm.zero_arg_super_context() else {
        return Err(ExcType::runtime_error_no_super_arguments());
    };
    let &Value::Ref(instance_id) = receiver else {
        return Err(ExcType::runtime_error_no_super_arguments());
    };
    let Some(start_class) = defining_class(instance_id, func_id, vm) else {
        return Err(ExcType::runtime_error_no_super_arguments());
    };
    debug_assert!(
        matches!(vm.heap.get(instance_id), HeapData::Instance(_)),
        "defining_class only resolves for instances"
    );
    vm.heap.inc_ref(instance_id);
    vm.heap.inc_ref(start_class);
    let super_obj = SuperObject::new(Value::Ref(instance_id), start_class);
    Ok(Value::Ref(vm.heap.allocate(HeapData::Super(super_obj))))
}
