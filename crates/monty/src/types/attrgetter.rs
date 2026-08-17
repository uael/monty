//! `operator.attrgetter`: a callable that fetches one or more attributes.
//!
//! Each argument is split on `.` at construction, as CPython's `attrgetter_new`
//! does, so a dotted path is walked one attribute at a time when the getter is
//! called. One argument yields the attribute itself; two or more yield a tuple,
//! which is the shape `sorted(key=...)` relies on.
//!
//! Splitting eagerly is what makes `attrgetter('x.')` raise about an empty
//! attribute name rather than about `'x.'`, matching CPython.

use std::fmt::Write;

use crate::{
    args::ArgValues,
    bytecode::{CallResult, VM},
    exception_private::{ExcType, ExcTypeExt, RunResult, SimpleException},
    heap::{DropWithContext, HeapData, HeapId, HeapItem, HeapRead},
    types::{LazyHeapSet, PyTrait, Type, tuple::allocate_tuple},
    value::{EitherStr, Value},
};

/// An `operator.attrgetter` instance.
///
/// Holds no heap references: every path component is an owned string or an
/// interned id, so this is a leaf type for GC purposes.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct AttrGetter {
    /// One entry per constructor argument, each already split on `.`. A
    /// single-component entry is the undotted case.
    paths: Vec<Vec<EitherStr>>,
}

/// `operator.attrgetter(attr, *attrs)`.
///
/// CPython's `attrgetter_new` rejects keywords wholesale and requires at least
/// one argument, then type-checks each; `into_pos_only` gives the first two and
/// the loop below the third, in that order.
pub(crate) fn init(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let names: Vec<Value> = args.into_pos_only("attrgetter", vm.heap)?.collect();
    if names.is_empty() {
        return Err(ExcType::type_error_expected_exact("attrgetter", 1, 0));
    }
    let mut paths = Vec::with_capacity(names.len());
    for name in &names {
        let Some(name) = name.as_either_str(vm.heap) else {
            names.drop_with(vm);
            return Err(ExcType::type_error("attribute name must be a string"));
        };
        // Split on the interned/owned text once; the components are what every
        // later call walks, so the cost is paid here rather than per call.
        paths.push(
            name.as_str(vm.interns)
                .split('.')
                .map(|part| EitherStr::from(part.to_owned()))
                .collect(),
        );
    }
    names.drop_with(vm);
    Ok(Value::Ref(vm.heap.allocate(HeapData::AttrGetter(AttrGetter { paths }))))
}

/// Applies the getter to `target`, the body of `attrgetter.__call__`.
///
/// Walks each path with the ordinary attribute lookup, so a missing attribute
/// raises exactly what `getattr` would.
pub(crate) fn call(self_id: HeapId, vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let positional: Vec<Value> = args.into_pos_only("attrgetter", vm.heap)?.collect();
    let [target] = positional.as_slice() else {
        let given = positional.len();
        positional.drop_with(vm);
        return Err(ExcType::type_error_expected_exact("attrgetter", 1, given));
    };
    // Cloned out of the heap entry: the walk re-enters attribute lookup, which
    // must not run while a read handle on this getter is alive.
    let paths = clone_paths(self_id, vm);
    let target = target.clone_with_heap(vm.heap);
    positional.drop_with(vm);
    crate::defer_drop!(target, vm);

    let mut results = Vec::with_capacity(paths.len());
    for path in &paths {
        match walk(target, path, vm) {
            Ok(value) => results.push(value),
            Err(error) => {
                results.drop_with(vm);
                return Err(error);
            }
        }
    }
    // One argument yields the attribute itself; more yield a tuple.
    if paths.len() == 1 {
        Ok(results.pop().expect("one path yields one result"))
    } else {
        Ok(allocate_tuple(results.into_iter().collect(), vm.heap))
    }
}

/// Copies the paths out of the heap entry so the walk can re-enter the VM.
fn clone_paths(self_id: HeapId, vm: &VM<'_>) -> Vec<Vec<EitherStr>> {
    let HeapData::AttrGetter(getter) = vm.heap.get(self_id) else {
        unreachable!("dispatched on HeapData::AttrGetter")
    };
    getter.paths.clone()
}

/// Follows one dotted path from `target`.
fn walk(target: &Value, path: &[EitherStr], vm: &mut VM<'_>) -> RunResult<Value> {
    let mut current = target.clone_with_heap(vm.heap);
    for component in path {
        let next = current.py_getattr(component, vm);
        current.drop_with(vm);
        match next? {
            CallResult::Value(value) => current = value,
            other => {
                other.drop_with(vm);
                // Mirrors `getattr()`: only a plain value can be handed back,
                // since there is nowhere here to yield to the host from.
                return Err(SimpleException::new_msg(
                    ExcType::TypeError,
                    "attrgetter(): attribute is not a simple value",
                )
                .into());
            }
        }
    }
    Ok(current)
}

impl<'h> PyTrait<'h> for HeapRead<'h, AttrGetter> {
    fn py_type(&self, _vm: &VM<'h>) -> Type {
        Type::AttrGetter
    }

    fn py_len(&self, _vm: &VM<'h>) -> Option<usize> {
        None
    }

    fn py_eq_impl(&self, _other: &Value, _vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        // CPython defines no `__eq__`, so two getters over the same attribute
        // compare unequal and identity decides.
        Ok(None)
    }

    /// `operator.attrgetter('a', 'b.c')` — the arguments rebuilt by rejoining
    /// each path, which round-trips exactly because the split was on `.`.
    fn py_repr_fmt(&self, f: &mut impl Write, vm: &mut VM<'h>, _heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        f.write_str("operator.attrgetter(")?;
        for (index, path) in self.get(vm.heap).paths.iter().enumerate() {
            if index > 0 {
                f.write_str(", ")?;
            }
            let joined = path
                .iter()
                .map(|part| part.as_str(vm.interns))
                .collect::<Vec<_>>()
                .join(".");
            write!(f, "'{joined}'")?;
        }
        Ok(f.write_str(")")?)
    }
}

impl HeapItem for AttrGetter {
    fn py_dec_ref_ids(&mut self, _stack: &mut Vec<HeapId>) {
        // A leaf: paths are plain strings, never heap references.
    }
}
