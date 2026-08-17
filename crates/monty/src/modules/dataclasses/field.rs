//! The `dataclasses.Field` objects making up a class's `__dataclass_fields__`,
//! and the `field()` factory that configures one before a class claims it.

use std::fmt::Write;

use crate::{
    args::{ArgValues, FromArgs},
    bytecode::{CallResult, VM},
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunResult},
    hash::{HashValue, identity_hash},
    heap::{ContainsHeap, DropWithContext, HeapData, HeapId, HeapItem, HeapRead},
    intern::{StaticStrings, StringId},
    types::{Dict, LazyHeapSet, PyTrait, Type, str::allocate_string},
    value::{EitherStr, Marker, Value},
};

/// `dataclasses.MISSING`: the sentinel standing where no value was given.
///
/// An immediate value, so it is a true singleton (`f.default is MISSING` holds
/// without any heap identity to preserve) and costs nothing to copy around.
pub(crate) const MISSING: Value = Value::Marker(Marker(StaticStrings::Missing));

/// Whether `value` is [`MISSING`].
pub(crate) fn is_missing(value: &Value) -> bool {
    matches!(value, Value::Marker(Marker(StaticStrings::Missing)))
}

/// One entry of a class's `__dataclass_fields__`: CPython's `dataclasses.Field`.
///
/// Also the value `field(...)` returns before any class has claimed it, which
/// is the same object in CPython: `name`/`annotation` are then unset and
/// [`claim`](Self::claim) fills them in when `@dataclass` walks the body.
///
/// **Owns heap references** (every `Value` field), reported by
/// [`child_values`](Self::child_values), which both the cycle collector and
/// `py_dec_ref_ids` below drive: a default can reach back to its class,
/// closing a cycle.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct DataclassField {
    /// The interned field name, `None` until a class claims this spec.
    name: Option<StringId>,
    /// `Field.type`: the annotation as source text, never evaluated.
    /// `Value::None` on an unclaimed spec, as in CPython.
    annotation: Value,
    /// The default **captured when `@dataclass` ran**, or `None` for
    /// `MISSING`. Rebinding the class attribute afterwards must not change it.
    default: Option<Value>,
    /// Called with no arguments to build a per-instance default.
    default_factory: Option<Value>,
    /// `Field.metadata`: the mapping given to `field(metadata=...)`.
    metadata: Option<Value>,
    /// `Field.doc`: the string given to `field(doc=...)`.
    doc: Option<Value>,
    /// Whether the field is a parameter of the synthesized `__init__`.
    init: bool,
    /// Whether the synthesized `__repr__` shows it.
    repr: bool,
    /// Whether the synthesized `__eq__` and ordering compare it.
    compare: bool,
    /// `Field.hash`: `None` means "follow `compare`", CPython's default.
    hash: Option<bool>,
    /// Keyword-only in `__init__`. `None` on a spec that did not say, which
    /// [`claim`](Self::claim) then resolves from the class-level `kw_only`.
    kw_only: Option<bool>,
}

impl DataclassField {
    /// A field for `x: int` or `x: int = 5`, with every per-field option at its
    /// CPython default. Takes ownership of `annotation` and `default`.
    #[must_use]
    pub fn new(name: StringId, annotation: Value, default: Option<Value>) -> Self {
        Self {
            name: Some(name),
            annotation,
            default,
            default_factory: None,
            metadata: None,
            doc: None,
            init: true,
            repr: true,
            compare: true,
            hash: None,
            kw_only: None,
        }
    }

    /// The spec `field(...)` returns: unnamed and untyped until a class body
    /// claims it. Takes ownership of every value passed.
    #[must_use]
    #[expect(clippy::too_many_arguments, reason = "one per `field()` keyword")]
    pub fn spec(
        default: Option<Value>,
        default_factory: Option<Value>,
        metadata: Option<Value>,
        doc: Option<Value>,
        init: bool,
        repr: bool,
        compare: bool,
        hash: Option<bool>,
        kw_only: Option<bool>,
    ) -> Self {
        Self {
            name: None,
            annotation: Value::None,
            default,
            default_factory,
            metadata,
            doc,
            init,
            repr,
            compare,
            hash,
            kw_only,
        }
    }

    /// Names and types a `field()` spec, and settles its keyword-only-ness from
    /// the class-level `kw_only` when the spec did not say. Takes ownership of
    /// `annotation`.
    ///
    /// The annotation it overwrites is the `Value::None` [`spec`](Self::spec)
    /// left, which owns no reference, so nothing needs releasing here. The
    /// caller always claims a *clone* of the spec the class body holds, so this
    /// never runs twice on one field.
    pub fn claim(&mut self, name: StringId, annotation: Value, class_kw_only: bool) {
        debug_assert!(
            matches!(self.annotation, Value::None),
            "claim overwrites the placeholder annotation of an unclaimed spec"
        );
        self.name = Some(name);
        self.annotation = annotation;
        self.settle_kw_only(class_kw_only);
    }

    /// Resolves "keyword-only unless the field said otherwise" from the
    /// class-level `kw_only`, which every field must do before it is stored.
    pub fn settle_kw_only(&mut self, class_kw_only: bool) {
        self.kw_only = Some(self.kw_only.unwrap_or(class_kw_only));
    }

    /// A field carrying the same configuration and fresh references to every
    /// value it holds. Used to lift a `field()` spec out of the class
    /// namespace, which keeps owning the original.
    #[must_use]
    pub fn clone_with_heap(&self, ctx: &impl ContainsHeap) -> Self {
        let clone = |value: &Option<Value>| value.as_ref().map(|v| v.clone_with_heap(ctx));
        Self {
            name: self.name,
            annotation: self.annotation.clone_with_heap(ctx),
            default: clone(&self.default),
            default_factory: clone(&self.default_factory),
            metadata: clone(&self.metadata),
            doc: clone(&self.doc),
            init: self.init,
            repr: self.repr,
            compare: self.compare,
            hash: self.hash,
            kw_only: self.kw_only,
        }
    }

    /// The interned field name, as the synthesized `__init__` binds it.
    ///
    /// # Panics
    /// On a spec no class has claimed. Every field reachable from a
    /// `__dataclass_fields__` mapping has been claimed; a bare spec is only
    /// ever read through the Python-visible accessors below.
    #[must_use]
    pub fn name(&self) -> StringId {
        self.name.expect("a field in __dataclass_fields__ has been claimed")
    }

    /// `Field.type`, borrowed: the annotation the class was defined with.
    #[must_use]
    pub fn annotation(&self) -> &Value {
        &self.annotation
    }

    /// The captured default, or `None` for a field without one.
    #[must_use]
    pub fn default(&self) -> Option<&Value> {
        self.default.as_ref()
    }

    /// The zero-argument callable building this field's default, if any.
    #[must_use]
    pub fn default_factory(&self) -> Option<&Value> {
        self.default_factory.as_ref()
    }

    /// Whether `__init__` takes this field as a parameter.
    #[must_use]
    pub fn init(&self) -> bool {
        self.init
    }

    /// Whether the synthesized `__repr__` shows this field.
    #[must_use]
    pub fn repr(&self) -> bool {
        self.repr
    }

    /// Whether `__eq__` and the ordering methods compare this field.
    #[must_use]
    pub fn compare(&self) -> bool {
        self.compare
    }

    /// Whether the generated `__hash__` includes this field: `hash=None`
    /// follows `compare`, matching CPython's rule.
    #[must_use]
    pub fn hashed(&self) -> bool {
        self.hash.unwrap_or(self.compare)
    }

    /// Whether `__init__` takes this field by keyword only. Unclaimed specs
    /// report CPython's `False` default.
    #[must_use]
    pub fn kw_only(&self) -> bool {
        self.kw_only.unwrap_or(false)
    }

    /// Whether `__init__` can fill this field without being given a value.
    #[must_use]
    pub fn has_default(&self) -> bool {
        self.default.is_some() || self.default_factory.is_some()
    }

    /// Every value this field owns a reference to, for the cycle collector and
    /// refcount teardown. Kept in one place so a new field cannot be missed.
    pub fn child_values(&self) -> impl Iterator<Item = &Value> {
        [
            Some(&self.annotation),
            self.default.as_ref(),
            self.default_factory.as_ref(),
            self.metadata.as_ref(),
            self.doc.as_ref(),
        ]
        .into_iter()
        .flatten()
    }
}

/// Releases a field that never reached the heap: one the decorator collected
/// before rejecting the class.
impl<C: ContainsHeap> DropWithContext<C> for DataclassField {
    fn drop_with(self, ctx: &mut C) {
        self.annotation.drop_with(ctx);
        self.default.drop_with(ctx);
        self.default_factory.drop_with(ctx);
        self.metadata.drop_with(ctx);
        self.doc.drop_with(ctx);
    }
}

impl<'h> PyTrait<'h> for HeapRead<'h, DataclassField> {
    fn py_type(&self, _vm: &VM<'h>) -> Type {
        Type::DataclassField
    }

    fn py_len(&self, _vm: &VM<'h>) -> Option<usize> {
        None
    }

    fn py_eq_impl(&self, _other: &Value, _vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        // `Field` defines no `__eq__`, so it compares by identity, which
        // `Value::py_eq_impl` resolves before ever reaching here.
        Ok(None)
    }

    fn py_hash(&self, self_id: HeapId, _vm: &mut VM<'h>) -> RunResult<Option<HashValue>> {
        Ok(Some(identity_hash(self_id)))
    }

    /// CPython's `Field.__repr__`, attribute for attribute. The two spellings
    /// Monty cannot reproduce are documented divergences: `type` is annotation
    /// text, and no repr here carries an object address.
    fn py_repr_fmt(&self, f: &mut impl Write, vm: &mut VM<'h>, heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        let Ok(mut guard) = vm.recursion_guard() else {
            return Ok(f.write_str("...")?);
        };
        let vm = &mut *guard;
        // Rendered as the empty mapping `Field.metadata` hands back, which is
        // where CPython writes `mappingproxy({})`.
        let empty_metadata = Value::Ref(vm.heap.allocate(HeapData::Dict(Dict::new())));
        defer_drop!(empty_metadata, vm);
        // Cloned out first: recursing into `py_repr_fmt` needs the heap mutably.
        let (name, claimed, values) = {
            let this = self.get(vm.heap);
            let clone = |value: Option<&Value>| value.map_or(MISSING, |v| v.clone_with_heap(vm.heap));
            (
                this.name.map_or(Value::None, Value::InternString),
                this.name.is_some(),
                vec![
                    this.annotation.clone_with_heap(vm.heap),
                    clone(this.default.as_ref()),
                    clone(this.default_factory.as_ref()),
                    Value::Bool(this.init),
                    Value::Bool(this.repr),
                    this.hash.map_or(Value::None, Value::Bool),
                    Value::Bool(this.compare),
                    this.metadata.as_ref().map_or_else(
                        || empty_metadata.clone_with_heap(vm.heap),
                        |v| v.clone_with_heap(vm.heap),
                    ),
                    this.kw_only.map_or(MISSING, Value::Bool),
                    // `doc`'s absent value is `None`, not the `MISSING` the
                    // defaults use.
                    this.doc.as_ref().map_or(Value::None, |v| v.clone_with_heap(vm.heap)),
                ],
            )
        };
        defer_drop!(values, vm);
        f.write_str("Field(name=")?;
        name.py_repr_fmt(f, vm, heap_ids)?;
        for (label, value) in REPR_LABELS.iter().zip(values) {
            write!(f, ",{label}=")?;
            value.py_repr_fmt(f, vm, heap_ids)?;
        }
        // `_FIELD` / `None` textually: the sentinel has no Monty object, which
        // is also why reading the attribute raises (see `py_getattr`).
        let field_type = if claimed { "_FIELD" } else { "None" };
        Ok(write!(f, ",_field_type={field_type})")?)
    }

    /// Every attribute CPython's `Field` exposes except `_field_type`, whose
    /// `dataclasses._FIELD` sentinel Monty has no object for.
    fn py_getattr(&self, attr: &EitherStr, vm: &mut VM<'h>) -> RunResult<Option<CallResult>> {
        let attr_str = attr.as_str(vm.interns);
        let value = match attr_str {
            "name" => match self.get(vm.heap).name {
                Some(name) => {
                    let name = vm.interns.get_str(name).to_owned();
                    allocate_string(name, vm.heap)
                }
                None => Value::None,
            },
            "type" => self.get(vm.heap).annotation.clone_with_heap(vm.heap),
            "default" => optional_attr(self.get(vm.heap).default.as_ref(), vm),
            "default_factory" => optional_attr(self.get(vm.heap).default_factory.as_ref(), vm),
            "init" => Value::Bool(self.get(vm.heap).init),
            "repr" => Value::Bool(self.get(vm.heap).repr),
            "compare" => Value::Bool(self.get(vm.heap).compare),
            "hash" => self.get(vm.heap).hash.map_or(Value::None, Value::Bool),
            "kw_only" => self.get(vm.heap).kw_only.map_or(MISSING, Value::Bool),
            "doc" => match self.get(vm.heap).doc.as_ref() {
                Some(doc) => doc.clone_with_heap(vm.heap),
                None => Value::None,
            },
            // A plain dict where CPython hands back a read-only view of it.
            "metadata" => match self.get(vm.heap).metadata.as_ref() {
                Some(metadata) => metadata.clone_with_heap(vm.heap),
                None => Value::Ref(vm.heap.allocate(HeapData::Dict(Dict::new()))),
            },
            "_field_type" => {
                return Err(ExcType::not_implemented(
                    "Field._field_type is not yet supported, dataclasses._FIELD is not implemented",
                )
                .into());
            }
            _ => return Err(ExcType::attribute_error("Field", attr_str)),
        };
        Ok(Some(CallResult::Value(value)))
    }
}

/// The attributes `Field.__repr__` writes after `name`, in CPython's order.
const REPR_LABELS: [&str; 10] = [
    "type",
    "default",
    "default_factory",
    "init",
    "repr",
    "hash",
    "compare",
    "metadata",
    "kw_only",
    "doc",
];

/// A `Field` attribute whose absent case *is* `MISSING` in CPython.
fn optional_attr(value: Option<&Value>, vm: &VM<'_>) -> Value {
    value.map_or(MISSING, |v| v.clone_with_heap(vm.heap))
}

impl HeapItem for DataclassField {
    fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        for value in [
            Some(&mut self.annotation),
            self.default.as_mut(),
            self.default_factory.as_mut(),
            self.metadata.as_mut(),
            self.doc.as_mut(),
        ]
        .into_iter()
        .flatten()
        {
            value.py_dec_ref_ids(stack);
        }
    }
}

/// `field(*, default=MISSING, default_factory=MISSING, init=True, repr=True,
/// hash=None, compare=True, metadata=None, kw_only=MISSING, doc=None)`.
///
/// CPython implements it in Python, so its binding errors are `def`-family.
pub(super) fn field(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let FieldArgs {
        default,
        default_factory,
        init,
        repr,
        hash,
        compare,
        metadata,
        kw_only,
        doc,
    } = FieldArgs::from_args(args, vm)?;
    defer_drop!(default, vm);
    defer_drop!(default_factory, vm);
    defer_drop!(metadata, vm);
    defer_drop!(doc, vm);
    defer_drop!(hash, vm);
    defer_drop!(kw_only, vm);
    defer_drop!(init, vm);
    defer_drop!(repr, vm);
    defer_drop!(compare, vm);

    let default = present(default, vm);
    let default_factory = present(default_factory, vm);
    if default.is_some() && default_factory.is_some() {
        return Err(ExcType::value_error("cannot specify both default and default_factory"));
    }
    let metadata = match metadata {
        Value::None => None,
        // CPython hands the argument straight to `mappingproxy`, so its error is
        // what a non-mapping raises; Monty has no proxy but keeps the wording.
        value if matches!(value.py_type(vm), Type::Dict | Type::DefaultDict | Type::Counter) => {
            Some(value.clone_with_heap(vm.heap))
        }
        value => {
            let type_name = value.py_type_name(vm);
            return Err(ExcType::type_error(format!(
                "mappingproxy() argument must be a mapping, not {type_name}"
            )));
        }
    };
    let doc = match doc {
        Value::None => None,
        value => Some(value.clone_with_heap(vm.heap)),
    };
    let spec = DataclassField::spec(
        default,
        default_factory,
        metadata,
        doc,
        init.py_bool(vm)?,
        repr.py_bool(vm)?,
        compare.py_bool(vm)?,
        optional_bool(hash, vm)?,
        if is_missing(kw_only) {
            None
        } else {
            Some(kw_only.py_bool(vm)?)
        },
    );
    Ok(Value::Ref(vm.heap.allocate(HeapData::DataclassField(Box::new(spec)))))
}

/// A fresh reference to `value` unless it is [`MISSING`], which stands for
/// "not given".
fn present(value: &Value, vm: &VM<'_>) -> Option<Value> {
    if is_missing(value) {
        None
    } else {
        Some(value.clone_with_heap(vm.heap))
    }
}

/// `field(hash=...)`: `None` keeps CPython's "follow `compare`" default,
/// anything else is taken for its truth.
fn optional_bool(value: &Value, vm: &mut VM<'_>) -> RunResult<Option<bool>> {
    match value {
        Value::None => Ok(None),
        value => Ok(Some(value.py_bool(vm)?)),
    }
}

#[derive(FromArgs)]
#[from_args(name = "field", style = def)]
struct FieldArgs {
    #[from_args(kw_only, default = MISSING)]
    default: Value,
    #[from_args(kw_only, default = MISSING)]
    default_factory: Value,
    #[from_args(kw_only, default = Value::Bool(true))]
    init: Value,
    #[from_args(kw_only, default = Value::Bool(true))]
    repr: Value,
    #[from_args(kw_only, default = Value::None)]
    hash: Value,
    #[from_args(kw_only, default = Value::Bool(true))]
    compare: Value,
    #[from_args(kw_only, default = Value::None)]
    metadata: Value,
    #[from_args(kw_only, default = MISSING)]
    kw_only: Value,
    #[from_args(kw_only, default = Value::None)]
    doc: Value,
}
