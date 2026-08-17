//! PEP 750 template strings: `string.templatelib.Template` and `Interpolation`.
//!
//! A `t"..."` literal evaluates to a [`Template`] holding two tuples: the literal
//! text around each replacement field, and one [`Interpolation`] per field.
//! Neither type is constructible from Python (see `limitations/string_templatelib.md`);
//! the VM builds them through [`allocate_template`] / [`allocate_interpolation`].

use std::fmt::Write;

use crate::{
    bytecode::{CallResult, VM},
    defer_drop,
    exception_private::RunResult,
    hash::{HashValue, identity_hash},
    heap::{HeapData, HeapId, HeapItem, HeapRead},
    intern::StaticStrings,
    types::{LazyHeapSet, PyTrait, TupleIterator, Type, allocate_tuple, tuple::TupleVec},
    value::{EitherStr, Value},
};

/// PEP 750 `string.templatelib.Template`: the value of a `t"..."` literal.
///
/// Both fields are owned references, released by [`HeapItem::py_dec_ref_ids`].
/// Iteration interleaves them and skips empty strings, so `list(t"{x}")` yields a
/// single `Interpolation`; there is no `len()`, matching CPython.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Template {
    /// Owned `Value::Ref` to a `Tuple` of `str`; always one longer than `interpolations`.
    strings: Value,
    /// Owned `Value::Ref` to a `Tuple` of `Interpolation`.
    interpolations: Value,
}

/// PEP 750 `string.templatelib.Interpolation`: one `{...}` field of a template.
///
/// All four fields are owned references, released by [`HeapItem::py_dec_ref_ids`].
/// `expression` is the field's source text, `conversion` is `'r'`/`'s'`/`'a'` or
/// `None`, and `format_spec` is `''` when the literal omitted it.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Interpolation {
    value: Value,
    expression: Value,
    conversion: Value,
    format_spec: Value,
}

/// Allocates a `Template`, taking ownership of both tuple values.
///
/// # Panics
/// If `strings` is not a `Tuple` of `str` exactly one longer than the `Tuple`
/// `interpolations`. PEP 750 guarantees that shape, so a mismatch is a bug in the
/// compiler rather than anything sandboxed code can provoke, and iteration below
/// reads both parts of the invariant as given.
pub(crate) fn allocate_template(strings: Value, interpolations: Value, vm: &mut VM<'_>) -> Value {
    let (strings_len, interpolations_len) = {
        let string_items = tuple_items(&strings, vm);
        for item in string_items {
            assert!(
                item.to_str(vm).is_ok(),
                "Template.strings must hold only str values, got {}",
                item.py_type_name(vm)
            );
        }
        (string_items.len(), tuple_items(&interpolations, vm).len())
    };
    assert_eq!(
        strings_len,
        interpolations_len + 1,
        "PEP 750 requires exactly one more string than interpolation"
    );
    Value::Ref(vm.heap.allocate(HeapData::Template(Template {
        strings,
        interpolations,
    })))
}

/// Allocates an `Interpolation`, taking ownership of all four values.
pub(crate) fn allocate_interpolation(
    value: Value,
    expression: Value,
    conversion: Value,
    format_spec: Value,
    vm: &mut VM<'_>,
) -> Value {
    Value::Ref(vm.heap.allocate(HeapData::Interpolation(Box::new(Interpolation {
        value,
        expression,
        conversion,
        format_spec,
    }))))
}

impl Template {
    /// The two owned tuple references, for the heap's GC child walker.
    pub(crate) fn owned_values(&self) -> [&Value; 2] {
        [&self.strings, &self.interpolations]
    }
}

impl Interpolation {
    /// The four owned field references, for the heap's GC child walker.
    pub(crate) fn owned_values(&self) -> [&Value; 4] {
        [&self.value, &self.expression, &self.conversion, &self.format_spec]
    }
}

impl<'h> PyTrait<'h> for HeapRead<'h, Template> {
    fn py_type(&self, _vm: &VM<'h>) -> Type {
        Type::Template
    }

    fn py_len(&self, _vm: &VM<'h>) -> Option<usize> {
        // CPython raises `TypeError: object of type
        // 'string.templatelib.Template' has no len()`; the counts are reached
        // through `.strings` / `.interpolations` instead.
        None
    }

    fn py_eq_impl(&self, _other: &Value, _vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        // CPython's `Template` defines no `__eq__`, so two templates with equal
        // parts compare unequal; identity is resolved before this is reached.
        Ok(None)
    }

    fn py_hash(&self, self_id: HeapId, _vm: &mut VM<'h>) -> RunResult<Option<HashValue>> {
        Ok(Some(identity_hash(self_id)))
    }

    fn py_repr_fmt(&self, f: &mut impl Write, vm: &mut VM<'h>, heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        let Ok(mut guard) = vm.recursion_guard() else {
            return Ok(f.write_str("...")?);
        };
        let vm = &mut *guard;
        // Cloned out first: recursing into `py_repr_fmt` needs the heap mutably.
        let parts = {
            let this = self.get(vm.heap);
            [
                this.strings.clone_with_heap(vm.heap),
                this.interpolations.clone_with_heap(vm.heap),
            ]
        };
        defer_drop!(parts, vm);
        f.write_str("Template(strings=")?;
        parts[0].py_repr_fmt(f, vm, heap_ids)?;
        f.write_str(", interpolations=")?;
        parts[1].py_repr_fmt(f, vm, heap_ids)?;
        Ok(f.write_char(')')?)
    }

    fn py_getattr(&self, attr: &EitherStr, vm: &mut VM<'h>) -> RunResult<Option<CallResult>> {
        // Fast path: interned names match by id, with no string comparison.
        if let Some(ss) = attr.static_string() {
            return Ok(match ss {
                StaticStrings::Strings => Some(self.get(vm.heap).strings.clone_with_heap(vm.heap)),
                StaticStrings::Interpolations => Some(self.get(vm.heap).interpolations.clone_with_heap(vm.heap)),
                StaticStrings::Values => Some(self.interpolation_values(vm)),
                _ => None,
            }
            .map(CallResult::Value));
        }
        // Slow path: a name built at runtime (`getattr(t, 'val' + 'ues')`) is a heap str.
        Ok(match attr.as_str(vm.interns) {
            "strings" => Some(self.get(vm.heap).strings.clone_with_heap(vm.heap)),
            "interpolations" => Some(self.get(vm.heap).interpolations.clone_with_heap(vm.heap)),
            "values" => Some(self.interpolation_values(vm)),
            _ => None,
        }
        .map(CallResult::Value))
    }

    fn py_is_iterable(&self, _vm: &VM<'h>) -> bool {
        true
    }

    fn py_iter(&self, _self_id: Option<HeapId>, vm: &mut VM<'h>) -> RunResult<Value> {
        let items = self.iteration_items(vm);
        let Value::Ref(items_id) = &items else {
            unreachable!("allocate_tuple always returns a Ref")
        };
        // `from_tuple` takes its own reference, so this function's is released.
        let iterator = TupleIterator::from_tuple(*items_id, vm);
        items.drop_with(vm);
        Ok(iterator)
    }
}

impl<'h> HeapRead<'h, Template> {
    /// Builds the `values` tuple: each interpolation's `value`, in order.
    ///
    /// Freshly built per access, as CPython's `Template.values` property is.
    fn interpolation_values(&self, vm: &VM<'h>) -> Value {
        let items = tuple_items(&self.get(vm.heap).interpolations, vm);
        let mut values = TupleVec::with_capacity(items.len());
        for item in items {
            let Value::Ref(id) = item else {
                unreachable!("Template.interpolations holds only Interpolation objects")
            };
            let HeapData::Interpolation(interpolation) = vm.heap.get(*id) else {
                unreachable!("Template.interpolations holds only Interpolation objects")
            };
            values.push(interpolation.value.clone_with_heap(vm.heap));
        }
        allocate_tuple(values, vm.heap)
    }

    /// Builds the tuple `__iter__` walks: strings and interpolations interleaved,
    /// with empty strings skipped (CPython yields no `''` items).
    ///
    /// Materializing it lets iteration reuse `TupleIterator` rather than needing a
    /// heap iterator type of its own. Its size is bounded by what the template
    /// already holds, so it needs no allocation preflight.
    fn iteration_items(&self, vm: &VM<'h>) -> Value {
        let strings = tuple_items(&self.get(vm.heap).strings, vm);
        let interpolations = tuple_items(&self.get(vm.heap).interpolations, vm);
        let mut items = TupleVec::with_capacity(strings.len() + interpolations.len());
        for (index, string) in strings.iter().enumerate() {
            // `allocate_template` validated these as `str`, so a failure is a Monty bug.
            let text = string.to_str(vm).expect("Template.strings holds only str values");
            if !text.is_empty() {
                items.push(string.clone_with_heap(vm.heap));
            }
            if let Some(interpolation) = interpolations.get(index) {
                items.push(interpolation.clone_with_heap(vm.heap));
            }
        }
        allocate_tuple(items, vm.heap)
    }
}

impl HeapItem for Template {
    fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        self.strings.py_dec_ref_ids(stack);
        self.interpolations.py_dec_ref_ids(stack);
    }
}

impl<'h> PyTrait<'h> for HeapRead<'h, Interpolation> {
    fn py_type(&self, _vm: &VM<'h>) -> Type {
        Type::Interpolation
    }

    fn py_len(&self, _vm: &VM<'h>) -> Option<usize> {
        None
    }

    fn py_eq_impl(&self, _other: &Value, _vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        // As for `Template`: no `__eq__`, so interpolations compare by identity.
        Ok(None)
    }

    fn py_hash(&self, self_id: HeapId, _vm: &mut VM<'h>) -> RunResult<Option<HashValue>> {
        Ok(Some(identity_hash(self_id)))
    }

    fn py_repr_fmt(&self, f: &mut impl Write, vm: &mut VM<'h>, heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        let Ok(mut guard) = vm.recursion_guard() else {
            return Ok(f.write_str("...")?);
        };
        let vm = &mut *guard;
        // Cloned out first: recursing into `py_repr_fmt` needs the heap mutably.
        let fields = {
            let this = self.get(vm.heap);
            [
                this.value.clone_with_heap(vm.heap),
                this.expression.clone_with_heap(vm.heap),
                this.conversion.clone_with_heap(vm.heap),
                this.format_spec.clone_with_heap(vm.heap),
            ]
        };
        defer_drop!(fields, vm);
        f.write_str("Interpolation(")?;
        for (index, field) in fields.iter().enumerate() {
            if index > 0 {
                f.write_str(", ")?;
            }
            field.py_repr_fmt(f, vm, heap_ids)?;
        }
        Ok(f.write_char(')')?)
    }

    fn py_getattr(&self, attr: &EitherStr, vm: &mut VM<'h>) -> RunResult<Option<CallResult>> {
        // Fast path: interned names match by id, with no string comparison.
        if let Some(ss) = attr.static_string() {
            let this = self.get(vm.heap);
            return Ok(match ss {
                StaticStrings::ValueAttr => Some(this.value.clone_with_heap(vm.heap)),
                StaticStrings::Expression => Some(this.expression.clone_with_heap(vm.heap)),
                StaticStrings::Conversion => Some(this.conversion.clone_with_heap(vm.heap)),
                StaticStrings::FormatSpec => Some(this.format_spec.clone_with_heap(vm.heap)),
                _ => None,
            }
            .map(CallResult::Value));
        }
        // Slow path: a name built at runtime is a heap str.
        let name = attr.as_str(vm.interns);
        let this = self.get(vm.heap);
        Ok(match name {
            "value" => Some(this.value.clone_with_heap(vm.heap)),
            "expression" => Some(this.expression.clone_with_heap(vm.heap)),
            "conversion" => Some(this.conversion.clone_with_heap(vm.heap)),
            "format_spec" => Some(this.format_spec.clone_with_heap(vm.heap)),
            _ => None,
        }
        .map(CallResult::Value))
    }
}

impl HeapItem for Interpolation {
    fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        self.value.py_dec_ref_ids(stack);
        self.expression.py_dec_ref_ids(stack);
        self.conversion.py_dec_ref_ids(stack);
        self.format_spec.py_dec_ref_ids(stack);
    }
}

/// Borrows the items of a value the caller promised is a `Tuple`.
///
/// # Panics
/// If the value is not a heap `Tuple`. See [`allocate_template`] for why that
/// can only be a compiler bug.
fn tuple_items<'a>(value: &Value, vm: &'a VM<'_>) -> &'a [Value] {
    match value {
        Value::Ref(id) => match vm.heap.get(*id) {
            HeapData::Tuple(tuple) => tuple.as_slice(),
            other => panic!("template component must be a tuple, got {:?}", other.py_type()),
        },
        other => panic!("template component must be a heap tuple, got {other:?}"),
    }
}
