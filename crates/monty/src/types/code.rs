//! The code object `compile()` answers and `eval()` / `exec()` take.

use std::{
    collections::hash_map::DefaultHasher,
    fmt::Write,
    hash::{Hash, Hasher},
};

use super::LazyHeapSet;
use crate::{
    bytecode::VM,
    exception_private::RunResult,
    hash::HashValue,
    heap::{HeapId, HeapItem, HeapObjectRead, HeapReadOutput},
    types::{PyTrait, Type},
    value::Value,
};

/// The mode a code object was compiled in, which decides how its source parses.
///
/// `'single'` is not here: it would have to echo the value of an expression
/// statement through `sys.displayhook`, which Monty has no equivalent of, so
/// `compile()` refuses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(crate) enum CodeMode {
    /// `'exec'`: the source is a module body.
    Exec,
    /// `'eval'`: the source is one expression.
    Eval,
}

/// A compiled code object.
///
/// Monty compiles a snippet against the namespace it will run in, and that
/// namespace is only known at `eval()` / `exec()`, so a code object cannot
/// carry bytecode: it carries the source `compile()` validated, the name of the
/// file it came from, and the mode it was compiled in, and the real compilation
/// happens where it runs. A syntax error therefore still reaches the caller of
/// `compile()`, which is what the two are usually split for, but the source is
/// parsed once more at each run. See `limitations/eval_exec.md`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Code {
    source: Box<str>,
    filename: Box<str>,
    mode: CodeMode,
}

impl Code {
    /// Creates a code object over source `compile()` has already parsed.
    #[must_use]
    pub fn new(source: Box<str>, filename: Box<str>, mode: CodeMode) -> Self {
        Self { source, filename, mode }
    }

    /// The source this code object runs.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The file name `compile()` was given, which names the frames of this code
    /// in a traceback.
    #[must_use]
    pub fn filename(&self) -> &str {
        &self.filename
    }

    /// The mode this code object was compiled in.
    #[must_use]
    pub fn mode(&self) -> CodeMode {
        self.mode
    }
}

impl<'h> PyTrait<'h> for HeapObjectRead<'h, Code> {
    fn py_type(&self, _vm: &VM<'h>) -> Type {
        Type::Code
    }

    fn py_len(&self, _vm: &VM<'h>) -> Option<usize> {
        None
    }

    fn py_bool(&self, _vm: &mut VM<'h>) -> RunResult<bool> {
        Ok(true)
    }

    /// Two code objects are equal when they run the same source in the same
    /// mode. CPython compares the compiled code, which also ignores the file
    /// name but reads two spellings of one program as equal; see
    /// `limitations/eval_exec.md`.
    fn py_eq_impl(&self, other: &Value, vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        let Some(HeapReadOutput::Code(other)) = other.read_heap(vm) else {
            return Ok(None);
        };
        let a = self.get(vm.heap);
        let b = other.get(vm.heap);
        Ok(Some(a.source == b.source && a.mode == b.mode))
    }

    fn py_hash(&self, vm: &mut VM<'h>) -> RunResult<Option<HashValue>> {
        let mut hasher = DefaultHasher::new();
        let code = self.get(vm.heap);
        code.source.hash(&mut hasher);
        code.mode.hash(&mut hasher);
        Ok(Some(HashValue::new(hasher.finish())))
    }

    /// CPython writes `<code object <module> at 0x.., file "f.py", line 1>`.
    /// Monty has no line for a whole snippet and no address to show, so it
    /// names the file alone (see `limitations/eval_exec.md`).
    fn py_repr_fmt(&self, f: &mut impl Write, vm: &mut VM<'h>, _heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        Ok(write!(
            f,
            "<code object <module>, file \"{}\">",
            self.get(vm.heap).filename()
        )?)
    }
}

impl HeapItem for Code {
    fn py_dec_ref_ids(&mut self, _stack: &mut Vec<HeapId>) {
        // A code object holds text alone, no heap references.
    }
}
