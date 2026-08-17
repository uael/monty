//! `itertools.accumulate(iterable, func=None, *, initial=None)` — running totals.

use serde::{Deserialize, Serialize};

use crate::{
    args::ArgValues,
    bytecode::VM,
    defer_drop,
    exception_private::RunResult,
    heap::{DropWithContext, HeapId, HeapRead},
    types::itertools::ItertoolsIter,
    value::Value,
};

/// Yields `t0, f(t0, s1), f(f(t0, s1), s2), …` over a source iterator.
///
/// `total` is the running value and a second owned ref alongside `source`.
/// CPython cannot tell an omitted `initial`/`func` from an explicit `None`, so
/// neither does this: both arrive here as `None` and mean "absent".
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Accumulate {
    /// The iterator being folded. `None` once exhausted, which both latches the
    /// adaptor as spent and releases the source there and then.
    source: Option<Value>,
    /// The binary callable, or `None` for addition.
    func: Option<Value>,
    /// The running total. Before the first step this is the `initial` argument,
    /// if one was given.
    total: Option<Value>,
    /// Whether a value has been yielded yet. Distinct from `total.is_some()`,
    /// which cannot tell "no initial given" from "initial consumed".
    started: bool,
}

impl Accumulate {
    /// Takes ownership of `source` (already resolved by `py_iter`), `func` and
    /// the `initial` total.
    pub(crate) fn new(source: Value, func: Option<Value>, initial: Option<Value>) -> Self {
        Self {
            source: Some(source),
            func,
            total: initial,
            started: false,
        }
    }

    /// Invokes `on_child` for each heap id this iterator owns (GC trace hook).
    pub(crate) fn for_each_child_id(&self, mut on_child: impl FnMut(HeapId)) {
        for slot in [&self.source, &self.func, &self.total] {
            if let Some(Value::Ref(id)) = slot {
                on_child(*id);
            }
        }
    }

    /// Releases the refs this iterator owns (mirrors `for_each_child_id`).
    pub(crate) fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        for value in [&mut self.source, &mut self.func, &mut self.total]
            .into_iter()
            .flatten()
        {
            value.py_dec_ref_ids(stack);
        }
    }
}

/// Produces the next running total.
///
/// The first step yields the initial value, or the source's first item when
/// there was none; every later step folds one more item in.
pub(super) fn next<'h>(iter: &mut HeapRead<'h, ItertoolsIter>, vm: &mut VM<'h>) -> RunResult<Option<Value>> {
    let ItertoolsIter::Accumulate(accumulate) = iter.get(vm.heap) else {
        unreachable!("dispatched on Kind::Accumulate")
    };
    if accumulate.source.is_none() {
        return Ok(None);
    }

    if !accumulate.started {
        // With an `initial`, the first yield is that value untouched and the
        // source is not advanced at all.
        if accumulate.total.is_some() {
            let ItertoolsIter::Accumulate(accumulate) = iter.get_mut(vm.heap) else {
                unreachable!("dispatched on Kind::Accumulate")
            };
            accumulate.started = true;
            let ItertoolsIter::Accumulate(accumulate) = iter.get(vm.heap) else {
                unreachable!("dispatched on Kind::Accumulate")
            };
            return Ok(Some(
                accumulate
                    .total
                    .as_ref()
                    .expect("checked above")
                    .clone_with_heap(vm.heap),
            ));
        }
        // Without one, the source's first item becomes the total. A source that
        // stops immediately yields nothing at all.
        let Some(first) = drive_source(iter, vm)? else {
            return Ok(None);
        };
        let yielded = first.clone_with_heap(vm.heap);
        let ItertoolsIter::Accumulate(accumulate) = iter.get_mut(vm.heap) else {
            unreachable!("dispatched on Kind::Accumulate")
        };
        accumulate.started = true;
        accumulate.total = Some(first);
        return Ok(Some(yielded));
    }

    let Some(element) = drive_source(iter, vm)? else {
        return Ok(None);
    };
    defer_drop!(element, vm);

    // Both are cloned out before the fold: combining re-enters the VM when
    // `func` is a Python callable, and no borrow may be held across that.
    let (func, total) = {
        let ItertoolsIter::Accumulate(accumulate) = iter.get(vm.heap) else {
            unreachable!("dispatched on Kind::Accumulate")
        };
        (
            accumulate.func.as_ref().map(|f| f.clone_with_heap(vm.heap)),
            accumulate
                .total
                .as_ref()
                .expect("started implies a total")
                .clone_with_heap(vm.heap),
        )
    };
    let combined = combine(func, total, element, vm)?;

    let yielded = combined.clone_with_heap(vm.heap);
    let ItertoolsIter::Accumulate(accumulate) = iter.get_mut(vm.heap) else {
        unreachable!("dispatched on Kind::Accumulate")
    };
    let previous = accumulate.total.replace(combined);
    previous.drop_with(vm);
    Ok(Some(yielded))
}

/// Folds one element into the running total, by `func` or by `+`.
///
/// Consumes `func` and `total`; `element` stays with its caller's guard.
fn combine(func: Option<Value>, total: Value, element: &Value, vm: &mut VM<'_>) -> RunResult<Value> {
    let Some(func) = func else {
        // CPython's default is `operator.add`, so this is the same `+` any
        // Python expression would do, error messages included.
        let combined = total.py_add(element, vm);
        total.drop_with(vm);
        return combined;
    };
    defer_drop!(func, vm);
    let args = ArgValues::Two(total, element.clone_with_heap(vm.heap));
    vm.evaluate_function("accumulate()", func, args)
}

/// Advances the source, releasing it and the total once it stops.
fn drive_source<'h>(iter: &mut HeapRead<'h, ItertoolsIter>, vm: &mut VM<'h>) -> RunResult<Option<Value>> {
    let source = {
        let ItertoolsIter::Accumulate(accumulate) = iter.get(vm.heap) else {
            unreachable!("dispatched on Kind::Accumulate")
        };
        accumulate.source.as_ref().map(|source| source.clone_with_heap(vm.heap))
    };
    let Some(source) = source else {
        return Ok(None);
    };
    defer_drop!(source, vm);
    let item = {
        let mut source_read = source.read(vm);
        source_read.py_next(vm)
    };

    if let Some(item) = item? {
        Ok(Some(item))
    } else {
        let ItertoolsIter::Accumulate(accumulate) = iter.get_mut(vm.heap) else {
            unreachable!("dispatched on Kind::Accumulate")
        };
        // Released here rather than at destruction, as the other source-driving
        // adaptors do, so a spent accumulate frees what it was folding over.
        let (source, func, total) = (
            accumulate.source.take(),
            accumulate.func.take(),
            accumulate.total.take(),
        );
        source.drop_with(vm);
        func.drop_with(vm);
        total.drop_with(vm);
        Ok(None)
    }
}
