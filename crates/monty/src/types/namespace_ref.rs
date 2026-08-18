//! A global namespace the sandbox holds as a value.
//!
//! The host has been able to create, copy, select and release namespaces
//! since they existed; this is the same surface for code running inside,
//! wearing a mapping's clothes. A [`NamespaceRef`] names one namespace, and
//! reading or binding through it touches the same slot vector the namespace's
//! own code resolves globals against, so a value bound through the handle IS
//! the value the next snippet in that namespace reads: one heap, one slot map,
//! no marshalling and no copy.
//!
//! Names are program slots. A key resolves through the program's one name
//! map, so a handle can only bind a name some compiled code somewhere has
//! mentioned; binding an unheard-of name is refused loudly rather than given
//! a private side-table that the namespace's own compiled loads would never
//! consult. An embedder that must bind a name before any code mentions it
//! compiles one mention first.
//!
//! The handle owns its namespace the way a list owns its items: when the last
//! reference to the handle drops, the namespace is released. The release
//! cannot happen inside the heap (dropping runs with no reach into the
//! namespace collection), so it is queued on the heap and swept where both
//! live together: the VM's instruction-counted checkpoint and its exits, and
//! the REPL after a host-side release; see `Heap::condemn_scope`.

use std::{fmt::Write, mem};

use crate::{
    args::ArgValues,
    bytecode::{CallResult, VM},
    deepcopy::deepcopy_namespace,
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunError, RunResult, SimpleException},
    hash::HashValue,
    heap::{ContainsHeap, DropWithContext, HeapData, HeapId, HeapRead},
    intern::StringId,
    namespace::{NamespaceId, ScopeId},
    types::{LazyHeapSet, List, PyTrait, Type, tuple::allocate_tuple},
    value::{EitherStr, Value},
};

/// A reference to one of the session's global namespaces.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct NamespaceRef {
    /// Which namespace this handle names.
    scope: ScopeId,
    /// Whether the last handle dropping releases the namespace.
    ///
    /// False for a handle onto a namespace that was already there and outlives
    /// it, which is what `namespace.current()` hands back.
    #[serde(default)]
    owns: bool,
}

impl NamespaceRef {
    /// Wraps a namespace the caller just created; the handle now owns it.
    pub(crate) fn new(scope: ScopeId) -> Self {
        Self { scope, owns: true }
    }

    /// Wraps a namespace that outlives the handle; see [`NamespaceRef::owns`].
    pub(crate) fn borrowed(scope: ScopeId) -> Self {
        Self { scope, owns: false }
    }

    /// Whether this handle's drop condemns the namespace it names.
    pub(crate) fn owns(&self) -> bool {
        self.owns
    }

    /// The namespace this handle names.
    pub(crate) fn scope(&self) -> ScopeId {
        self.scope
    }
}

/// The slot a key names, or `None` for a name no compiled code has mentioned.
///
/// A literal key arrives interned and resolves in one map lookup; a built
/// string (an f-string, a slice) goes through the interner's own reverse
/// lookup first.
///
/// # Errors
/// `TypeError` for a key that is not a string: namespace keys are names.
fn slot_of(key: &Value, vm: &VM<'_>) -> RunResult<Option<NamespaceId>> {
    if let Value::InternString(id) = key {
        return Ok(vm.names.get(*id));
    }
    let Some(name) = key.as_either_str(vm.heap) else {
        return Err(SimpleException::new_msg(
            ExcType::TypeError,
            format!(
                "namespace keys are names: got {}",
                key.py_type(vm).name(vm.heap, vm.interns)
            ),
        )
        .into());
    };
    let name = name.as_str(vm.interns).to_owned();
    Ok(vm.interns.get_string_id_by_name(&name).and_then(|id| vm.names.get(id)))
}

/// The values of `scope`, however the VM currently holds them.
///
/// # Errors
/// `RuntimeError` for a released namespace: the handle outlived what it named,
/// which a dump loaded into a smaller world can produce.
pub(crate) fn values_of<'a>(scope: ScopeId, vm: &'a VM<'_>) -> RunResult<&'a [Value]> {
    if scope == vm.scope {
        return Ok(&vm.globals);
    }
    vm.parked.get(scope).map(Vec::as_slice).ok_or_else(|| released(scope))
}

fn released(scope: ScopeId) -> RunError {
    SimpleException::new_msg(
        ExcType::RuntimeError,
        format!("namespace {} was released", scope.index()),
    )
    .into()
}

/// One bound value out of `scope`, counted.
pub(crate) fn read(scope: ScopeId, slot: NamespaceId, vm: &mut VM<'_>) -> RunResult<Option<Value>> {
    let values = values_of(scope, vm)?;
    Ok(match values.get(slot.index()) {
        None | Some(Value::Undefined) => None,
        Some(value) => Some(value.clone_with_heap(vm.heap)),
    })
}

/// Binds `value` at `slot` in `scope`, releasing what stood there.
fn write(scope: ScopeId, slot: NamespaceId, value: Value, vm: &mut VM<'_>) -> RunResult<()> {
    let index = slot.index();
    let was = if scope == vm.scope {
        if vm.globals.len() <= index {
            vm.globals.resize_with(index + 1, || Value::Undefined);
        }
        mem::replace(&mut vm.globals[index], value)
    } else if let Some(values) = vm.parked.get_mut(scope) {
        if values.len() <= index {
            values.resize_with(index + 1, || Value::Undefined);
        }
        mem::replace(&mut values[index], value)
    } else {
        value.drop_with(vm);
        return Err(released(scope));
    };
    was.drop_with(vm);
    Ok(())
}

/// Every bound `(slot, name)` of `scope`, in slot order.
///
/// Slot order is the order the program first met each name, which is the same
/// on every replay of the same record: iteration is deterministic.
pub(crate) fn bound(scope: ScopeId, vm: &VM<'_>) -> RunResult<Vec<(NamespaceId, StringId)>> {
    let values = values_of(scope, vm)?;
    Ok(vm
        .names
        .iter()
        .filter(|(slot, _)| !matches!(values.get(slot.index()), None | Some(Value::Undefined)))
        .collect())
}

/// `namespace()` / `namespace(source)`: a fresh namespace, empty or copied.
///
/// A namespace source is copied, exactly as the host-side `copy_namespace`:
/// the names are copied, the objects shared. A dict source binds each key,
/// under the same rule as any handle bind: a key must name a program slot.
///
/// # Errors
/// `RuntimeError` when the session has namespaces disabled or holds as many
/// as a handle can address; `TypeError` for any other source.
pub(crate) fn init(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    if !vm.heap.heap().namespaces_enabled() {
        args.drop_with(vm);
        return Err(SimpleException::new_msg(
            ExcType::RuntimeError,
            "namespaces are not enabled for this session".to_owned(),
        )
        .into());
    }
    let source = args.get_zero_one_arg("namespace", vm.heap)?;
    let mut values: Vec<Value> = Vec::new();
    if let Some(source) = source {
        let copied = match &source {
            Value::Ref(id) => match vm.heap.get(*id) {
                HeapData::Namespace(handle) => {
                    let from = handle.scope();
                    match values_of(from, vm) {
                        Ok(seen) => Some(seen.iter().map(|value| value.clone_with_heap(vm.heap)).collect()),
                        Err(e) => {
                            source.drop_with(vm);
                            return Err(e);
                        }
                    }
                }
                HeapData::Dict(pairs) => {
                    let pairs: Vec<(Value, Value)> = pairs
                        .into_iter()
                        .map(|(k, v)| (k.clone_with_heap(vm.heap), v.clone_with_heap(vm.heap)))
                        .collect();
                    let mut seen: Vec<Value> = Vec::new();
                    let mut failed = None;
                    for (key, value) in pairs {
                        if failed.is_some() {
                            key.drop_with(vm);
                            value.drop_with(vm);
                            continue;
                        }
                        match slot_of(&key, vm) {
                            Ok(Some(slot)) => {
                                let index = slot.index();
                                if seen.len() <= index {
                                    seen.resize_with(index + 1, || Value::Undefined);
                                }
                                let was = mem::replace(&mut seen[index], value);
                                was.drop_with(vm);
                            }
                            Ok(None) => {
                                let said = key
                                    .as_either_str(vm.heap)
                                    .map(|name| name.as_str(vm.interns).to_owned())
                                    .unwrap_or_default();
                                value.drop_with(vm);
                                failed = Some(SimpleException::new_msg(
                                    ExcType::KeyError,
                                    format!("no program slot for {said:?}: compile one mention of the name first"),
                                ));
                            }
                            Err(e) => {
                                value.drop_with(vm);
                                key.drop_with(vm);
                                source.drop_with(vm);
                                seen.drop_with(vm);
                                return Err(e);
                            }
                        }
                        key.drop_with(vm);
                    }
                    if let Some(failed) = failed {
                        source.drop_with(vm);
                        seen.drop_with(vm);
                        return Err(failed.into());
                    }
                    Some(seen)
                }
                _ => None,
            },
            _ => None,
        };
        let Some(copied) = copied else {
            let type_name = source.py_type(vm).name(vm.heap, vm.interns);
            source.drop_with(vm);
            return Err(SimpleException::new_msg(
                ExcType::TypeError,
                format!("namespace() takes a namespace or a dict of names, not {type_name}"),
            )
            .into());
        };
        source.drop_with(vm);
        values = copied;
    }
    hold(vm, values)
}

/// Puts `values` in a namespace of their own and hands back the handle that
/// owns it.
///
/// # Errors
/// `RuntimeError` when the session already holds as many namespaces as a
/// handle can address.
pub(crate) fn hold(vm: &mut VM<'_>, values: Vec<Value>) -> RunResult<Value> {
    let Some(scope) = vm.parked.create(vm.scope) else {
        values.drop_with(vm);
        return Err(SimpleException::new_msg(
            ExcType::RuntimeError,
            "this session already holds as many namespaces as a handle can address".to_owned(),
        )
        .into());
    };
    vm.parked.put_over(scope, values);
    Ok(Value::Ref(
        vm.heap.allocate(HeapData::Namespace(NamespaceRef::new(scope))),
    ))
}

/// `namespace.current()`: a handle to the namespace the caller runs in.
///
/// The handle does not own this one. A namespace a session is running in
/// outlives any handle to it, and releasing it out from under the code holding
/// the handle is not something a caller could recover from.
///
/// # Errors
/// `RuntimeError` when the session has namespaces disabled.
pub(crate) fn current(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    if !vm.heap.heap().namespaces_enabled() {
        args.drop_with(vm);
        return Err(SimpleException::new_msg(
            ExcType::RuntimeError,
            "namespaces are not enabled for this session".to_owned(),
        )
        .into());
    }
    args.check_zero_args("namespace.current", vm.heap)?;
    Ok(Value::Ref(
        vm.heap.allocate(HeapData::Namespace(NamespaceRef::borrowed(vm.scope))),
    ))
}

impl<'h> PyTrait<'h> for HeapRead<'h, NamespaceRef> {
    fn py_type(&self, _vm: &VM<'h>) -> Type {
        Type::Namespace
    }

    fn py_len(&self, vm: &VM<'h>) -> Option<usize> {
        let scope = self.get(vm.heap).scope;
        bound(scope, vm).ok().map(|names| names.len())
    }

    /// Unhashable, like the mapping it wears the clothes of.
    fn py_hash(&self, _self_id: HeapId, _vm: &mut VM<'h>) -> RunResult<Option<HashValue>> {
        Ok(None)
    }

    /// Two handles are equal exactly when they name one namespace: the handle
    /// is a reference, and what it references has no structural equality of
    /// its own.
    fn py_eq_impl(&self, other: &Value, vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        let Value::Ref(other_id) = other else {
            return Ok(Some(false));
        };
        let scope = self.get(vm.heap).scope;
        Ok(Some(match vm.heap.get(*other_id) {
            HeapData::Namespace(other_ref) => other_ref.scope == scope,
            _ => false,
        }))
    }

    fn py_repr_fmt(&self, f: &mut impl Write, vm: &mut VM<'h>, _heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        write!(f, "<namespace {}>", self.get(vm.heap).scope.index()).map_err(Into::into)
    }

    fn py_getitem(&self, key: &Value, vm: &mut VM<'h>) -> RunResult<Value> {
        let scope = self.get(vm.heap).scope;
        match slot_of(key, vm)?
            .map(|slot| read(scope, slot, vm))
            .transpose()?
            .flatten()
        {
            Some(value) => Ok(value),
            None => Err(ExcType::key_error(key, vm)),
        }
    }

    fn py_setitem(&mut self, key: Value, value: Value, vm: &mut VM<'h>) -> RunResult<()> {
        let scope = self.get(vm.heap).scope;
        let slot = match slot_of(&key, vm) {
            Ok(Some(slot)) => slot,
            Ok(None) => {
                // A slot is minted by compiled code and the program is sealed
                // while it runs, so the refusal names the way out.
                let said = key
                    .as_either_str(vm.heap)
                    .map(|name| name.as_str(vm.interns).to_owned())
                    .unwrap_or_default();
                key.drop_with(vm);
                value.drop_with(vm);
                return Err(SimpleException::new_msg(
                    ExcType::KeyError,
                    format!("no program slot for {said:?}: compile one mention of the name first"),
                )
                .into());
            }
            Err(e) => {
                key.drop_with(vm);
                value.drop_with(vm);
                return Err(e);
            }
        };
        key.drop_with(vm);
        write(scope, slot, value, vm)
    }

    fn py_delitem(&mut self, key: Value, vm: &mut VM<'h>) -> RunResult<()> {
        let scope = self.get(vm.heap).scope;
        let slot = slot_of(&key, vm)?;
        let held = slot.map(|slot| read(scope, slot, vm)).transpose()?.flatten();
        if let (Some(slot), Some(held)) = (slot, held) {
            held.drop_with(vm);
            key.drop_with(vm);
            write(scope, slot, Value::Undefined, vm)
        } else {
            let missing = ExcType::key_error(&key, vm);
            key.drop_with(vm);
            Err(missing)
        }
    }

    fn py_contains_impl(&self, _self_id: HeapId, item: &Value, vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        let scope = self.get(vm.heap).scope;
        match slot_of(item, vm)? {
            None => Ok(Some(false)),
            Some(slot) => {
                let values = values_of(scope, vm)?;
                Ok(Some(!matches!(values.get(slot.index()), None | Some(Value::Undefined))))
            }
        }
    }

    /// Iterating a namespace yields its names, like a dict.
    fn py_iter(&self, _self_id: Option<HeapId>, vm: &mut VM<'h>) -> RunResult<Value> {
        let scope = self.get(vm.heap).scope;
        let keys: Vec<Value> = bound(scope, vm)?
            .into_iter()
            .map(|(_, name_id)| Value::InternString(name_id))
            .collect();
        let keys = Value::Ref(vm.heap.allocate(HeapData::List(List::new(keys))));
        defer_drop!(keys, vm);
        keys.py_iter(vm)
    }

    fn py_call_attr(
        &mut self,
        _self_id: HeapId,
        vm: &mut VM<'h>,
        attr: &EitherStr,
        args: ArgValues,
    ) -> RunResult<CallResult> {
        let scope = self.get(vm.heap).scope;
        match attr.as_str(vm.interns) {
            "get" => {
                let (key, default) = args.get_one_two_args("namespace.get", vm.heap)?;
                let held = slot_of(&key, vm)?
                    .map(|slot| read(scope, slot, vm))
                    .transpose()?
                    .flatten();
                key.drop_with(vm);
                Ok(CallResult::Value(match (held, default) {
                    (Some(value), fallback) => {
                        fallback.drop_with(vm);
                        value
                    }
                    (None, Some(fallback)) => fallback,
                    (None, None) => Value::None,
                }))
            }
            "copy" => {
                args.check_zero_args("namespace.copy", vm.heap)?;
                let values: Vec<Value> = values_of(scope, vm)?
                    .iter()
                    .map(|value| value.clone_with_heap(vm.heap))
                    .collect();
                Ok(CallResult::Value(hold(vm, values)?))
            }
            "deepcopy" => {
                args.check_zero_args("namespace.deepcopy", vm.heap)?;
                let made = deepcopy_namespace(vm, scope)?;
                Ok(CallResult::Value(Value::Ref(
                    vm.heap.allocate(HeapData::Namespace(NamespaceRef::new(made))),
                )))
            }
            "keys" => {
                args.check_zero_args("namespace.keys", vm.heap)?;
                let keys: Vec<Value> = bound(scope, vm)?
                    .into_iter()
                    .map(|(_, name_id)| Value::InternString(name_id))
                    .collect();
                Ok(CallResult::Value(Value::Ref(
                    vm.heap.allocate(HeapData::List(List::new(keys))),
                )))
            }
            "values" => {
                args.check_zero_args("namespace.values", vm.heap)?;
                let mut out = Vec::new();
                for (slot, _) in bound(scope, vm)? {
                    if let Some(value) = read(scope, slot, vm)? {
                        out.push(value);
                    }
                }
                Ok(CallResult::Value(Value::Ref(
                    vm.heap.allocate(HeapData::List(List::new(out))),
                )))
            }
            "items" => {
                args.check_zero_args("namespace.items", vm.heap)?;
                let mut out = Vec::new();
                for (slot, name_id) in bound(scope, vm)? {
                    if let Some(value) = read(scope, slot, vm)? {
                        let pair = [Value::InternString(name_id), value].into_iter().collect();
                        out.push(allocate_tuple(pair, vm.heap));
                    }
                }
                Ok(CallResult::Value(Value::Ref(
                    vm.heap.allocate(HeapData::List(List::new(out))),
                )))
            }
            _ => {
                args.drop_with(vm);
                Err(ExcType::attribute_error("namespace", attr.as_str(vm.interns)))
            }
        }
    }
}
