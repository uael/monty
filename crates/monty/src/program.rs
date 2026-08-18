//! The symbol space every compilation in one session extends.

use crate::{
    intern::{InternerBuilder, Interns},
    name_map::NameMap,
};

/// Names to slots, and everything interned: strings, bytes, big integers and
/// compiled functions.
///
/// Append-only, and owned in exactly one place. Every id the interpreter
/// stores is an index into one of these tables, so an id's meaning is whatever
/// currently sits at that index. Two tables grown from one base put different
/// entries at the same index, and no later merge can reconcile them, because a
/// value already on the heap carries the id and not the entry: reconciling
/// would mean rewriting every `StringId` in the heap, the bytecode and the
/// name map. So a compile extends this in place and commits before the snippet
/// it compiled ever runs, which is what makes an interleaved compile
/// impossible to lose.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Program {
    /// Module-level name to slot. One map serves every namespace over the
    /// session's heap: slot `k` names the same variable in all of them, and a
    /// namespace that never bound it holds `Undefined` there.
    pub(crate) globals: NameMap,
    /// Interned strings, bytes, big integers, and the compiled body of every
    /// function the session has defined.
    pub(crate) interns: Interns,
}

impl Program {
    /// An empty symbol space, before anything has been compiled into it.
    pub(crate) fn new() -> Self {
        Self {
            globals: NameMap::new(),
            interns: Interns::new(InternerBuilder::default(), Vec::new()),
        }
    }
}
