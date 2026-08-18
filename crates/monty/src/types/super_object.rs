//! `super()`: attribute lookup that skips ahead in the base chain.
//!
//! Monty implements only the zero-argument form, which is the one Python code
//! actually writes. CPython resolves it from the compiler-injected `__class__`
//! cell plus the method's first argument; Monty has no such cell, so the
//! defining class is recovered by finding which class in the receiver's chain
//! binds the function that is currently running (see [`defining_class`]). That
//! agrees with CPython whenever a function object is bound in at most one class
//! of the chain, which a `class` statement always produces.

use std::fmt::Write;

use super::{LazyHeapSet, PyTrait, Type};
use crate::{
    args::ArgValues,
    bytecode::{CallResult, VM},
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunResult},
    hash::{HashValue, identity_hash},
    heap::{DropWithContext, HeapData, HeapId, HeapItem, HeapRead, HeapReadOutput},
    intern::FunctionId,
    types::{
        allocate_string, allocate_tuple,
        class::{MAX_MRO_DEPTH, class_base_id, class_exc_base},
        instance::{class_name, instance_class_id, instance_super_lookup},
    },
    value::{EitherStr, Value},
};

/// The proxy `super()` returns: a receiver plus the point in its class chain
/// that attribute lookup resumes from.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct SuperObject {
    /// The bound receiver (`self` in the calling method); owned.
    instance: Value,
    /// The class whose *bases* the lookup starts from, i.e. the class that
    /// defines the running method.
    start_class: HeapId,
}

impl SuperObject {
    /// Builds the proxy; `instance` is taken by ownership.
    #[must_use]
    pub fn new(instance: Value, start_class: HeapId) -> Self {
        Self { instance, start_class }
    }

    /// Calls `on_child` for every heap value this proxy reaches, for the cycle
    /// collector's trial deletion.
    pub fn for_each_child(&self, mut on_child: impl FnMut(HeapId)) {
        if let Value::Ref(id) = self.instance {
            on_child(id);
        }
        on_child(self.start_class);
    }
}

impl HeapItem for SuperObject {
    fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        self.instance.py_dec_ref_ids(stack);
        stack.push(self.start_class);
    }
}

impl<'h> PyTrait<'h> for HeapRead<'h, SuperObject> {
    fn py_type(&self, _vm: &VM<'h>) -> Type {
        Type::Super
    }

    fn py_len(&self, _vm: &VM<'h>) -> Option<usize> {
        None
    }

    fn py_eq_impl(&self, _other: &Value, _vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        Ok(None)
    }

    fn py_hash(&self, self_id: HeapId, _vm: &mut VM<'h>) -> RunResult<Option<HashValue>> {
        Ok(Some(identity_hash(self_id)))
    }

    fn py_repr_fmt(&self, f: &mut impl Write, vm: &mut VM<'h>, _heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        let name = class_name(self.get(vm.heap).start_class, vm.heap, vm.interns).into_owned();
        Ok(write!(f, "<super: <class '{name}'>>")?)
    }

    fn py_getattr(&self, attr: &EitherStr, vm: &mut VM<'h>) -> RunResult<Option<CallResult>> {
        let (instance_id, start_class) = self.target(vm)?;
        let attr_str = attr.as_str(vm.interns);
        match instance_super_lookup(instance_id, start_class, attr_str, vm)? {
            Some(value) => Ok(Some(CallResult::Value(value))),
            None => Err(ExcType::attribute_error("super", attr_str)),
        }
    }

    fn py_call_attr(
        &mut self,
        _self_id: HeapId,
        vm: &mut VM<'h>,
        attr: &EitherStr,
        args: ArgValues,
    ) -> RunResult<CallResult> {
        let (instance_id, start_class) = match self.target(vm) {
            Ok(target) => target,
            Err(e) => {
                args.drop_with(vm);
                return Err(e);
            }
        };
        let attr_str = attr.as_str(vm.interns);
        let bound = match instance_super_lookup(instance_id, start_class, attr_str, vm) {
            Ok(bound) => bound,
            Err(e) => {
                args.drop_with(vm);
                return Err(e);
            }
        };
        match bound {
            Some(callable) => {
                defer_drop!(callable, vm);
                vm.call_function(callable, args)
            }
            // Nothing in the remaining chain defines it, so the implicit root
            // class answers: `BaseException` for an exception class, `object`
            // otherwise. Only the initializer has behaviour worth inheriting.
            None if attr_str == "__init__" => root_init(instance_id, start_class, args, vm),
            None => {
                args.drop_with(vm);
                Err(ExcType::attribute_error("super", attr_str))
            }
        }
    }
}

impl<'h> HeapRead<'h, SuperObject> {
    /// The receiver's heap id and the class to resume the walk from.
    ///
    /// A `super()` proxy is only ever built around a real instance, so a
    /// receiver that is not one means the proxy was smuggled in through a
    /// crafted snapshot rather than created by `super()`.
    fn target(&self, vm: &VM<'h>) -> RunResult<(HeapId, HeapId)> {
        let this = self.get(vm.heap);
        match this.instance {
            Value::Ref(id) if matches!(vm.heap.get(id), HeapData::Instance(_)) => Ok((id, this.start_class)),
            _ => Err(ExcType::type_error("super(): __class__ cell not found")),
        }
    }
}

/// `BaseException.__init__` / `object.__init__`, reached when no class left in
/// the chain defines `__init__`.
///
/// The exception form stores its arguments as `self.args`, exactly as
/// `BaseException.__init__` does, which is what makes
/// `super().__init__(message)` inside a user exception work. The object form
/// takes no arguments.
fn root_init(instance_id: HeapId, start_class: HeapId, args: ArgValues, vm: &mut VM<'_>) -> RunResult<CallResult> {
    if class_exc_base(start_class, vm).is_none() {
        return if matches!(args, ArgValues::Empty) {
            Ok(CallResult::Value(Value::None))
        } else {
            args.drop_with(vm);
            Err(ExcType::type_error(
                "object.__init__() takes exactly one argument (the instance to initialize)",
            ))
        };
    }
    let (pos, kwargs) = args.into_parts();
    if !kwargs.is_empty() {
        pos.drop_with(vm);
        kwargs.drop_with(vm);
        return Err(ExcType::type_error_no_kwargs("BaseException"));
    }
    let arg_values = pos.collect();
    set_exception_args(instance_id, arg_values, vm)?;
    Ok(CallResult::Value(Value::None))
}

/// Stores `args` as the exception instance's `args` tuple, taking ownership.
///
/// `BaseException.__new__` does this for every exception, so the attribute is
/// present from construction and a custom `__init__` that never calls `super()`
/// still leaves the constructor arguments visible.
pub(crate) fn set_exception_args(instance_id: HeapId, args: Vec<Value>, vm: &mut VM<'_>) -> RunResult<()> {
    let tuple = allocate_tuple(args.into_iter().collect(), vm.heap);
    let name = allocate_string("args", vm.heap);
    let HeapReadOutput::Instance(mut inst) = vm.heap.read(instance_id) else {
        name.drop_with(vm);
        tuple.drop_with(vm);
        return Err(ExcType::type_error("BaseException.__init__ called on a non-instance"));
    };
    let previous = inst.set_attr(name, tuple, vm)?;
    previous.drop_with(vm);
    Ok(())
}

/// The class in `instance`'s chain that defines the function currently running,
/// i.e. the class `super()` should resume the lookup *after*.
///
/// Falls back to the receiver's own class when the running function is not
/// bound in the chain (a nested function, or a method reached through a value
/// that is not in the class namespace), which makes `super()` there behave as
/// if the method were defined on the most derived class.
pub(crate) fn defining_class(instance_id: HeapId, func_id: FunctionId, vm: &VM<'_>) -> Option<HeapId> {
    let class_id = instance_class_id(instance_id, vm)?;
    let mut current = Some(class_id);
    for _ in 0..MAX_MRO_DEPTH {
        let id = current?;
        if let HeapData::Class(class) = vm.heap.get(id)
            && class
                .namespace()
                .iter()
                .any(|(_, value)| function_id_of(value, vm) == Some(func_id))
        {
            return Some(id);
        }
        current = class_base_id(id, vm);
    }
    Some(class_id)
}

/// The `FunctionId` a class-namespace member wraps, seeing through the
/// descriptor wrappers a method can be stored behind.
fn function_id_of(value: &Value, vm: &VM<'_>) -> Option<FunctionId> {
    match value {
        Value::DefFunction(id) => Some(*id),
        Value::Ref(id) => match vm.heap.get(*id) {
            HeapData::Closure(closure) => Some(closure.func_id),
            HeapData::FunctionDefaults(fd) => Some(fd.func_id),
            HeapData::MethodDescriptor(md) => function_id_of(&md.func, vm),
            HeapData::Property(property) => function_id_of(&property.fget, vm),
            _ => None,
        },
        _ => None,
    }
}
