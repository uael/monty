//! Implementation of the `monty` module: what this interpreter does that
//! CPython does not, for the host that embeds it.
//!
//! `rebound(x, source, target, memo=None)` is `copy.deepcopy(x, memo)` with one
//! rule more: a function or a class made under the `exec()` / `eval()` globals
//! dict `source` is made again under `target`, an instance of such a class is
//! one of the class made again, and what a session has one of, a module, an
//! object of the host, is shared. It is how a host that runs code in a
//! namespace of its own takes a copy of what that code left which runs in
//! another namespace: CPython's deep copy shares functions and classes, so
//! the copy it gives stays bound to the namespace the originals were made in.
//!
//! `instance(cls, fields)` is an instance of a class of the session holding
//! these attributes, made as the interpreter makes one and with no `__init__`
//! run. It is how a host gives back an instance it took out with its fields,
//! since the session has no `__new__` to make a bare one with.

use crate::{
    args::{ArgValues, FromArgs},
    bytecode::VM,
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunResult},
    heap::{DropWithContext, HeapData, HeapId, HeapReadOutput},
    intern::StaticStrings,
    modules::{ModuleFunctions, copy},
    types::{Dict, Instance, Module},
    value::Value,
};

/// `monty` module functions, one variant per Python-visible function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::Display, serde::Serialize, serde::Deserialize)]
#[strum(serialize_all = "lowercase")]
pub(crate) enum MontyFunctions {
    Rebound,
    Instance,
}

/// Creates the `monty` module on the heap.
pub fn create_module(vm: &mut VM<'_>) -> HeapId {
    let mut module = Module::new(StaticStrings::Monty, vm.interns);
    module.set_attr(
        StaticStrings::Rebound,
        Value::ModuleFunction(ModuleFunctions::Monty(MontyFunctions::Rebound)),
        vm,
    );
    module.set_attr(
        StaticStrings::Instance,
        Value::ModuleFunction(ModuleFunctions::Monty(MontyFunctions::Instance)),
        vm,
    );
    vm.heap.allocate(HeapData::Module(Box::new(module)))
}

/// Dispatches a call to a `monty` module function.
pub(super) fn call(vm: &mut VM<'_>, function: MontyFunctions, args: ArgValues) -> RunResult<Value> {
    match function {
        MontyFunctions::Rebound => call_rebound(vm, args),
        MontyFunctions::Instance => call_instance(vm, args),
    }
}

/// `rebound(x, source, target, memo=None)`, bound as a pure-Python `def` binds.
#[derive(FromArgs)]
#[from_args(name = "rebound", style = def)]
struct ReboundArgs {
    x: Value,
    source: Value,
    target: Value,
    #[from_args(default = Value::None, static_string = "Memo")]
    memo: Value,
}

/// `monty.rebound(x, source, target, memo=None)`: a deep copy of `x` in which
/// what was made under `source` is made again under `target`.
fn call_rebound(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let ReboundArgs {
        x,
        source,
        target,
        memo,
    } = ReboundArgs::from_args(args, vm)?;
    defer_drop!(x, vm);
    defer_drop!(source, vm);
    defer_drop!(target, vm);
    let source = namespace(source, "rebound", "source", vm)?;
    let target = namespace(target, "rebound", "target", vm)?;
    copy::rebound(x, source, target, memo, vm)
}

/// `instance(cls, fields)`, bound as a pure-Python `def` binds.
#[derive(FromArgs)]
#[from_args(name = "instance", style = def)]
struct InstanceArgs {
    cls: Value,
    fields: Value,
}

/// `monty.instance(cls, fields)`: an instance of `cls`, a class of the
/// session, holding these attributes, with no `__init__` run.
fn call_instance(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let InstanceArgs { cls, fields } = InstanceArgs::from_args(args, vm)?;
    defer_drop!(cls, vm);
    defer_drop!(fields, vm);
    let class_id = match cls {
        Value::Ref(id) if matches!(vm.heap.read(*id), HeapReadOutput::Class(_)) => *id,
        other => {
            return Err(ExcType::type_error(format!(
                "instance() cls must be a class of the session, not {}",
                other.py_type_name(vm)
            )));
        }
    };
    let fields = namespace(fields, "instance", "fields", vm)?;
    let pairs = {
        let HeapReadOutput::Dict(dict) = vm.heap.read(fields) else {
            unreachable!("checked to be a dict")
        };
        dict.clone_all_pairs(vm)?
    };
    if let Some((bad, _)) = pairs.iter().find(|(key, _)| !key.is_str(vm.heap)) {
        let key_type = bad.py_type_name(vm);
        pairs.drop_with(vm);
        return Err(ExcType::type_error(format!(
            "instance() fields must be keyed by str, not {key_type}"
        )));
    }
    let attrs = Dict::from_pairs(pairs, vm)?;
    vm.heap.inc_ref(class_id);
    Ok(Value::Ref(vm.heap.allocate(HeapData::Instance(Box::new(
        Instance::new(class_id, attrs),
    )))))
}

/// The dict an argument of a function is, borrowed, or the `TypeError` for
/// what is no dict.
fn namespace(value: &Value, function: &str, which: &str, vm: &mut VM<'_>) -> RunResult<HeapId> {
    match value {
        Value::Ref(id) if matches!(vm.heap.read(*id), HeapReadOutput::Dict(_)) => Ok(*id),
        other => Err(ExcType::type_error(format!(
            "{function}() {which} must be a dict, not {}",
            other.py_type_name(vm)
        ))),
    }
}
