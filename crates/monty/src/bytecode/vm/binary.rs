//! Unary, binary and in-place operation helpers for the VM.

use super::VM;
use crate::{
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunError, RunResult},
    heap::DropGuard,
    types::PyTrait,
    value::{OpForm, Value},
};

impl VM<'_> {
    /// Unary minus (`-x`).
    pub(super) fn unary_neg(&mut self) -> Result<(), RunError> {
        self.unary_op(|value, vm| value.py_neg_impl(vm, value.ref_id()), "-")
    }

    /// Unary plus (`+x`) — an identity for numbers, but not for `bool`
    /// (`+True` is `1`) or a `Counter` (which drops its non-positive counts).
    pub(super) fn unary_pos(&mut self) -> Result<(), RunError> {
        self.unary_op(|value, vm| value.py_pos_impl(vm, value.ref_id()), "+")
    }

    /// Binary or augmented addition, the form deciding only what a failure names.
    pub(super) fn binary_add(&mut self, form: OpForm) -> Result<(), RunError> {
        self.binary_op(|lhs, rhs, vm| lhs.py_add(rhs, vm, form))
    }

    /// Binary or augmented subtraction.
    pub(super) fn binary_sub(&mut self, form: OpForm) -> Result<(), RunError> {
        self.binary_op(|lhs, rhs, vm| lhs.py_sub(rhs, vm, form))
    }

    /// Binary or augmented multiplication.
    pub(super) fn binary_mult(&mut self, form: OpForm) -> Result<(), RunError> {
        self.binary_op(|lhs, rhs, vm| lhs.py_mul(rhs, vm, form))
    }

    /// Binary or augmented division.
    pub(super) fn binary_div(&mut self, form: OpForm) -> Result<(), RunError> {
        self.binary_op(|lhs, rhs, vm| lhs.py_truediv(rhs, vm, form))
    }

    /// Binary or augmented floor division.
    pub(super) fn binary_floordiv(&mut self, form: OpForm) -> Result<(), RunError> {
        self.binary_op(|lhs, rhs, vm| lhs.py_floordiv(rhs, vm, form))
    }

    /// Binary or augmented modulo.
    pub(super) fn binary_mod(&mut self, form: OpForm) -> Result<(), RunError> {
        self.binary_op(|lhs, rhs, vm| lhs.py_mod(rhs, vm, form))
    }

    /// Binary or augmented power.
    #[inline(never)]
    pub(super) fn binary_pow(&mut self, form: OpForm) -> Result<(), RunError> {
        self.binary_op(|lhs, rhs, vm| lhs.py_pow(rhs, None, vm, form))
    }

    /// Binary or augmented `&`.
    pub(super) fn binary_and(&mut self, form: OpForm) -> Result<(), RunError> {
        self.binary_op(|lhs, rhs, vm| lhs.py_and(rhs, vm, form))
    }

    /// Binary or augmented `|`.
    pub(super) fn binary_or(&mut self, form: OpForm) -> Result<(), RunError> {
        self.binary_op(|lhs, rhs, vm| lhs.py_or(rhs, vm, form))
    }

    /// Binary or augmented `^`.
    pub(super) fn binary_xor(&mut self, form: OpForm) -> Result<(), RunError> {
        self.binary_op(|lhs, rhs, vm| lhs.py_xor(rhs, vm, form))
    }

    /// Binary or augmented left shift.
    pub(super) fn binary_lshift(&mut self, form: OpForm) -> Result<(), RunError> {
        self.binary_op(|lhs, rhs, vm| lhs.py_lshift(rhs, vm, form))
    }

    /// Binary or augmented right shift.
    pub(super) fn binary_rshift(&mut self, form: OpForm) -> Result<(), RunError> {
        self.binary_op(|lhs, rhs, vm| lhs.py_rshift(rhs, vm, form))
    }

    /// In-place addition.
    ///
    /// Mutable types (a list, a `Counter`, a `deque` whose `+=` is `extend`)
    /// mutate through `py_iadd_impl` and keep their identity, so an alias sees
    /// the update; immutable ones fall back to ordinary addition. Uses lazy type
    /// capture: only calls `py_type()` in error paths.
    pub(super) fn inplace_add(&mut self) -> Result<(), RunError> {
        self.inplace_op(
            |lhs, rhs, vm| lhs.py_iadd_impl(rhs, vm, lhs.ref_id()),
            |lhs, rhs, vm| lhs.py_add(rhs, vm, OpForm::Augmented),
        )
    }

    /// `-=` — an in-place mutation (a `Counter` subtracts counts), or binary `-`.
    pub(super) fn inplace_sub(&mut self) -> Result<(), RunError> {
        self.inplace_op(
            |lhs, rhs, vm| lhs.py_isub_impl(rhs, vm, lhs.ref_id()),
            |lhs, rhs, vm| lhs.py_sub(rhs, vm, OpForm::Augmented),
        )
    }

    /// `&=` — an in-place mutation (a `Counter` intersects counts), or binary `&`.
    pub(super) fn inplace_and(&mut self) -> Result<(), RunError> {
        self.inplace_op(
            |lhs, rhs, vm| lhs.py_iand_impl(rhs, vm, lhs.ref_id()),
            |lhs, rhs, vm| lhs.py_and(rhs, vm, OpForm::Augmented),
        )
    }

    /// `|=` — an in-place mutation (a `Counter` unions counts), or binary `|`.
    pub(super) fn inplace_or(&mut self) -> Result<(), RunError> {
        self.inplace_op(
            |lhs, rhs, vm| lhs.py_ior_impl(rhs, vm, lhs.ref_id()),
            |lhs, rhs, vm| lhs.py_or(rhs, vm, OpForm::Augmented),
        )
    }

    /// Binary matrix multiplication (`@` operator).
    pub(super) fn binary_matmul(&mut self) -> Result<(), RunError> {
        self.binary_op(|lhs, rhs, vm| lhs.py_matmul(rhs, vm))
    }

    /// Applies a unary operation while owning stack-value cleanup, raising
    /// CPython's `TypeError` (naming `symbol`) when the type has no such form.
    fn unary_op(
        &mut self,
        operation: impl FnOnce(&Value, &mut Self) -> RunResult<Option<Value>>,
        symbol: &str,
    ) -> Result<(), RunError> {
        let this = self;

        let value = this.pop();
        defer_drop!(value, this);

        if let Some(result) = operation(value, this)? {
            this.push(result);
            Ok(())
        } else {
            Err(ExcType::unary_type_error(symbol, &value.py_type_name(this)))
        }
    }

    /// Applies a binary operation while owning stack-value cleanup.
    fn binary_op(
        &mut self,
        operation: impl FnOnce(&Value, &Value, &mut Self) -> RunResult<Value>,
    ) -> Result<(), RunError> {
        let this = self;

        let rhs = this.pop();
        defer_drop!(rhs, this);
        let lhs = this.pop();
        defer_drop!(lhs, this);

        let result = operation(lhs, rhs, this)?;
        this.push(result);
        Ok(())
    }

    /// Applies an in-place operation, offering the left operand `inplace` (its
    /// `py_i*_impl` hook) first and otherwise falling back to `binary`.
    ///
    /// A hook reporting `true` has mutated the left operand, which is pushed
    /// back so it keeps its identity; `false` means the type has no in-place
    /// form and the ordinary binary operator produces a fresh value.
    fn inplace_op(
        &mut self,
        inplace: impl FnOnce(&mut Value, &Value, &mut Self) -> RunResult<bool>,
        binary: impl FnOnce(&Value, &Value, &mut Self) -> RunResult<Value>,
    ) -> Result<(), RunError> {
        let this = self;

        let rhs = this.pop();
        defer_drop!(rhs, this);
        // A `DropGuard` rather than `defer_drop!`: a successful in-place operation
        // reclaims the left operand to push it back onto the stack.
        let mut lhs_guard = DropGuard::new(this.pop(), this);
        let (lhs, this) = lhs_guard.as_parts_mut();

        if inplace(lhs, rhs, this)? {
            let (lhs, this) = lhs_guard.into_parts();
            this.push(lhs);
            Ok(())
        } else {
            let result = binary(lhs, rhs, this)?;
            this.push(result);
            Ok(())
        }
    }
}
