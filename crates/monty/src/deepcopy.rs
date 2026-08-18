//! Copying a namespace so that the copy shares nothing with it.
//!
//! Copying a namespace ([`Scopes::copy`](crate::namespaces::Scopes::copy))
//! copies the names and shares the objects. Copying it deeply copies the
//! objects too, so the two namespaces can be run independently: a mutation on
//! one side is invisible on the other, which is what lets a line of execution
//! be tried and thrown away.
//!
//! One law decides every case: **what was defined in the donor is rebuilt
//! against the copy; everything else is shared.** A function, a closure and a
//! class each carry the namespace they were defined in, so the question is
//! answerable about each of them, and answering it is what keeps the copy's own
//! code reading the copy's globals while an imported function keeps reading
//! its own.
//!
//! What is shared besides that is what cannot be observed to be shared: an
//! immutable value has no state to diverge, so copying one would only cost
//! memory. That is the line CPython's `copy.deepcopy` draws with its atomic
//! types, and this draws it in the same place.
//!
//! An object whose class defines `__deepcopy__` is handed to it, as in
//! CPython. That is how a value that must keep its identity across a copy says
//! so, and it means nothing here needs a list of which classes those are.
//!
//! Every copy is built by allocating an empty value of the right shape,
//! recording it as the copy before its children are copied, and swapping the
//! finished payload in afterwards. Recording it first is what lets a value that
//! reaches itself close on the copy instead of recurring forever. Building it
//! rather than rewriting ids under the original is what keeps a hashed
//! container correct: a dict or set keyed by an object hashes that object by
//! identity, so a copied key belongs in a different bucket than the one it came
//! from, and inserting it is also what runs whatever `__hash__` and `__eq__`
//! the keys define.
//!
//! Anything else faults, by name: a generator, a file, a running task. The copy
//! stops and says what it could not carry rather than handing back a namespace
//! that is quietly a different shape from the one that was asked for.

use ahash::AHashMap;

use crate::{
    args::ArgValues,
    exception_private::{ExcType, RunError, RunResult, SimpleException},
    heap::{ContainsHeap, DropWithContext, HeapData, HeapId},
    heap_data::{CellValue, Closure, FunctionDefaults},
    intern::FunctionId,
    namespace::ScopeId,
    types::{
        BoundMethod, Class, Dataclass, Dict, FrozenSet, Instance, List, NamespaceRef, Set, Tuple,
        instance::instance_call_dunder_sync, namespace_ref::values_of,
    },
    value::{EitherStr, Value},
};

use crate::bytecode::VM;

/// What the copy has already made.
struct Memo {
    /// Donor entry to its copy. Holds one reference per entry, released when
    /// the copy ends.
    done: AHashMap<HeapId, Value>,
    /// The memo handed to every `__deepcopy__`, as CPython hands one dict to
    /// the whole of one deep copy.
    guest: Value,
}

impl Memo {
    fn release(self, vm: &mut VM<'_>) {
        for (_, value) in self.done {
            value.drop_with(vm);
        }
        self.guest.drop_with(vm);
    }
}

fn cannot(what: &str) -> RunError {
    SimpleException::new_msg(ExcType::TypeError, format!("a deep copy cannot carry {what}")).into()
}

/// Copies every value bound in `from` into a namespace of its own.
///
/// The new namespace is created before anything is copied, because a function
/// defined in the donor is rebuilt pointing at it.
///
/// # Errors
/// `TypeError` naming the first value that cannot be copied, or whatever a
/// `__deepcopy__` raised. The half-built namespace is released first, so a
/// refused copy leaves the session as it was.
pub(crate) fn deepcopy_namespace(vm: &mut VM<'_>, from: ScopeId) -> RunResult<ScopeId> {
    let Some(to) = vm.parked.create(vm.scope) else {
        return Err(SimpleException::new_msg(
            ExcType::RuntimeError,
            "this session already holds as many namespaces as a handle can address".to_owned(),
        )
        .into());
    };
    let donor: Vec<Value> = match values_of(from, vm) {
        Ok(values) => values.iter().map(|value| value.clone_with_heap(vm.heap)).collect(),
        Err(e) => {
            vm.parked.release(to, vm.heap.heap_mut());
            return Err(e);
        }
    };
    let mut memo = Memo {
        done: AHashMap::new(),
        guest: Value::Ref(vm.heap.allocate(HeapData::Dict(Dict::new()))),
    };
    let mut copied: Vec<Value> = Vec::with_capacity(donor.len());
    let mut failed = None;
    for value in &donor {
        match copy_value(vm, value, from, to, &mut memo) {
            Ok(copy) => copied.push(copy),
            Err(e) => {
                failed = Some(e);
                break;
            }
        }
    }
    memo.release(vm);
    donor.drop_with(vm);
    if let Some(e) = failed {
        copied.drop_with(vm);
        vm.parked.release(to, vm.heap.heap_mut());
        return Err(e);
    }
    vm.parked.put_over(to, copied);
    Ok(to)
}

/// One value's copy, counted.
fn copy_value(vm: &mut VM<'_>, value: &Value, from: ScopeId, to: ScopeId, memo: &mut Memo) -> RunResult<Value> {
    match value {
        // A function defined in the donor is the same code against the copy's
        // globals; one defined anywhere else keeps reading its own.
        Value::DefFunction(func_id, scope) if *scope == from => Ok(Value::DefFunction(*func_id, to)),
        Value::Ref(id) => copy_ref(vm, *id, from, to, memo),
        // Everything else is immediate: it holds nothing and shares nothing.
        other => Ok(other.clone_with_heap(vm.heap)),
    }
}

/// The copy of a heap value, counted, memoized under its donor id.
fn copy_ref(vm: &mut VM<'_>, id: HeapId, from: ScopeId, to: ScopeId, memo: &mut Memo) -> RunResult<Value> {
    if let Some(copy) = memo.done.get(&id) {
        return Ok(copy.clone_with_heap(vm.heap));
    }
    if let Some(copy) = deep_copy_dunder(vm, id, memo)? {
        let handed = copy.clone_with_heap(vm.heap);
        memo.done.insert(id, copy);
        return Ok(handed);
    }
    if shares(vm, id, from) {
        vm.heap.inc_ref(id);
        return Ok(Value::Ref(id));
    }
    if !can_copy(vm, id) {
        let shape = vm.heap.get(id).py_type();
        let named = shape.name(vm.heap, vm.interns);
        return Err(cannot(&format!("a {named}")));
    }
    let new_id = placeholder(vm);
    // Recorded before the children are copied, so a value that reaches itself
    // closes on this copy rather than recurring forever.
    memo.done.insert(id, Value::Ref(new_id));
    fill(vm, id, new_id, from, to, memo)?;
    vm.heap.inc_ref(new_id);
    Ok(Value::Ref(new_id))
}

/// The value `__deepcopy__` answered, or nothing when the class defines none.
fn deep_copy_dunder(vm: &mut VM<'_>, id: HeapId, memo: &Memo) -> RunResult<Option<Value>> {
    if !matches!(vm.heap.get(id), HeapData::Instance(_)) {
        return Ok(None);
    }
    let args = ArgValues::One(memo.guest.clone_with_heap(vm.heap));
    instance_call_dunder_sync(id, "__deepcopy__", args, vm)
}

/// Whether the donor and its copy may hold this same object.
///
/// True for what has no state to diverge, and for what was defined outside the
/// donor: a class or a closure from elsewhere is as shared afterwards as an
/// imported module is.
fn shares(vm: &VM<'_>, id: HeapId, from: ScopeId) -> bool {
    match vm.heap.get(id) {
        // Immutable through and through: copying one could not be observed.
        HeapData::Str(_)
        | HeapData::Bytes(_)
        | HeapData::LongInt(_)
        | HeapData::Date(_)
        | HeapData::DateTime(_)
        | HeapData::TimeDelta(_)
        | HeapData::TimeZone(_)
        | HeapData::Path(_)
        | HeapData::Range(_)
        | HeapData::Module(_)
        | HeapData::ExtFunction(_) => true,
        // Immutable and reaching nothing, so no copy of it could be told apart
        // from it; this is also what keeps `()` the one interned empty tuple.
        HeapData::Tuple(tuple) => !tuple.contains_refs(),
        // A handle onto some other namespace is a reference out of the donor,
        // as a module is. A handle onto the donor is not, and is copied below.
        HeapData::Namespace(handle) => handle.scope() != from,
        HeapData::Class(class) => class.scope() != from,
        HeapData::Closure(closure) => closure.scope != from,
        HeapData::FunctionDefaults(defaults) => defaults.scope != from,
        _ => false,
    }
}

/// Whether a deep copy can carry this shape at all.
///
/// Asked before anything is allocated, so a refusal costs nothing and names the
/// value that could not be carried.
fn can_copy(vm: &VM<'_>, id: HeapId) -> bool {
    matches!(
        vm.heap.get(id),
        HeapData::List(_)
            | HeapData::Tuple(_)
            | HeapData::Dict(_)
            | HeapData::Set(_)
            | HeapData::FrozenSet(_)
            | HeapData::Cell(_)
            | HeapData::Instance(_)
            | HeapData::Dataclass(_)
            | HeapData::Class(_)
            | HeapData::Closure(_)
            | HeapData::FunctionDefaults(_)
            | HeapData::BoundMethod(_)
            | HeapData::Namespace(_)
    )
}

/// The entry a copy is built in, holding nothing.
///
/// Deliberately not a value of the shape being copied: a placeholder that named
/// its original's children would be an entry whose references nothing counted,
/// and a collection running between here and the swap would see those children
/// as one reference short of their true reach. An empty tuple reaches nothing,
/// so there is no window in which the heap is lying. What it becomes is settled
/// by the swap in [`fill`], before anything but the memo can reach it.
fn placeholder(vm: &mut VM<'_>) -> HeapId {
    vm.heap.allocate(HeapData::Tuple(Tuple::default()))
}

/// What a value is made of, read out of the donor and counted, so the copy
/// walk never holds a borrow on the entry it is about to replace.
enum Parts {
    List(Vec<Value>),
    Tuple(Vec<Value>),
    Dict(Vec<(Value, Value)>),
    Set(Vec<Value>),
    FrozenSet(Vec<Value>),
    Cell(Value),
    Instance {
        class: Value,
        attrs: Vec<(Value, Value)>,
    },
    Class {
        name: EitherStr,
        exc_base: Option<ExcType>,
        bases: Vec<Value>,
        members: Vec<(Value, Value)>,
    },
    Closure {
        func_id: FunctionId,
        cells: Vec<Value>,
        defaults: Vec<Value>,
    },
    Defaults {
        func_id: FunctionId,
        defaults: Vec<Value>,
    },
    Bound {
        instance: Value,
        func: Value,
    },
    /// A handle onto the donor itself, which the copy holds onto itself: the
    /// same turn a list that contains itself takes.
    Namespace {
        owns: bool,
    },
    Dataclass {
        name: String,
        type_id: u64,
        field_names: Vec<String>,
        attrs: Vec<(Value, Value)>,
        frozen: bool,
    },
}

/// Reads what `old` is made of. Every value handed back is counted.
fn parts_of(vm: &VM<'_>, old: HeapId) -> Option<Parts> {
    Some(match vm.heap.get(old) {
        HeapData::List(list) => Parts::List(held(list.as_slice(), vm)),
        HeapData::Tuple(tuple) => Parts::Tuple(held(tuple.as_slice(), vm)),
        HeapData::Dict(dict) => Parts::Dict(held_pairs(dict.into_iter(), vm)),
        HeapData::Set(set) => Parts::Set(held_each(set.storage().iter(), vm)),
        HeapData::FrozenSet(frozen) => Parts::FrozenSet(held_each(frozen.storage().iter(), vm)),
        HeapData::Cell(cell) => Parts::Cell(cell.0.clone_with_heap(vm.heap)),
        HeapData::Instance(instance) => {
            let class = instance.class();
            vm.heap.inc_ref(class);
            Parts::Instance {
                class: Value::Ref(class),
                attrs: held_pairs(instance.attrs().into_iter(), vm),
            }
        }
        HeapData::Class(class) => Parts::Class {
            name: class.name().clone(),
            exc_base: class.exc_base(),
            bases: held(class.bases(), vm),
            members: held_pairs(class.namespace().into_iter(), vm),
        },
        HeapData::Closure(closure) => Parts::Closure {
            func_id: closure.func_id,
            cells: closure
                .cells
                .iter()
                .map(|id| {
                    vm.heap.inc_ref(*id);
                    Value::Ref(*id)
                })
                .collect(),
            defaults: held(&closure.defaults, vm),
        },
        HeapData::FunctionDefaults(fd) => Parts::Defaults {
            func_id: fd.func_id,
            defaults: held(&fd.defaults, vm),
        },
        HeapData::Namespace(handle) => Parts::Namespace { owns: handle.owns() },
        HeapData::Dataclass(dc) => Parts::Dataclass {
            name: dc.name(vm.interns).to_owned(),
            type_id: dc.type_id(),
            field_names: dc.field_names().to_vec(),
            attrs: held_pairs(dc.attrs().into_iter(), vm),
            frozen: dc.is_frozen(),
        },
        HeapData::BoundMethod(bm) => Parts::Bound {
            instance: bm.instance.clone_with_heap(vm.heap),
            func: bm.func.clone_with_heap(vm.heap),
        },
        _ => return None,
    })
}

/// Copies `old`'s children and swaps the finished payload into `new`.
fn fill(vm: &mut VM<'_>, old: HeapId, new: HeapId, from: ScopeId, to: ScopeId, memo: &mut Memo) -> RunResult<()> {
    let Some(parts) = parts_of(vm, old) else {
        return Err(RunError::internal("fill was handed a shape empty_like refused"));
    };
    let made = match parts {
        Parts::List(items) => HeapData::List(List::new(copy_each(vm, items, from, to, memo)?)),
        Parts::Tuple(items) => HeapData::Tuple(Tuple::new(copy_each(vm, items, from, to, memo)?.into_iter().collect())),
        Parts::Dict(pairs) => HeapData::Dict(copy_dict(vm, pairs, from, to, memo)?),
        Parts::Set(items) => HeapData::Set(copy_set(vm, items, from, to, memo)?),
        Parts::FrozenSet(items) => HeapData::FrozenSet(FrozenSet::from_set(copy_set(vm, items, from, to, memo)?)),
        Parts::Cell(held) => HeapData::Cell(CellValue(copy_one(vm, held, from, to, memo)?)),
        Parts::Instance { class, attrs } => {
            // `into_ref_id` rather than a match on the variant: taking the id
            // out of a `Value` is taking its count too, and only this hands the
            // wrapper over instead of leaving it to Rust's drop.
            let Some(class) = copy_one(vm, class, from, to, memo)?.into_ref_id() else {
                return Err(RunError::internal("a class copies to a class"));
            };
            let attrs = match copy_dict(vm, attrs, from, to, memo) {
                Ok(attrs) => attrs,
                Err(e) => {
                    vm.heap.dec_ref(class);
                    return Err(e);
                }
            };
            HeapData::Instance(Box::new(Instance::new(class, attrs)))
        }
        Parts::Class {
            name,
            exc_base,
            bases,
            members,
        } => {
            let bases = copy_each(vm, bases, from, to, memo)?;
            let members = match copy_dict(vm, members, from, to, memo) {
                Ok(members) => members,
                Err(e) => {
                    bases.drop_with(vm);
                    return Err(e);
                }
            };
            HeapData::Class(Box::new(Class::new(name, members, bases, exc_base, to)))
        }
        Parts::Closure {
            func_id,
            cells,
            defaults,
        } => {
            // The captured cells are copied, so what the copy's function writes
            // through a capture is not what the donor's reads.
            let cells = copy_each(vm, cells, from, to, memo)?;
            let defaults = match copy_each(vm, defaults, from, to, memo) {
                Ok(defaults) => defaults,
                Err(e) => {
                    cells.drop_with(vm);
                    return Err(e);
                }
            };
            let mut ids = Vec::with_capacity(cells.len());
            let mut wrong = false;
            for cell in cells {
                match cell.into_ref_id() {
                    Some(id) => ids.push(id),
                    None => wrong = true,
                }
            }
            if wrong {
                for id in ids {
                    vm.heap.dec_ref(id);
                }
                defaults.drop_with(vm);
                return Err(RunError::internal("a cell copies to a cell"));
            }
            HeapData::Closure(Closure {
                func_id,
                scope: to,
                cells: ids,
                defaults,
            })
        }
        Parts::Defaults { func_id, defaults } => HeapData::FunctionDefaults(FunctionDefaults {
            func_id,
            scope: to,
            defaults: copy_each(vm, defaults, from, to, memo)?,
        }),
        Parts::Namespace { owns } => HeapData::Namespace(if owns {
            NamespaceRef::new(to)
        } else {
            NamespaceRef::borrowed(to)
        }),
        Parts::Dataclass {
            name,
            type_id,
            field_names,
            attrs,
            frozen,
        } => HeapData::Dataclass(Box::new(Dataclass::new(
            name,
            type_id,
            field_names,
            copy_dict(vm, attrs, from, to, memo)?,
            frozen,
        ))),
        Parts::Bound { instance, func } => {
            let instance = copy_one(vm, instance, from, to, memo)?;
            let func = match copy_one(vm, func, from, to, memo) {
                Ok(func) => func,
                Err(e) => {
                    instance.drop_with(vm);
                    return Err(e);
                }
            };
            HeapData::BoundMethod(BoundMethod { instance, func })
        }
    };
    // The placeholder reached nothing, so what comes back owns no reference and
    // needs no release.
    let _ = vm.heap.heap_mut().replace_data(new, made);
    Ok(())
}

/// Counted copies of `values`, for reading them out of an entry that the copy
/// walk is about to mutate.
fn held(values: &[Value], vm: &VM<'_>) -> Vec<Value> {
    held_each(values.iter(), vm)
}

fn held_each<'a>(values: impl Iterator<Item = &'a Value>, vm: &VM<'_>) -> Vec<Value> {
    values.map(|value| value.clone_with_heap(vm.heap)).collect()
}

fn held_pairs<'a>(pairs: impl Iterator<Item = (&'a Value, &'a Value)>, vm: &VM<'_>) -> Vec<(Value, Value)> {
    pairs
        .map(|(k, v)| (k.clone_with_heap(vm.heap), v.clone_with_heap(vm.heap)))
        .collect()
}

/// Copies one held value, releasing the original.
fn copy_one(vm: &mut VM<'_>, value: Value, from: ScopeId, to: ScopeId, memo: &mut Memo) -> RunResult<Value> {
    let made = copy_value(vm, &value, from, to, memo);
    value.drop_with(vm);
    made
}

/// Copies each held value in order, releasing the originals.
fn copy_each(
    vm: &mut VM<'_>,
    values: Vec<Value>,
    from: ScopeId,
    to: ScopeId,
    memo: &mut Memo,
) -> RunResult<Vec<Value>> {
    let mut out = Vec::with_capacity(values.len());
    let mut failed = None;
    for value in &values {
        match copy_value(vm, value, from, to, memo) {
            Ok(copy) => out.push(copy),
            Err(e) => {
                failed = Some(e);
                break;
            }
        }
    }
    values.drop_with(vm);
    if let Some(e) = failed {
        out.drop_with(vm);
        return Err(e);
    }
    Ok(out)
}

/// A dict of the copied pairs, inserted rather than rewritten so every key
/// lands in the bucket its own hash names.
fn copy_dict(
    vm: &mut VM<'_>,
    pairs: Vec<(Value, Value)>,
    from: ScopeId,
    to: ScopeId,
    memo: &mut Memo,
) -> RunResult<Dict> {
    let mut built = Dict::new();
    for (key, value) in &pairs {
        let made = copy_value(vm, key, from, to, memo).and_then(|key| match copy_value(vm, value, from, to, memo) {
            Ok(value) => Ok((key, value)),
            Err(e) => {
                key.drop_with(vm);
                Err(e)
            }
        });
        match made {
            Ok((key, value)) => match built.set(key, value, vm) {
                Ok(replaced) => replaced.drop_with(vm),
                Err(e) => {
                    built.drop_with(vm);
                    pairs.drop_with(vm);
                    return Err(e);
                }
            },
            Err(e) => {
                built.drop_with(vm);
                pairs.drop_with(vm);
                return Err(e);
            }
        }
    }
    pairs.drop_with(vm);
    Ok(built)
}

/// A set of the copied values, added rather than rewritten, for the reason
/// [`copy_dict`] inserts.
fn copy_set(vm: &mut VM<'_>, items: Vec<Value>, from: ScopeId, to: ScopeId, memo: &mut Memo) -> RunResult<Set> {
    let copies = copy_each(vm, items, from, to, memo)?;
    let mut built = Set::new();
    for value in copies {
        if let Err(e) = built.add(value, vm) {
            built.drop_with(vm);
            return Err(e);
        }
    }
    Ok(built)
}
