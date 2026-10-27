//! Runtime behaviour for `collections.defaultdict`.
//!
//! A defaultdict is a `dict` tagged with a defaultdict [`DictKind`](crate::types::DictKind),
//! carrying its `default_factory`. The only behavioural addition is the
//! missing-key hook below; everything else is the inherited dict surface.

use crate::{
    args::ArgValues,
    bytecode::{CallResult, VM},
    exception_private::{ExcType, ExcTypeExt, RunResult},
    frozen::FrozenFunction,
    heap::HeapObjectRead,
    types::Dict,
    value::Value,
};

/// `defaultdict.__missing__(key)` as CPython writes it, which runs on the VM's
/// own loop as a frame of its own, so a factory that calls the host suspends
/// the run.
pub(crate) const MISSING_SOURCE: &str = r"
def __monty_defaultdict_missing(mapping, key, factory):
    value = factory()
    mapping[key] = value
    return value
";

/// Implements `defaultdict.__missing__(key)` for a read on the VM's own loop:
/// `d[key]` and `d.__missing__(key)`.
///
/// With a `default_factory`, the factory runs in a frame of [`MISSING_SOURCE`],
/// which stores and gives its value; without one, this raises `KeyError`.
pub(crate) fn defaultdict_missing_on_loop<'h>(
    dict: &HeapObjectRead<'h, Dict>,
    key: Value,
    vm: &mut VM<'h>,
) -> RunResult<CallResult> {
    let factory = dict.get(vm.heap).default_factory().map(|f| f.clone_with_heap(vm.heap));
    let Some(factory) = factory else {
        let error = ExcType::key_error(&key, vm);
        key.drop_with(vm);
        return Err(error);
    };
    let id = dict.id();
    vm.heap.inc_ref(id);
    vm.call_frozen_exact(FrozenFunction::DefaultdictMissing, [Value::Ref(id), key, factory])
}

/// Implements `defaultdict.__missing__(key)` — the hook a missing-key access
/// invokes: if a `default_factory` is set, call it, insert `key -> factory()`,
/// and return the value; otherwise raise `KeyError`.
///
/// For a read that native code makes (`Value::py_getitem`), as `operator.getitem`
/// does. The factory runs via `VM::evaluate_function`, so a factory that
/// performs external/OS calls is unsupported there (documented); a read on the
/// VM's own loop goes through [`defaultdict_missing_on_loop`] instead.
pub(crate) fn defaultdict_missing<'h>(
    dict: &mut HeapObjectRead<'h, Dict>,
    key: &Value,
    vm: &mut VM<'h>,
) -> RunResult<Value> {
    let factory = dict.get(vm.heap).default_factory().map(|f| f.clone_with_heap(vm.heap));
    let Some(factory) = factory else {
        return Err(ExcType::key_error(key, vm));
    };
    let value = match vm.evaluate_function(&factory, ArgValues::Empty) {
        Ok(value) => value,
        Err(e) => {
            factory.drop_with(vm);
            return Err(e);
        }
    };
    factory.drop_with(vm);

    // Insert `key -> value` and return the same value (CPython stores and
    // returns the one object the factory produced).
    let key_clone = key.clone_with_heap(vm.heap);
    let value_ret = value.clone_with_heap(vm.heap);
    // A `set` failure (e.g. a key `__eq__` raising during collision) must
    // release the cloned `value_ret`, which the `?` early-return would leak.
    match dict.set(key_clone, value, vm) {
        Ok(Some(old)) => old.drop_with(vm),
        Ok(None) => {}
        Err(e) => {
            value_ret.drop_with(vm);
            return Err(e);
        }
    }
    Ok(value_ret)
}
