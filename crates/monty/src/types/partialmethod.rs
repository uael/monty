//! `functools.partialmethod`: a method with some arguments already supplied.
//!
//! Bound through the ordinary class-member path rather than a descriptor
//! protocol: Monty binds a function-valued class member into a
//! [`BoundMethod`](super::BoundMethod), and a `partialmethod` joins that set
//! (see `is_method_value` in `instance.rs`). So `c.abort()` arrives here with
//! `c` as the first positional and the call's own arguments after it, which is
//! exactly the shape CPython's generated `_method` receives.
//!
//! Reached through the class instead (`C.abort(c)`), the receiver is simply the
//! first argument the caller passed, and the same rule applies unchanged.

use std::{fmt::Write, mem};

use crate::{
    args::{ArgValues, KwargsValues},
    bytecode::{CallResult, VM},
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunResult, SimpleException},
    heap::{DropWithContext, HeapData, HeapId, HeapItem, HeapRead},
    types::{Dict, LazyHeapSet, PyTrait, Type, tuple::allocate_tuple},
    value::{EitherStr, Value},
};

/// A `functools.partialmethod` instance.
///
/// `func`, `args` and every key and value in `keywords` are owned references,
/// released by [`HeapItem::py_dec_ref_ids`].
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct PartialMethod {
    /// The callable the stored arguments are applied to.
    func: Value,
    /// Positional arguments inserted after the receiver.
    args: Vec<Value>,
    /// Keyword arguments, which a call may override.
    keywords: Vec<(Value, Value)>,
}

impl PartialMethod {
    /// Runs `f` on every owned reference. Backs the GC child walker, and MUST
    /// report the same references as [`HeapItem::py_dec_ref_ids`].
    pub(crate) fn for_each_owned_value(&self, mut f: impl FnMut(&Value)) {
        f(&self.func);
        for arg in &self.args {
            f(arg);
        }
        for (key, value) in &self.keywords {
            f(key);
            f(value);
        }
    }
}

/// `functools.partialmethod(func, /, *args, **keywords)`.
pub(crate) fn init(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let (positional, keywords) = args.into_parts();
    let mut positional: Vec<Value> = positional.collect();
    let keywords: Vec<(Value, Value)> = keywords.into_iter().collect();
    if positional.is_empty() {
        positional.drop_with(vm);
        keywords.drop_with(vm);
        return Err(SimpleException::new_msg(
            ExcType::TypeError,
            "_partial_new() missing 1 required positional argument: 'func'",
        )
        .into());
    }
    let func = positional.remove(0);
    if !func.is_callable(vm.heap) {
        // CPython also accepts a descriptor here; Monty has none, so a
        // non-callable is the whole of the rejection.
        let mut rendered = String::new();
        let mut heap_ids = LazyHeapSet::default();
        let written = func.py_repr_fmt(&mut rendered, vm, &mut heap_ids);
        func.drop_with(vm);
        positional.drop_with(vm);
        keywords.drop_with(vm);
        written?;
        return Err(ExcType::type_error(format!(
            "the first argument {rendered} must be a callable or a descriptor"
        )));
    }
    Ok(Value::Ref(vm.heap.allocate(HeapData::PartialMethod(Box::new(
        PartialMethod {
            func,
            args: positional,
            keywords,
        },
    )))))
}

/// Calls the underlying function as `func(receiver, *stored, *given, **merged)`.
///
/// The receiver is whatever arrived first: the instance a `BoundMethod`
/// prepended, or the caller's own first argument when reached through the class.
pub(crate) fn call(self_id: HeapId, vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let (given, given_keywords) = args.into_parts();
    let given: Vec<Value> = given.collect();
    let given_keywords: Vec<(Value, Value)> = given_keywords.into_iter().collect();

    // Everything is cloned out of the entry before the call, which re-enters
    // the VM and so may not run while a read handle is alive.
    let (func, stored, mut merged) = {
        let HeapData::PartialMethod(partial) = vm.heap.get(self_id) else {
            unreachable!("dispatched on HeapData::PartialMethod")
        };
        (
            partial.func.clone_with_heap(vm.heap),
            partial
                .args
                .iter()
                .map(|a| a.clone_with_heap(vm.heap))
                .collect::<Vec<_>>(),
            partial
                .keywords
                .iter()
                .map(|(k, v)| (k.clone_with_heap(vm.heap), v.clone_with_heap(vm.heap)))
                .collect::<Vec<_>>(),
        )
    };
    defer_drop!(func, vm);

    // `{**self.keywords, **keywords}`: a keyword given at the call replaces the
    // stored one rather than colliding with it.
    for (key, value) in given_keywords {
        let existing = merged
            .iter()
            .position(|(stored_key, _)| same_keyword(stored_key, &key, vm));
        match existing {
            Some(index) => {
                let (old_key, old_value) = mem::replace(&mut merged[index], (key, value));
                old_key.drop_with(vm);
                old_value.drop_with(vm);
            }
            None => merged.push((key, value)),
        }
    }

    // The receiver keeps its place at the front, with the stored arguments
    // between it and the call's own — the order CPython's `_method` produces.
    let mut positional = Vec::with_capacity(given.len() + stored.len());
    let mut given = given.into_iter();
    if let Some(receiver) = given.next() {
        positional.push(receiver);
    }
    positional.extend(stored);
    positional.extend(given);

    let kwargs = if merged.is_empty() {
        KwargsValues::Empty
    } else {
        KwargsValues::Pairs(merged)
    };
    let call_args = ArgValues::ArgsKargs {
        args: positional,
        kwargs,
    };
    vm.evaluate_function("partialmethod()", func, call_args)
}

/// Whether two keyword keys name the same parameter.
fn same_keyword(left: &Value, right: &Value, vm: &VM<'_>) -> bool {
    match (
        left.to_str_heap(vm.heap, vm.interns),
        right.to_str_heap(vm.heap, vm.interns),
    ) {
        (Ok(left), Ok(right)) => left == right,
        // Non-string keys cannot be parameter names, so they never merge.
        _ => false,
    }
}

impl<'h> PyTrait<'h> for HeapRead<'h, PartialMethod> {
    fn py_type(&self, _vm: &VM<'h>) -> Type {
        Type::PartialMethod
    }

    fn py_len(&self, _vm: &VM<'h>) -> Option<usize> {
        None
    }

    fn py_eq_impl(&self, _other: &Value, _vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        // A plain object with no `__eq__`, so identity decides, as in CPython.
        Ok(None)
    }

    /// `functools.partialmethod(<func>, 'abort', k=1)` — CPython's repr, which
    /// renders the function and every stored argument.
    fn py_repr_fmt(&self, f: &mut impl Write, vm: &mut VM<'h>, heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        let (func, args, keywords) = {
            let partial = self.get(vm.heap);
            (
                partial.func.clone_with_heap(vm.heap),
                partial
                    .args
                    .iter()
                    .map(|a| a.clone_with_heap(vm.heap))
                    .collect::<Vec<_>>(),
                partial
                    .keywords
                    .iter()
                    .map(|(k, v)| (k.clone_with_heap(vm.heap), v.clone_with_heap(vm.heap)))
                    .collect::<Vec<_>>(),
            )
        };
        let rendered = render_repr(f, &func, &args, &keywords, vm, heap_ids);
        func.drop_with(vm);
        args.drop_with(vm);
        keywords.drop_with(vm);
        rendered
    }

    fn py_getattr(&self, attr: &EitherStr, vm: &mut VM<'h>) -> RunResult<Option<CallResult>> {
        match attr.as_str(vm.interns) {
            "func" => {
                let func = self.get(vm.heap).func.clone_with_heap(vm.heap);
                Ok(Some(CallResult::Value(func)))
            }
            "args" => {
                let args = self
                    .get(vm.heap)
                    .args
                    .iter()
                    .map(|a| a.clone_with_heap(vm.heap))
                    .collect::<Vec<_>>();
                Ok(Some(CallResult::Value(allocate_tuple(
                    args.into_iter().collect(),
                    vm.heap,
                ))))
            }
            "keywords" => {
                let pairs = self
                    .get(vm.heap)
                    .keywords
                    .iter()
                    .map(|(k, v)| (k.clone_with_heap(vm.heap), v.clone_with_heap(vm.heap)))
                    .collect::<Vec<_>>();
                let mut keywords = Dict::new();
                for (key, value) in pairs {
                    keywords.set(key, value, vm)?;
                }
                Ok(Some(CallResult::Value(Value::Ref(
                    vm.heap.allocate(HeapData::Dict(keywords)),
                ))))
            }
            _ => Ok(None),
        }
    }
}

/// Writes the repr body once every part has been cloned out of the heap entry.
fn render_repr(
    f: &mut impl Write,
    func: &Value,
    args: &[Value],
    keywords: &[(Value, Value)],
    vm: &mut VM<'_>,
    heap_ids: &mut LazyHeapSet,
) -> RunResult<()> {
    f.write_str("functools.partialmethod(")?;
    func.py_repr_fmt(f, vm, heap_ids)?;
    for arg in args {
        f.write_str(", ")?;
        arg.py_repr_fmt(f, vm, heap_ids)?;
    }
    for (key, value) in keywords {
        match key.to_str_heap(vm.heap, vm.interns) {
            Ok(name) => write!(f, ", {name}=")?,
            Err(_) => f.write_str(", ")?,
        }
        value.py_repr_fmt(f, vm, heap_ids)?;
    }
    Ok(f.write_str(")")?)
}

impl HeapItem for PartialMethod {
    fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        // Mirrors `for_each_owned_value`.
        self.func.py_dec_ref_ids(stack);
        for arg in &mut self.args {
            arg.py_dec_ref_ids(stack);
        }
        for (key, value) in &mut self.keywords {
            key.py_dec_ref_ids(stack);
            value.py_dec_ref_ids(stack);
        }
    }
}
