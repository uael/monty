//! `contextvars.ContextVar` and the `Token` its `set()` hands back.
//!
//! CPython keeps a variable's value in the *context*, a per-thread mapping that
//! `asyncio` copies into each task. Monty has one context and no way to make
//! another (`copy_context`, `Context.run` and `contextvars.Context` are absent),
//! so the value lives on the variable itself. Every observable difference that
//! follows is listed in `limitations/contextvars.md`.
//!
//! A `Token` owns a reference to its variable, which is what makes `reset()`
//! answerable without a context to look the variable up in.
//!
//! Both reprs end in the object's address, which [`PyTrait::py_repr_fmt`] has no
//! id to render, so they live in [`repr_fmt`] and are dispatched from
//! `Value::py_repr_fmt`'s `Ref` arm — the route `Instance` takes, for the same
//! reason.

use std::fmt::Write;

use crate::{
    args::{ArgValues, FromArgs},
    bytecode::{CallResult, VM},
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunError, RunResult, SimpleException},
    hash::{HashValue, identity_hash},
    heap::{DropWithContext, HeapData, HeapId, HeapItem, HeapRead, HeapReadOutput},
    types::{LazyHeapSet, PyTrait, Type, str::allocate_string},
    value::{EitherStr, Value},
};

/// A `contextvars.ContextVar`: a named slot with an optional default.
///
/// `default` and `value` are owned references, released by
/// [`HeapItem::py_dec_ref_ids`]. `value` is `None` when the variable has never
/// been set, or has been reset back past its first `set()` — the state that
/// makes a defaultless `get()` raise `LookupError`.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct ContextVar {
    /// `.name`, read-only as in CPython. Heap-owned when the name was built at
    /// runtime, since the intern table is frozen after prepare.
    name: EitherStr,
    /// The `default=` given at construction, if any.
    default: Option<Value>,
    /// The current value, or `None` when the variable is unset.
    value: Option<Value>,
}

/// The receipt `ContextVar.set()` returns, restoring the previous state.
///
/// Single-use, as in CPython: a second `reset()` with the same token raises
/// rather than silently re-applying a stale value.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct ContextToken {
    /// Owned reference to the `ContextVar` this token came from; `reset` refuses
    /// any other variable.
    var: Value,
    /// The value the variable held before the `set()`, or `None` for CPython's
    /// `Token.MISSING` — it was unset.
    old_value: Option<Value>,
    /// Whether `reset()` has already consumed this token.
    used: bool,
}

impl ContextVar {
    /// Runs `f` on every owned reference. Backs the GC child walker, and MUST
    /// report the same references as [`HeapItem::py_dec_ref_ids`].
    pub(crate) fn for_each_owned_value(&self, mut f: impl FnMut(&Value)) {
        if let Some(default) = &self.default {
            f(default);
        }
        if let Some(value) = &self.value {
            f(value);
        }
    }
}

impl ContextToken {
    /// Runs `f` on every owned reference; see [`ContextVar::for_each_owned_value`].
    pub(crate) fn for_each_owned_value(&self, mut f: impl FnMut(&Value)) {
        f(&self.var);
        if let Some(old) = &self.old_value {
            f(old);
        }
    }
}

/// Argument shape for `ContextVar(name, *, default=...)`.
///
/// CPython's `contextvar_tp_new` parses `"O|$O:ContextVar"` with an empty kwlist
/// entry for `name`, so the name is positional-only, `default` is keyword-only,
/// and arity counts positionals plus keywords together.
#[derive(FromArgs)]
#[from_args(name = "ContextVar", style = c_named, at_most_total)]
struct ContextVarArgs {
    #[from_args(pos_only)]
    name: Value,
    #[from_args(kw_only, default)]
    default: Option<Value>,
}

/// `contextvars.ContextVar(name, *, default=...)`.
pub(crate) fn init(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let ContextVarArgs { name, default } = ContextVarArgs::from_args(args, vm)?;
    let Some(interned) = name.as_either_str(vm.heap) else {
        name.drop_with(vm);
        default.drop_with(vm);
        return Err(ExcType::type_error("context variable name must be a str"));
    };
    name.drop_with(vm);
    Ok(Value::Ref(vm.heap.allocate(HeapData::ContextVar(ContextVar {
        name: interned,
        default,
        value: None,
    }))))
}

/// Writes the `repr` of the `ContextVar` or `Token` at `id`.
///
/// Takes the id rather than a read handle because both forms end in the object's
/// address, which `PyTrait` cannot supply.
///
/// # Panics
/// If `id` is neither, which only a corrupt dispatch can produce.
pub(crate) fn repr_fmt(id: HeapId, f: &mut impl Write, vm: &mut VM<'_>, heap_ids: &mut LazyHeapSet) -> RunResult<()> {
    match vm.heap.get(id) {
        HeapData::ContextVar(var) => {
            let name = var.name.as_str(vm.interns).to_owned();
            let default = var.default.as_ref().map(|d| d.clone_with_heap(vm.heap));
            write!(f, "<ContextVar name='{name}'")?;
            if let Some(default) = default {
                f.write_str(" default=")?;
                let written = default.py_repr_fmt(f, vm, heap_ids);
                default.drop_with(vm);
                written?;
            }
        }
        HeapData::ContextToken(token) => {
            let used = token.used;
            let var = token.var.clone_with_heap(vm.heap);
            f.write_str(if used { "<Token used var=" } else { "<Token var=" })?;
            let written = var.py_repr_fmt(f, vm, heap_ids);
            var.drop_with(vm);
            written?;
        }
        other => unreachable!("contextvars::repr_fmt on {}", other.py_type()),
    }
    // `HeapId::index` stands in for CPython's object address here as it does in
    // an instance's default repr, so `id()` and this agree.
    Ok(write!(f, " at 0x{:x}>", id.index())?)
}

/// [`repr_fmt`] into a fresh `String`, for the errors that quote an object.
fn repr_of(id: HeapId, vm: &mut VM<'_>) -> RunResult<String> {
    let mut rendered = String::new();
    let mut heap_ids = LazyHeapSet::default();
    repr_fmt(id, &mut rendered, vm, &mut heap_ids)?;
    Ok(rendered)
}

impl<'h> PyTrait<'h> for HeapRead<'h, ContextVar> {
    fn py_type(&self, _vm: &VM<'h>) -> Type {
        Type::ContextVar
    }

    fn py_len(&self, _vm: &VM<'h>) -> Option<usize> {
        None
    }

    fn py_eq_impl(&self, _other: &Value, _vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        // CPython defines no `__eq__`, so two same-named variables are distinct.
        Ok(None)
    }

    fn py_hash(&self, self_id: HeapId, _vm: &mut VM<'h>) -> RunResult<Option<HashValue>> {
        Ok(Some(identity_hash(self_id)))
    }

    fn py_getattr(&self, attr: &EitherStr, vm: &mut VM<'h>) -> RunResult<Option<CallResult>> {
        if attr.as_str(vm.interns) == "name" {
            let name = match &self.get(vm.heap).name {
                EitherStr::Interned(id) => Value::InternString(*id),
                EitherStr::Heap(name) => allocate_string(name.clone(), vm.heap),
            };
            Ok(Some(CallResult::Value(name)))
        } else {
            Ok(None)
        }
    }

    fn py_call_attr(
        &mut self,
        self_id: HeapId,
        vm: &mut VM<'h>,
        attr: &EitherStr,
        args: ArgValues,
    ) -> RunResult<CallResult> {
        match attr.as_str(vm.interns) {
            "get" => self.call_get(self_id, vm, args).map(CallResult::Value),
            "set" => self.call_set(self_id, vm, args).map(CallResult::Value),
            "reset" => self.call_reset(self_id, vm, args).map(CallResult::Value),
            other => {
                let other = other.to_owned();
                args.drop_with(vm);
                Err(ExcType::attribute_error(Type::ContextVar, &other))
            }
        }
    }
}

impl<'h> HeapRead<'h, ContextVar> {
    /// `ContextVar.get(default=?)` — the current value, else the argument, else
    /// the construction default, else `LookupError`.
    fn call_get(&self, self_id: HeapId, vm: &mut VM<'h>, args: ArgValues) -> RunResult<Value> {
        let fallback = args.get_zero_one_arg("get", vm.heap)?;
        if let Some(value) = &self.get(vm.heap).value {
            let value = value.clone_with_heap(vm.heap);
            fallback.drop_with(vm);
            return Ok(value);
        }
        if let Some(fallback) = fallback {
            return Ok(fallback);
        }
        if let Some(default) = &self.get(vm.heap).default {
            return Ok(default.clone_with_heap(vm.heap));
        }
        // CPython raises the variable's own repr as the message, so the error
        // names which variable was unset.
        Err(SimpleException::new_msg(ExcType::LookupError, repr_of(self_id, vm)?).into())
    }

    /// `ContextVar.set(value)` — installs `value` and returns the token that
    /// undoes it.
    fn call_set(&mut self, self_id: HeapId, vm: &mut VM<'h>, args: ArgValues) -> RunResult<Value> {
        let Some(value) = args.get_zero_one_arg("set", vm.heap)? else {
            return Err(ExcType::type_error(
                "ContextVar.set() takes exactly one argument (0 given)",
            ));
        };
        let old_value = self.get_mut(vm.heap).value.replace(value);
        // The token's back-reference is what lets `reset` verify the variable
        // without a context to resolve it through.
        vm.heap.inc_ref(self_id);
        Ok(Value::Ref(vm.heap.allocate(HeapData::ContextToken(ContextToken {
            var: Value::Ref(self_id),
            old_value,
            used: false,
        }))))
    }

    /// `ContextVar.reset(token)` — restores what the token recorded.
    fn call_reset(&mut self, self_id: HeapId, vm: &mut VM<'h>, args: ArgValues) -> RunResult<Value> {
        let Some(token) = args.get_zero_one_arg("reset", vm.heap)? else {
            return Err(ExcType::type_error(
                "ContextVar.reset() takes exactly one argument (0 given)",
            ));
        };
        defer_drop!(token, vm);
        let Value::Ref(token_id) = *token else {
            return Err(token_type_error(token, vm));
        };
        // `get` hands out no read handle, so nothing is held across the repr
        // calls or the mutation below.
        let HeapData::ContextToken(state) = vm.heap.get(token_id) else {
            return Err(token_type_error(token, vm));
        };
        let (used, same_var) = (state.used, matches!(state.var, Value::Ref(id) if id == self_id));
        if used {
            let rendered = repr_of(token_id, vm)?;
            return Err(ExcType::runtime_error(format!("{rendered} has already been used once")));
        }
        if !same_var {
            let rendered = repr_of(token_id, vm)?;
            return Err(ExcType::value_error(format!(
                "{rendered} was created by a different ContextVar"
            )));
        }

        let HeapReadOutput::ContextToken(mut token_read) = vm.heap.read(token_id) else {
            unreachable!("the variant was matched above")
        };
        let state = token_read.get_mut(vm.heap);
        state.used = true;
        let restored = state.old_value.take();
        drop(token_read);

        let replaced = match restored {
            Some(old) => self.get_mut(vm.heap).value.replace(old),
            None => self.get_mut(vm.heap).value.take(),
        };
        replaced.drop_with(vm);
        Ok(Value::None)
    }
}

/// CPython's `reset()` type check, which names the value it refused.
fn token_type_error(value: &Value, vm: &mut VM<'_>) -> RunError {
    let mut rendered = String::new();
    let mut heap_ids = LazyHeapSet::default();
    match value.py_repr_fmt(&mut rendered, vm, &mut heap_ids) {
        Ok(()) => ExcType::type_error(format!("expected an instance of Token, got {rendered}")),
        Err(error) => error,
    }
}

impl<'h> PyTrait<'h> for HeapRead<'h, ContextToken> {
    fn py_type(&self, _vm: &VM<'h>) -> Type {
        Type::ContextToken
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

    fn py_getattr(&self, attr: &EitherStr, vm: &mut VM<'h>) -> RunResult<Option<CallResult>> {
        match attr.as_str(vm.interns) {
            "var" => {
                let var = self.get(vm.heap).var.clone_with_heap(vm.heap);
                Ok(Some(CallResult::Value(var)))
            }
            // `Token.MISSING` is not exposed, so an unset old value reads as
            // `None`; see `limitations/contextvars.md`.
            "old_value" => {
                let old = self
                    .get(vm.heap)
                    .old_value
                    .as_ref()
                    .map_or(Value::None, |old| old.clone_with_heap(vm.heap));
                Ok(Some(CallResult::Value(old)))
            }
            _ => Ok(None),
        }
    }
}

impl HeapItem for ContextVar {
    fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        // Mirrors `for_each_owned_value`.
        if let Some(default) = &mut self.default {
            default.py_dec_ref_ids(stack);
        }
        if let Some(value) = &mut self.value {
            value.py_dec_ref_ids(stack);
        }
    }
}

impl HeapItem for ContextToken {
    fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        // Mirrors `for_each_owned_value`.
        self.var.py_dec_ref_ids(stack);
        if let Some(old) = &mut self.old_value {
            old.py_dec_ref_ids(stack);
        }
    }
}
