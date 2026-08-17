//! The `@dataclass(...)` keyword options: parsing them off the decorator call,
//! and the `__dataclass_params__` object a decorated class reports them in.
//!
//! The options themselves are [`DataclassOptions`], which lives in
//! [`crate::types::class`] because a `Class` stores them. Those bits are what a
//! class *acts* on, so overwriting `__dataclass_params__` afterwards reports
//! something else without changing behaviour, exactly as in CPython.

use std::fmt::Write;

use crate::{
    args::{ArgValues, FromArgs},
    bytecode::{CallResult, VM},
    exception_private::{ExcType, ExcTypeExt, RunResult},
    hash::{HashValue, identity_hash},
    heap::{DropWithContext, HeapId, HeapItem, HeapRead},
    types::{DataclassOptions, LazyHeapSet, Opt, PyTrait, Type},
    value::{EitherStr, Value},
};

impl DataclassOptions {
    /// Rejects an ordering that has no equality to build on.
    ///
    /// Checked where CPython checks it: after the per-field defaults, before
    /// the field order that would produce the `__init__` signature.
    pub(crate) fn validate_ordering(self) -> RunResult<()> {
        if self.get(Opt::Order) && !self.get(Opt::Eq) {
            return Err(ExcType::value_error("eq must be true if order is true"));
        }
        Ok(())
    }

    /// Rejects a weak-reference slot: CPython's own precondition first, then
    /// Monty's, since nothing here holds a weak reference and so there is no
    /// slot to add and no `__weakref__` to expose.
    pub(crate) fn validate_slots(self) -> RunResult<()> {
        if self.get(Opt::WeakrefSlot) && !self.get(Opt::Slots) {
            return Err(ExcType::type_error("weakref_slot is True but slots is False"));
        }
        if self.get(Opt::WeakrefSlot) {
            return Err(ExcType::not_implemented(
                "dataclass(weakref_slot=True) is not supported, Monty has no weak references",
            )
            .into());
        }
        Ok(())
    }
}

/// The `__dataclass_params__` object `@dataclass` writes into a class
/// namespace: CPython's `dataclasses._DataclassParams`.
///
/// A report of what the class was decorated with, never a control — the options
/// the class acts on live on the `Class` itself. Holds no heap references, so it
/// is the cheap half of a class's metadata; `__dataclass_fields__` owns the
/// captured defaults.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct DataclassParams {
    options: DataclassOptions,
}

impl DataclassParams {
    /// Wraps the options the decorator was called with.
    #[must_use]
    pub fn new(options: DataclassOptions) -> Self {
        Self { options }
    }
}

impl<'h> PyTrait<'h> for HeapRead<'h, DataclassParams> {
    fn py_type(&self, _vm: &VM<'h>) -> Type {
        Type::DataclassParams
    }

    fn py_len(&self, _vm: &VM<'h>) -> Option<usize> {
        None
    }

    fn py_eq_impl(&self, _other: &Value, _vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        // `_DataclassParams` defines no `__eq__`, so it compares by identity,
        // which `Value::py_eq_impl` resolves before ever reaching here.
        Ok(None)
    }

    fn py_hash(&self, self_id: HeapId, _vm: &mut VM<'h>) -> RunResult<Option<HashValue>> {
        Ok(Some(identity_hash(self_id)))
    }

    /// CPython's `_DataclassParams.__repr__`, flag for flag, every one of them
    /// the value the class was actually decorated with.
    fn py_repr_fmt(&self, f: &mut impl Write, vm: &mut VM<'h>, _heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        let options = self.get(vm.heap).options;
        f.write_str("_DataclassParams(")?;
        for (index, opt) in Opt::ALL.into_iter().enumerate() {
            if index > 0 {
                f.write_char(',')?;
            }
            write!(f, "{}={}", opt.keyword(), python_bool(options.get(opt)))?;
        }
        f.write_char(')')?;
        Ok(())
    }

    /// Every flag CPython exposes, read back off the stored options.
    fn py_getattr(&self, attr: &EitherStr, vm: &mut VM<'h>) -> RunResult<Option<CallResult>> {
        let attr_str = attr.as_str(vm.interns);
        let options = self.get(vm.heap).options;
        let Some(opt) = Opt::ALL.into_iter().find(|opt| opt.keyword() == attr_str) else {
            return Err(ExcType::attribute_error("_DataclassParams", attr_str));
        };
        Ok(Some(CallResult::Value(Value::Bool(options.get(opt)))))
    }
}

/// Python's spelling of a bool, for the flags in the repr above.
fn python_bool(value: bool) -> &'static str {
    if value { "True" } else { "False" }
}

impl HeapItem for DataclassParams {
    fn py_dec_ref_ids(&mut self, _stack: &mut Vec<HeapId>) {
        // A bit set, no heap references.
    }
}

/// Parses a `dataclass(...)` call into the class it was handed (when the bare
/// `@dataclass` form applied it directly) and the options it configured.
///
/// CPython's own signature is `dataclass(cls=None, /, *, ...)`, so a call with
/// no class simply leaves `cls` at `None` and returns the decorator; passing
/// `None` explicitly does the same there and here.
pub(crate) fn dataclass_options(vm: &mut VM<'_>, args: ArgValues) -> RunResult<(Option<Value>, DataclassOptions)> {
    let DataclassArgs {
        cls,
        init,
        repr,
        eq,
        order,
        unsafe_hash,
        frozen,
        match_args,
        kw_only,
        slots,
        weakref_slot,
    } = DataclassArgs::from_args(args, vm)?;
    // In `Opt::ALL` order, which is what the loop below zips against.
    let flags = vec![
        init,
        repr,
        eq,
        order,
        unsafe_hash,
        frozen,
        match_args,
        kw_only,
        slots,
        weakref_slot,
    ];
    let mut options = DataclassOptions::default();
    let mut failure = None;
    for (&opt, value) in Opt::ALL.iter().zip(flags.iter()) {
        match value.py_bool(vm) {
            Ok(on) => options.set(opt, on),
            // A `__bool__` that raises stops the parse; both the flags and the
            // class still need releasing, which the tail below does.
            Err(err) => {
                failure = Some(err);
                break;
            }
        }
    }
    flags.drop_with(vm);
    if let Some(err) = failure {
        cls.drop_with(vm);
        return Err(err);
    }
    let cls = if matches!(cls, Value::None) { None } else { Some(cls) };
    Ok((cls, options))
}

#[derive(FromArgs)]
#[from_args(name = "dataclass", style = def)]
struct DataclassArgs {
    #[from_args(pos_only, default = Value::None)]
    cls: Value,
    #[from_args(kw_only, default = Value::Bool(true))]
    init: Value,
    #[from_args(kw_only, default = Value::Bool(true))]
    repr: Value,
    #[from_args(kw_only, default = Value::Bool(true))]
    eq: Value,
    #[from_args(kw_only, default = Value::Bool(false))]
    order: Value,
    #[from_args(kw_only, default = Value::Bool(false))]
    unsafe_hash: Value,
    #[from_args(kw_only, default = Value::Bool(false))]
    frozen: Value,
    #[from_args(kw_only, default = Value::Bool(true))]
    match_args: Value,
    #[from_args(kw_only, default = Value::Bool(false))]
    kw_only: Value,
    #[from_args(kw_only, default = Value::Bool(false))]
    slots: Value,
    #[from_args(kw_only, default = Value::Bool(false))]
    weakref_slot: Value,
}
