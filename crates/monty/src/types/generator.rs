//! Generator and coroutine objects: a function body frozen between two words.
//!
//! A generator is the activation of a function whose body yields, and a
//! coroutine is the activation of an `async def`. Both are the same saved
//! frame, and they differ only in the word they stop at: a `yield` or an
//! `await`. Calling such a function binds its arguments and hands back one of
//! these, unstarted; each resume puts the frame back on the VM's own frame
//! stack, runs it until the next stop, and lifts the frame off again. See
//! [`VM::resume_generator`](crate::bytecode::VM::resume_generator) and
//! [`GeneratorKind`].

use std::fmt::Write;

use super::{LazyHeapSet, PyTrait, Type, py_trait::PyObjectIdentity};
use crate::{
    args::ArgValues,
    bytecode::{CallResult, FrameNamespace, VM},
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunError, RunResult},
    hash::{HashValue, identity_hash},
    heap::{DropWithContext, HeapId, HeapItem, HeapObjectRead},
    intern::FunctionId,
    value::{EitherStr, Value},
};

/// Where a generator is in its life.
///
/// The states are CPython's, and they are what the re-entrancy and exhaustion
/// errors are answered from: resuming a `Running` generator is the "already
/// executing" error, and resuming a `Done` one raises `StopIteration`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum GeneratorState {
    /// Created by the call, never resumed. `stack` holds the bound arguments.
    Created,
    /// Suspended at a `yield`. `stack` holds the frame's locals and operands.
    Suspended,
    /// Its frame is on the VM's frame stack right now.
    Running,
    /// Returned, raised, or was closed. `stack` is empty.
    Done,
}

/// Where a generator suspended inside a `yield from` stands.
///
/// The delegation loop leaves the receiver under the value it hands out, so a
/// generator that carries one of these has its receiver on top of its saved
/// stack. `close()` and `throw()` read it to reach that receiver, which is what
/// makes an exception, and an exit, travel to the innermost body as they do in
/// CPython.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(crate) struct Delegation {
    /// The `Yield` the loop suspended on. A frame put back here hands the
    /// receiver's next value onwards, rather than sending to it again.
    pub yield_ip: usize,

    /// Where the frame goes once the receiver is finished, which is what the
    /// receiver's return value is left at.
    pub done_ip: usize,
}

/// Which of the two objects a saved frame is.
///
/// A generator and a coroutine are one thing suspended between two different
/// words: a generator stops at a `yield` and a coroutine at an `await`. CPython
/// gives them one frame layout and two type objects, and so does this. The kind
/// decides the type name, the repr, and which methods the object answers, and
/// nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum GeneratorKind {
    /// A call to a function whose body yields.
    Generator,
    /// A call to an `async def` function.
    Coroutine,
}

impl GeneratorKind {
    /// What a resume of a body that has finished raises.
    ///
    /// A generator is simply exhausted, so it raises `StopIteration` as an
    /// iterator does. A coroutine is spent: CPython refuses to let anything
    /// reach one twice, whether it returned, raised, or was closed.
    pub fn spent(self) -> RunError {
        match self {
            Self::Generator => ExcType::stop_iteration(),
            Self::Coroutine => ExcType::cannot_reuse_already_awaited_coroutine(),
        }
    }

    /// What a refusal calls this, which is the name of its type.
    pub fn word(self) -> &'static str {
        match self {
            Self::Generator => "generator",
            Self::Coroutine => "coroutine",
        }
    }
}

/// A generator object: everything needed to put its frame back on the stack.
///
/// The saved state is exactly what a frame is, and no more. `ip` is relative to
/// the body's own bytecode, so it needs no fixing up; `stack_base` and
/// `exception_stack_base` are absolute and therefore deliberately absent, being
/// recomputed from wherever the frame is spliced in next. There is no saved
/// block state because exception handling reads a static table keyed by `ip`,
/// so where the frame stands inside its `try` blocks is implied by `ip` alone.
///
/// A coroutine is one of these too; see [`GeneratorKind`].
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Generator {
    /// The function whose body this runs. Its `namespace_size` is the frame's
    /// locals count, so that is derived rather than stored.
    pub func_id: FunctionId,

    /// Whether this is a generator or a coroutine; see [`GeneratorKind`].
    pub kind: GeneratorKind,

    /// Where in that body to resume; `0` before the first resume.
    pub ip: usize,

    /// The frame's stack region: `locals` first, then any operands left in the
    /// middle of the expression the `yield` sits in. Owned values.
    ///
    /// Before the first resume this is the bound arguments alone, which is the
    /// same thing — a frame's locals are its stack region's head.
    pub stack: Vec<Value>,

    /// The frame's slice of the VM-wide exception stack: the exceptions being
    /// handled by `except` blocks the `yield` is inside. Owned values.
    pub exception_stack: Vec<Value>,

    /// Where it is in its life; see [`GeneratorState`].
    pub state: GeneratorState,

    /// The namespace the body resolves names through, owned by this object as
    /// a frame owns its own; `None` when locals are stack slots and globals are
    /// module slots, which is every ordinary function.
    pub namespace: Option<Box<FrameNamespace>>,

    /// The `yield from` this is suspended inside, if it is inside one; see
    /// [`Delegation`]. Set at every suspend, so it never speaks of a
    /// delegation that is over.
    pub delegating: Option<Delegation>,
}

impl Generator {
    /// Creates an unstarted generator or coroutine holding its call's bound
    /// arguments.
    ///
    /// `locals` is the frame's stack region, filled the way a call fills it,
    /// and `namespace` is already inc_ref'd for this object.
    pub fn new(
        kind: GeneratorKind,
        func_id: FunctionId,
        locals: Vec<Value>,
        namespace: Option<Box<FrameNamespace>>,
    ) -> Self {
        Self {
            func_id,
            kind,
            ip: 0,
            stack: locals,
            exception_stack: Vec::new(),
            state: GeneratorState::Created,
            namespace,
            delegating: None,
        }
    }

    /// The type this object reports, which is the one thing its kind decides
    /// about how the rest of the interpreter sees it.
    pub fn py_type(&self) -> Type {
        match self.kind {
            GeneratorKind::Generator => Type::Generator,
            GeneratorKind::Coroutine => Type::Coroutine,
        }
    }
}

impl HeapItem for Generator {
    fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        // Mirrors the GC's child walk in `heap/mod.rs`: the saved frame owns
        // every value in both regions, plus every id of its namespace.
        for value in &mut self.stack {
            value.py_dec_ref_ids(stack);
        }
        for value in &mut self.exception_stack {
            value.py_dec_ref_ids(stack);
        }
        if let Some(namespace) = self.namespace.take() {
            stack.extend(namespace.owned_ids());
        }
    }
}

impl<'h> PyTrait<'h> for HeapObjectRead<'h, Generator> {
    fn py_type(&self, vm: &VM<'h>) -> Type {
        self.get(vm.heap).py_type()
    }

    /// A generator is its own iterator, which is what `iter()` checks for. A
    /// coroutine is not one: it is awaited, never iterated.
    fn py_is_iterable(&self, vm: &VM<'h>) -> bool {
        self.get(vm.heap).kind == GeneratorKind::Generator
    }

    fn py_is_iterator(&self, vm: &VM<'h>) -> bool {
        self.get(vm.heap).kind == GeneratorKind::Generator
    }

    fn py_iter(&self, vm: &mut VM<'h>) -> RunResult<Value> {
        if self.get(vm.heap).kind == GeneratorKind::Coroutine {
            let name = Type::Coroutine.name(vm.heap, vm.interns).into_owned();
            return Err(ExcType::type_error_not_iterable(&name));
        }
        Ok(self.clone_value(vm.heap))
    }

    /// Steps the generator from Rust, for the builtins that walk an iterator
    /// themselves; see [`VM::drive_generator`].
    fn py_next(&mut self, vm: &mut VM<'h>) -> RunResult<Option<Value>> {
        if self.get(vm.heap).kind == GeneratorKind::Coroutine {
            let name = Type::Coroutine.name(vm.heap, vm.interns).into_owned();
            return Err(ExcType::type_error_not_iterator(&name));
        }
        let self_id = self.id();
        vm.drive_generator(self_id, Value::None)
    }

    fn py_len(&self, _vm: &VM<'h>) -> Option<usize> {
        None
    }

    fn py_eq_impl(&self, _other: &Value, _vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        Ok(None)
    }

    fn py_hash(&self, _vm: &mut VM<'h>) -> RunResult<Option<HashValue>> {
        Ok(Some(identity_hash(self.id())))
    }

    fn py_bool(&self, _vm: &mut VM<'h>) -> RunResult<bool> {
        Ok(true)
    }

    fn py_repr_fmt(&self, f: &mut impl Write, vm: &mut VM<'h>, _heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        let kind = match self.get(vm.heap).kind {
            GeneratorKind::Generator => "generator",
            GeneratorKind::Coroutine => "coroutine",
        };
        let name = vm.interns.get_function(self.get(vm.heap).func_id).name.name_id;
        let name = vm.interns.get_str(name);
        Ok(write!(
            f,
            "<{kind} object {name} at 0x{:x}>",
            self.py_identity().encoded()
        )?)
    }

    fn py_call_attr(&mut self, vm: &mut VM<'h>, attr: &EitherStr, args: ArgValues) -> RunResult<CallResult> {
        let self_id = self.id();
        let kind = self.get(vm.heap).kind;
        // A coroutine is awaited, never iterated, so it answers neither of the
        // iterator methods; the three that drive a frame it answers as a
        // generator does.
        let iterating = kind == GeneratorKind::Generator;
        match attr.as_str(vm.interns) {
            "__iter__" if iterating => {
                args.check_zero_args("__iter__", vm.heap)?;
                vm.heap.inc_ref(self_id);
                Ok(CallResult::Value(Value::Ref(self_id)))
            }
            // Both resume the body; `__next__` is `send(None)` by another name.
            "__next__" if iterating => {
                args.check_zero_args("__next__", vm.heap)?;
                vm.resume_generator(self_id, Value::None)
            }
            "send" => {
                let value = args.get_one_arg("send", vm.heap)?;
                vm.resume_generator(self_id, value)
            }
            // Ends the body where it stands, running its `finally` blocks.
            "close" => {
                args.check_zero_args("close", vm.heap)?;
                vm.close_generator(self_id).map(|()| CallResult::Value(Value::None))
            }
            // Raises at the `yield`, so the body's own handlers get their turn.
            "throw" => {
                let value = args.get_one_arg("throw", vm.heap)?;
                defer_drop!(value, vm);
                vm.throw_into_generator(self_id, value)
            }
            other => {
                let other = other.to_owned();
                args.drop_with(vm);
                let name = self.get(vm.heap).py_type().name(vm.heap, vm.interns).into_owned();
                Err(ExcType::attribute_error(name, &other))
            }
        }
    }
}
