//! `contextvars.ContextVar` and the `Token` its `set()` hands back.
//!
//! Monty runs one implicit context, so a variable's current value lives on the
//! variable itself rather than in a `Context` mapping. `set()` records what the
//! variable held in a [`ContextVarToken`] and `reset()` puts it back, which is
//! all a single context makes observable; `limitations/contextvars.md` records
//! what a real one would add.

use std::{fmt::Write, mem};

use super::py_trait::PyObjectIdentity;
use crate::{
    args::{ArgValues, FromArgs},
    bytecode::{CallResult, VM},
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunResult, SimpleException},
    hash::{HashValue, identity_hash},
    heap::{DropWithContext, HeapData, HeapId, HeapItem, HeapObjectRead, HeapReadOutput},
    types::{LazyHeapSet, PyTrait, Type},
    value::{EitherStr, Value},
};

/// A `contextvars.ContextVar`: its name, the default `get()` falls back to, and
/// the value in force.
///
/// Every field is an owned reference, released by [`HeapItem::py_dec_ref_ids`].
/// `default` and `value` are `None` when the variable was created without a
/// default and while it is unset. The Python `None` cannot stand in for either:
/// `ContextVar('v', default=None).get()` returns `None` rather than raising.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct ContextVar {
    name: Value,
    default: Option<Value>,
    value: Option<Value>,
}

/// The token `ContextVar.set()` returns and `ContextVar.reset()` spends.
///
/// `var` and `old_value` are owned references. A token restores exactly once;
/// `used` is what makes a second `reset()` raise, as in CPython.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct ContextVarToken {
    var: Value,
    old_value: Option<Value>,
    used: bool,
}

/// Argument shape for `ContextVar(name, /, *, default=...)`.
///
/// `name` is positional-only and `default` keyword-only, so `ContextVar('v', 1)`
/// and `ContextVar(name='v')` both fail as they do in CPython. CPython parses
/// this one with `PyArg_ParseTupleAndKeywords` on `"O|$O:ContextVar"`, hence
/// `at_most_total` for the total pre-count (`ContextVar('v', default=1,
/// other=2)` reports three arguments against a maximum of two) and
/// `at_most_positional` for the overflow wording the `|` in that format picks.
#[derive(FromArgs)]
#[from_args(name = "ContextVar", style = c_named, at_most_total, at_most_positional)]
struct ContextVarArgs {
    #[from_args(pos_only)]
    name: Value,
    #[from_args(kw_only, default)]
    default: Option<Value>,
}

impl ContextVar {
    /// Builds a `ContextVar` from a `contextvars.ContextVar(...)` call.
    pub(crate) fn init(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
        let ContextVarArgs { name, default } = ContextVarArgs::from_args(args, vm)?;
        if name.to_str(vm).is_err() {
            name.drop_with(vm);
            default.drop_with(vm);
            return Err(ExcType::type_error("context variable name must be a str"));
        }
        Ok(vm
            .heap
            .allocate_as(Self {
                name,
                default,
                value: None,
            })
            .into_value())
    }

    /// The owned references, for the heap's GC child walker.
    pub(crate) fn owned_values(&self) -> [Option<&Value>; 3] {
        [Some(&self.name), self.default.as_ref(), self.value.as_ref()]
    }
}

impl ContextVarToken {
    /// The owned references, for the heap's GC child walker.
    pub(crate) fn owned_values(&self) -> [Option<&Value>; 2] {
        [Some(&self.var), self.old_value.as_ref()]
    }
}

impl<'h> PyTrait<'h> for HeapObjectRead<'h, ContextVar> {
    fn py_type(&self, _vm: &VM<'h>) -> Type {
        Type::ContextVar
    }

    fn py_len(&self, _vm: &VM<'h>) -> Option<usize> {
        None
    }

    fn py_eq_impl(&self, _other: &Value, _vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        // CPython's `ContextVar` defines no `__eq__`, so two variables of the
        // same name compare unequal; identity is resolved before this is reached.
        Ok(None)
    }

    fn py_hash(&self, _vm: &mut VM<'h>) -> RunResult<Option<HashValue>> {
        Ok(Some(identity_hash(self.id())))
    }

    fn py_repr_fmt(&self, f: &mut impl Write, vm: &mut VM<'h>, heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        // Cloned out first: recursing into `py_repr_fmt` needs the heap mutably.
        let (name, default) = {
            let this = self.get(vm.heap);
            (
                this.name.clone_with_heap(vm.heap),
                this.default.as_ref().map(|value| value.clone_with_heap(vm.heap)),
            )
        };
        defer_drop!(name, vm);
        defer_drop!(default, vm);
        f.write_str("<ContextVar name=")?;
        name.py_repr_fmt(f, vm, heap_ids)?;
        if let Some(default) = default {
            f.write_str(" default=")?;
            default.py_repr_fmt(f, vm, heap_ids)?;
        }
        Ok(write!(f, " at 0x{:x}>", self.py_identity().encoded())?)
    }

    fn py_getattr(&self, attr: &EitherStr, vm: &mut VM<'h>) -> RunResult<Option<CallResult>> {
        Ok(match attr.as_str(vm.interns) {
            "name" => Some(CallResult::Value(self.get(vm.heap).name.clone_with_heap(vm.heap))),
            _ => None,
        })
    }

    fn py_call_attr(&mut self, vm: &mut VM<'h>, attr: &EitherStr, args: ArgValues) -> RunResult<CallResult> {
        match attr.as_str(vm.interns) {
            "get" => self.get_value(vm, args),
            "set" => self.set_value(vm, args),
            "reset" => self.reset_value(vm, args),
            other => {
                let other = other.to_owned();
                args.drop_with(vm);
                Err(ExcType::attribute_error(Type::ContextVar, &other))
            }
        }
    }
}

impl<'h> HeapObjectRead<'h, ContextVar> {
    /// `var.get()` / `var.get(default)`: the value in force, else the call's own
    /// default, else the variable's, else `LookupError`.
    fn get_value(&self, vm: &mut VM<'h>, args: ArgValues) -> RunResult<CallResult> {
        let fallback = args.get_zero_one_arg("get", vm.heap)?;
        defer_drop!(fallback, vm);
        let held = self.get(vm.heap).value.as_ref().map(|v| v.clone_with_heap(vm.heap));
        let found = match (held, fallback) {
            (Some(value), _) => Some(value),
            (None, Some(fallback)) => Some(fallback.clone_with_heap(vm.heap)),
            (None, None) => self.get(vm.heap).default.as_ref().map(|v| v.clone_with_heap(vm.heap)),
        };
        if let Some(value) = found {
            Ok(CallResult::Value(value))
        } else {
            // CPython passes the variable itself as the exception's argument;
            // see `limitations/contextvars.md`.
            let repr = repr_of(&Value::Ref(self.id()), vm)?;
            Err(SimpleException::new_msg(ExcType::LookupError, repr).into())
        }
    }

    /// `var.set(value)`: binds `value` and hands back the token that undoes it.
    fn set_value(&mut self, vm: &mut VM<'h>, args: ArgValues) -> RunResult<CallResult> {
        let value = args.get_one_arg("ContextVar.set", vm.heap)?;
        let old_value = self.get_mut(vm.heap).value.replace(value);
        let id = self.id();
        // The token holds the variable alive for as long as the reset it owns.
        vm.heap.inc_ref(id);
        Ok(CallResult::Value(
            vm.heap
                .allocate_as(ContextVarToken {
                    var: Value::Ref(id),
                    old_value,
                    used: false,
                })
                .into_value(),
        ))
    }

    /// `var.reset(token)`: puts back what the `set()` that made `token` displaced.
    fn reset_value(&mut self, vm: &mut VM<'h>, args: ArgValues) -> RunResult<CallResult> {
        let token = args.get_one_arg("ContextVar.reset", vm.heap)?;
        defer_drop!(token, vm);
        let owner = match token {
            Value::Ref(id) => match vm.heap.get(*id) {
                HeapData::ContextVarToken(held) => match held.var {
                    Value::Ref(var_id) => Some((*id, var_id)),
                    _ => unreachable!("a token always holds its variable"),
                },
                _ => None,
            },
            _ => None,
        };
        let Some((token_id, owner)) = owner else {
            let repr = repr_of(token, vm)?;
            return Err(ExcType::type_error(format!(
                "expected an instance of Token, got {repr}"
            )));
        };
        if owner != self.id() {
            let repr = repr_of(token, vm)?;
            return Err(SimpleException::new_msg(
                ExcType::ValueError,
                format!("{repr} was created by a different ContextVar"),
            )
            .into());
        }
        spend_token(token_id, vm).map(CallResult::Value)
    }
}

/// Spends a token: marks it used and puts back what its `set()` displaced.
///
/// Reached from `ContextVar.reset(token)`, which has already checked that the
/// token belongs to the variable, and from `Token.__exit__`, where it cannot
/// belong to another.
fn spend_token(token_id: HeapId, vm: &mut VM<'_>) -> RunResult<Value> {
    let HeapReadOutput::ContextVarToken(mut token) = vm.heap.read(token_id) else {
        unreachable!("the caller matched a token")
    };
    if token.get(vm.heap).used {
        let repr = repr_of(&Value::Ref(token_id), vm)?;
        return Err(
            SimpleException::new_msg(ExcType::RuntimeError, format!("{repr} has already been used once")).into(),
        );
    }
    token.get_mut(vm.heap).used = true;
    let restored = token
        .get(vm.heap)
        .old_value
        .as_ref()
        .map(|value| value.clone_with_heap(vm.heap));
    let Value::Ref(var_id) = token.get(vm.heap).var else {
        unreachable!("a token always holds its variable")
    };
    let HeapReadOutput::ContextVar(mut var) = vm.heap.read(var_id) else {
        unreachable!("a token always holds its variable")
    };
    let replaced = mem::replace(&mut var.get_mut(vm.heap).value, restored);
    replaced.drop_with(vm);
    Ok(Value::None)
}

impl HeapItem for ContextVar {
    fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        self.name.py_dec_ref_ids(stack);
        if let Some(default) = &mut self.default {
            default.py_dec_ref_ids(stack);
        }
        if let Some(value) = &mut self.value {
            value.py_dec_ref_ids(stack);
        }
    }
}

impl<'h> PyTrait<'h> for HeapObjectRead<'h, ContextVarToken> {
    fn py_type(&self, _vm: &VM<'h>) -> Type {
        Type::ContextVarToken
    }

    fn py_len(&self, _vm: &VM<'h>) -> Option<usize> {
        None
    }

    fn py_eq_impl(&self, _other: &Value, _vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        Ok(None)
    }

    // CPython sets `Token.__hash__` to `None`, so a token is unhashable; the
    // inherited default says the same.

    fn py_repr_fmt(&self, f: &mut impl Write, vm: &mut VM<'h>, heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        let (var, used) = {
            let this = self.get(vm.heap);
            (this.var.clone_with_heap(vm.heap), this.used)
        };
        defer_drop!(var, vm);
        f.write_str(if used { "<Token used var=" } else { "<Token var=" })?;
        var.py_repr_fmt(f, vm, heap_ids)?;
        Ok(write!(f, " at 0x{:x}>", self.py_identity().encoded())?)
    }

    fn py_getattr(&self, attr: &EitherStr, vm: &mut VM<'h>) -> RunResult<Option<CallResult>> {
        Ok(match attr.as_str(vm.interns) {
            "var" => Some(CallResult::Value(self.get(vm.heap).var.clone_with_heap(vm.heap))),
            _ => None,
        })
    }

    fn py_is_context_manager(&self, _vm: &VM<'h>) -> bool {
        true
    }

    /// `with var.set(x) as token:` — CPython 3.14 made a token a context
    /// manager over the `set()` that made it.
    fn py_enter(&mut self, vm: &mut VM<'h>) -> RunResult<CallResult> {
        let id = self.id();
        vm.heap.inc_ref(id);
        Ok(CallResult::Value(Value::Ref(id)))
    }

    /// Leaving the block resets the variable, whether or not the body raised.
    /// A token already spent inside the block raises here, as in CPython.
    fn py_exit(&mut self, vm: &mut VM<'h>, _exc: Option<HeapId>) -> RunResult<CallResult> {
        spend_token(self.id(), vm).map(CallResult::Value)
    }
}

impl HeapItem for ContextVarToken {
    fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        self.var.py_dec_ref_ids(stack);
        if let Some(old_value) = &mut self.old_value {
            old_value.py_dec_ref_ids(stack);
        }
    }
}

/// A value's repr as a plain string, for the messages that name one.
fn repr_of(value: &Value, vm: &mut VM<'_>) -> RunResult<String> {
    let mut repr = String::new();
    value.py_repr_fmt(&mut repr, vm, &mut LazyHeapSet::default())?;
    Ok(repr)
}
