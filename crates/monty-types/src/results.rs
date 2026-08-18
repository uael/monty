//! Results crossing the sandbox boundary: what the host feeds back into a
//! suspended run ([`NameLookupResult`], [`ExtFunctionResult`]), and what one
//! snippet leaves behind ([`FeedOutcome`], [`ParseFacts`]).

use crate::{exceptions::MontyException, object::MontyObject};
/// Result of a name lookup from the host.
///
/// When the VM encounters an unresolved name, the host provides one of these:
/// - `Value(obj)`: The name resolves to this value (cached in the namespace for future access).
/// - `Undefined`: The name is truly undefined, causing `NameError`.
#[derive(Debug)]
pub enum NameLookupResult {
    /// The name resolves to this value.
    Value(MontyObject),
    /// The name is undefined — VM will raise `NameError`.
    Undefined,
}

impl From<MontyObject> for NameLookupResult {
    fn from(value: MontyObject) -> Self {
        Self::Value(value)
    }
}

/// Return value or exception from an external function.
#[derive(Debug)]
pub enum ExtFunctionResult {
    /// Continues execution with the return value from the external function.
    Return(MontyObject),
    /// Continues execution with the exception raised by the external function.
    Error(MontyException),
    /// Pending future — the external function is a coroutine.
    ///
    /// The `u32` is the `call_id` from the `FunctionCall` that created this
    /// snapshot. It is used to track the pending future so it can be resolved
    /// later via `ResolveFutures::resume()`.
    Future(u32),
    /// The function was not found, should result in a `NameError` exception.
    NotFound(String),
}
impl From<MontyObject> for ExtFunctionResult {
    fn from(value: MontyObject) -> Self {
        Self::Return(value)
    }
}

impl From<MontyException> for ExtFunctionResult {
    fn from(exception: MontyException) -> Self {
        Self::Error(exception)
    }
}

/// What one fed snippet produced.
///
/// CPython rejects a module-level `return` at compile time, so a host that
/// wanted one had to rewrite the snippet's AST and smuggle the value out
/// through an exception. Monty runs it and reports it here instead, which is
/// the only way to tell a snippet that closed itself from one that merely ran
/// out of statements: both hand back a value.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FeedOutcome {
    /// What a written `return` handed back, or the value of a trailing
    /// expression, or `None` for a snippet that ended on neither.
    pub value: MontyObject,
    /// Whether a written module-level `return` ended the snippet.
    pub returned: bool,
}

/// What reading a snippet says about it, with none of it run.
///
/// A host driving a session classifies source with this instead of carrying
/// its own Python parser: whether the text is finished, what is wrong with it
/// if not, and which bindings it makes.
#[derive(Debug, Clone)]
pub struct ParseFacts {
    /// False only when the snippet is unfinished rather than wrong: an open
    /// bracket, an unterminated triple-quoted string, or a block header with
    /// no body. That is a request for more input, so `error` is then `None`.
    /// This is the line CPython's `codeop.compile_command` draws for an
    /// interactive prompt.
    pub complete: bool,
    /// The syntax error the snippet would raise if fed. `None` when the
    /// snippet parses, and also when it is merely unfinished.
    pub error: Option<MontyException>,
    /// Whether a `global` statement appears anywhere in the snippet, in any
    /// scope.
    pub binds_global: bool,
    /// Which of the names the caller asked about the snippet binds at module
    /// level, in the order asked.
    pub stores: Vec<String>,
}
