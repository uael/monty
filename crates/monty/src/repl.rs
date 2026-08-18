//! Stateful REPL execution support for Monty.
//!
//! This module implements incremental snippet execution where each new snippet
//! is compiled and executed against persistent heap/namespace state without
//! replaying previously executed snippets.

use std::mem;

use ahash::AHashMap;
use monty_types::{ExcType, FeedOutcome, MontyException, MontyObject, OsFunctionCall, PrintWriter, ResourceTracker};
use ruff_python_ast::token::TokenKind;
use ruff_python_parser::{InterpolatedStringErrorType, LexicalErrorType, ParseError, ParseErrorType, parse_module};

use crate::{
    args::{ArgValues, KwargsValues},
    asyncio::CallId,
    bytecode::{VM, VMSnapshot},
    defer_drop,
    exception_private::{ExcTypeExt, RunError},
    heap::{DropWithContext, Heap, HeapData, HeapReader},
    intern::{InternerBuilder, Interns},
    name_map::NameMap,
    object_bridge::MontyObjectExt,
    parse::check_probe_expression,
    run::{CompileOptions, Executor},
    types::PyTrait,
    run_progress::{ConvertedExit, ExtFunctionResult, ExtFunctionResultExt, NameLookupResult, convert_frame_exit},
    value::Value,
};

/// Stateful REPL session that executes snippets incrementally without replay.
///
/// `MontyRepl` preserves heap and global variable state between snippets.
/// Each `feed()` compiles and executes only the new snippet against the current
/// state, avoiding the cost and semantic risks of replaying prior code.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct MontyRepl {
    /// Script name used for runtime error messages and REPL identification.
    ///
    /// Incremental `feed()` / `start()` snippets intentionally use internal script names
    /// like `<python-input-0>` to match CPython's interactive traceback style.
    script_name: String,
    /// Counter for generated `<python-input-N>` snippet filenames.
    next_input_id: u64,
    /// Stable mapping of global variable names to namespace slot IDs.
    global_names: NameMap,
    /// Persistent intern table across snippets so intern/function IDs remain valid.
    interns: Interns,
    /// Source text of every snippet that has been fed, keyed by its
    /// generated script name (`<python-input-N>`).
    ///
    /// Required because a traceback raised in snippet N can include frames
    /// from functions defined in snippet M < N. Those frames carry
    /// `CodeRange` byte offsets that index into snippet M's source, so the
    /// diagnostic pass must be able to look that source up by filename —
    /// the current snippet's `Executor.code` is not sufficient.
    #[serde(default)]
    sources: AHashMap<String, String>,
    /// [`CompileOptions`] applied to every snippet fed to this session, fixed
    /// at construction so all snippets compile consistently.
    #[serde(default)]
    options: CompileOptions,
    /// Persistent heap across snippets.
    heap: Heap,
    /// Persistent global variable values across snippets.
    ///
    /// Indexed by `NamespaceId` slots from `global_names`. Between snippet
    /// executions these are the only VM values that persist — stack and frames
    /// are transient.
    globals: Vec<Value>,
}

impl MontyRepl {
    /// Creates an empty REPL session with no code parsed or executed.
    ///
    /// All code execution is driven through `feed_run()` or `feed_start()`. This separates
    /// construction from execution, matching the pattern used by `MontyRun::new()`.
    /// The [`CompileOptions`] apply to every snippet fed to the session.
    #[must_use]
    pub fn new(script_name: &str, resource_tracker: ResourceTracker, options: CompileOptions) -> Self {
        let heap = Heap::new(0, resource_tracker);

        Self {
            script_name: script_name.to_owned(),
            next_input_id: 0,
            global_names: NameMap::new(),
            interns: Interns::new(InternerBuilder::default(), Vec::new()),
            sources: AHashMap::new(),
            options,
            heap,
            globals: Vec::new(),
        }
    }

    /// Returns the resource tracker that will be used for the next snippet.
    ///
    /// This is primarily intended for host integrations that need to attach
    /// per-execution state, such as cancellation markers, to an existing REPL.
    pub fn tracker(&self) -> &ResourceTracker {
        &self.heap.tracker
    }

    /// Returns mutable access to the resource tracker for the next snippet.
    ///
    /// REPL hosts use this to install ephemeral execution controls, such as
    /// async cancellation flags, before calling `feed_start()`.
    pub fn tracker_mut(&mut self) -> &mut ResourceTracker {
        &mut self.heap.tracker
    }

    /// Number of live heap entries (excluding the empty-tuple singleton) —
    /// `ref-count-return`-only introspection for GC tests.
    #[cfg(feature = "ref-count-return")]
    #[must_use]
    pub fn heap_entry_count(&self) -> usize {
        self.heap.entry_count()
    }

    /// Starts executing a new snippet and returns suspendable REPL progress.
    ///
    /// This is the REPL equivalent of `MontyRun::start`: execution may complete,
    /// suspend at external calls / OS calls / unresolved futures, or raise a Python
    /// exception. Resume with the returned state object and eventually recover the
    /// updated REPL from `ReplProgress::into_complete`.
    ///
    /// Unlike `MontyRepl::feed`, this method consumes `self` so runtime state can be
    /// safely moved into snapshot objects for serialization and cross-process resume.
    ///
    /// On a Python-level runtime exception the REPL is **not** destroyed: it is
    /// returned inside `ReplStartError` so the caller can continue feeding
    /// subsequent snippets against the same heap and namespace state.
    ///
    /// # Errors
    /// Returns `Err(Box<ReplStartError>)` for syntax, compile-time, or runtime
    /// failures — the REPL session is always preserved inside the error.
    pub fn feed_start(
        self,
        code: &str,
        inputs: Vec<(String, MontyObject)>,
        print: PrintWriter<'_>,
    ) -> Result<ReplProgress, Box<ReplStartError>> {
        self.start(code, inputs, None, print, SnippetKind::Feed)
    }

    /// Starts a snippet under a filename of the caller's choosing, so its
    /// frames say where the source came from; see
    /// [`feed_run_named`](Self::feed_run_named).
    ///
    /// # Errors
    /// As [`feed_start`](Self::feed_start).
    pub fn feed_start_named(
        self,
        code: &str,
        inputs: Vec<(String, MontyObject)>,
        script_name: Option<&str>,
        print: PrintWriter<'_>,
    ) -> Result<ReplProgress, Box<ReplStartError>> {
        self.start(code, inputs, script_name, print, SnippetKind::Feed)
    }

    /// Evaluates one expression against the session's namespace and hands back
    /// its value, binding nothing.
    ///
    /// This is how a host turns words into a value: an annotation, a contract,
    /// a name it wants the meaning of in the scope that defined it. Source that
    /// is not a single expression, or that could bind a name through `:=`, is
    /// refused rather than quietly leaving the session changed; what the
    /// expression *calls* can of course still mutate what it reaches.
    ///
    /// Suspends and resumes exactly as [`feed_start`](Self::feed_start) does,
    /// so an expression naming something the host provides is answered the same
    /// way.
    ///
    /// # Errors
    /// Returns `Err(Box<ReplStartError>)` for a rejected or failing expression;
    /// the session is preserved inside the error.
    pub fn probe_start(self, expr: &str, print: PrintWriter<'_>) -> Result<ReplProgress, Box<ReplStartError>> {
        if let Err(error) = check_probe_expression(expr, &self.script_name) {
            return Err(Box::new(ReplStartError { repl: self, error }));
        }
        self.start(expr, Vec::new(), None, print, SnippetKind::Probe)
    }

    /// Shared body of [`feed_start`](Self::feed_start) and
    /// [`probe_start`](Self::probe_start); `kind` only decides what the
    /// snippet's generated filename says.
    fn start(
        self,
        code: &str,
        inputs: Vec<(String, MontyObject)>,
        script_name: Option<&str>,
        print: PrintWriter<'_>,
        kind: SnippetKind,
    ) -> Result<ReplProgress, Box<ReplStartError>> {
        let mut this = self;
        if code.is_empty() {
            return Ok(ReplProgress::Complete {
                repl: this,
                outcome: FeedOutcome {
                    value: MontyObject::None,
                    returned: false,
                },
            });
        }

        let (input_names, input_values): (Vec<_>, Vec<_>) = inputs.into_iter().unzip();

        let input_script_name = this.snippet_script_name(script_name, kind);
        // Preserve this snippet's source (see `feed_run` for rationale).
        this.sources.insert(input_script_name.clone(), code.to_owned());
        let executor = match Executor::new_repl_snippet(
            code.to_owned(),
            &input_script_name,
            this.global_names.clone(),
            &this.interns,
            &input_names,
            this.options,
        ) {
            Ok(exec) => exec,
            Err(error) => return Err(Box::new(ReplStartError { repl: this, error })),
        };

        this.ensure_globals_size(executor.namespace_size());

        match HeapReader::with(&mut this.heap, &mut (&executor, print), |reader, (executor, print)| {
            let mut vm = VM::new(
                mem::take(&mut this.globals),
                reader,
                &executor.interns,
                print.reborrow(),
                executor.assert_repr_max_bytes,
            );

            // Inject inputs with VM alive
            if let Err(error) = inject_inputs_into_vm(executor, input_values, &mut vm) {
                this.globals = vm.take_globals();
                return Err(error);
            }

            let vm_result = vm.run_module(&executor.module_code);

            // Convert while VM alive, then snapshot or reclaim globals
            let converted = convert_frame_exit(vm_result, &mut vm);
            let vm_state = if converted.needs_snapshot() {
                Some(vm.snapshot())
            } else {
                this.globals = vm.take_globals();
                None
            };
            Ok((converted, vm_state))
        }) {
            Ok((converted, vm_state)) => build_repl_progress(converted, vm_state, executor, this),
            Err(error) => Err(Box::new(ReplStartError { repl: this, error })),
        }
    }

    /// Feeds and executes a new snippet against the current REPL state to completion.
    ///
    /// This compiles only `code` using the existing global slot map, extends the
    /// global namespace if new names are introduced, and executes the snippet once.
    /// Previously executed snippets are never replayed. If execution raises after
    /// partially mutating globals, those mutations remain visible in later feeds,
    /// matching Python REPL semantics.
    ///
    /// # Errors
    /// Returns `MontyException` for syntax/compile/runtime failures.
    pub fn feed_run(
        &mut self,
        code: &str,
        inputs: Vec<(String, MontyObject)>,
        print: PrintWriter<'_>,
    ) -> Result<FeedOutcome, MontyException> {
        self.feed_run_named(code, inputs, None, print)
    }

    /// Feeds and executes a snippet under a filename of the caller's choosing,
    /// so its frames say where the source came from.
    ///
    /// A session's snippets are otherwise named `<python-input-N>`, which is
    /// right for a REPL and wrong for a host feeding chunks that have names of
    /// their own. `None` takes the generated name.
    ///
    /// # Errors
    /// As [`feed_run`](Self::feed_run).
    pub fn feed_run_named(
        &mut self,
        code: &str,
        inputs: Vec<(String, MontyObject)>,
        script_name: Option<&str>,
        print: PrintWriter<'_>,
    ) -> Result<FeedOutcome, MontyException> {
        if code.is_empty() {
            return Ok(FeedOutcome {
                value: MontyObject::None,
                returned: false,
            });
        }

        let (input_names, input_values): (Vec<_>, Vec<_>) = inputs.into_iter().unzip();

        let input_script_name = self.snippet_script_name(script_name, SnippetKind::Feed);
        // Preserve this snippet's source before anything can fail, so later
        // tracebacks with frames from this snippet can still resolve line/
        // column/preview information — `Executor.code` only survives until
        // the next feed.
        self.sources.insert(input_script_name.clone(), code.to_owned());
        let executor = Executor::new_repl_snippet(
            code.to_owned(),
            &input_script_name,
            self.global_names.clone(),
            &self.interns,
            &input_names,
            self.options,
        )?;

        self.ensure_globals_size(executor.namespace_size());

        let result = HeapReader::with(&mut self.heap, &mut (&executor, print), |reader, (executor, print)| {
            let mut vm = VM::new(
                mem::take(&mut self.globals),
                reader,
                &executor.interns,
                print.reborrow(),
                executor.assert_repr_max_bytes,
            );

            if let Err(e) = inject_inputs_into_vm(executor, input_values, &mut vm) {
                self.globals = vm.take_globals();
                return Err(e);
            }

            let result = executor.run_to_completion(&mut vm);
            let returned = vm.module_returned();

            // Reclaim globals before cleanup.
            self.globals = vm.take_globals();
            Ok((result, returned))
        })?;

        // Commit compiler metadata even on runtime errors.
        // Snippets can mutate globals before raising, and those values may contain
        // FunctionId/StringId values that must be interpreted with the updated tables.
        let Executor {
            globals: snippet_globals,
            interns,
            ..
        } = executor;
        self.global_names = snippet_globals;
        self.interns = interns;

        // Resolve every traceback frame against the source of the snippet that
        // produced it — frames from earlier snippets live in `self.sources`.
        let (result, returned) = result;
        result
            .map(|value| FeedOutcome { value, returned })
            .map_err(|e| e.into_python_exception(&self.interns, |fname| self.sources.get(fname).map(String::as_str)))
    }

    /// Calls a Python function defined in the session by name.
    ///
    /// Looks up the function in the global namespace, converts the arguments,
    /// executes the function, and converts the result back.
    ///
    /// # Errors
    /// Returns `MontyException` if the function is not found, not callable,
    /// raises an exception, or encounters an external function call.
    pub fn call_function(
        &mut self,
        name: &str,
        args: Vec<MontyObject>,
        print: PrintWriter<'_>,
    ) -> Result<MontyObject, MontyException> {
        let slot_idx = self
            .interns
            .get_string_id_by_name(name)
            .and_then(|name_id| self.global_names.get(name_id));
        let Some(slot_idx) = slot_idx else {
            return Err(RunError::from(ExcType::name_error(name))
                .into_python_exception(&self.interns, |fname| self.sources.get(fname).map(String::as_str)));
        };

        let assert_repr_max_bytes = self.options.assert_message_annotations.max_bytes();
        HeapReader::with(
            &mut self.heap,
            &mut (&self.interns, print),
            |reader, (interns, print)| {
                let vm = &mut VM::new(
                    mem::take(&mut self.globals),
                    reader,
                    interns,
                    print.reborrow(),
                    assert_repr_max_bytes,
                );

                let callable = vm.globals[slot_idx.index()].clone_with_heap(vm);
                defer_drop!(callable, vm);

                let arg_values = match convert_args(args, vm) {
                    Ok(av) => av,
                    Err(e) => {
                        self.globals = vm.take_globals();
                        return Err(e);
                    }
                };

                // Host boundary: open an execution window so the time budget
                // advances (and accumulates) during the call. This cannot go
                // through `VM::run_external` because `evaluate_function` must
                // push and run a single function frame itself.
                vm.heap.tracker.on_execution_start();
                let eval_result = vm.evaluate_function("MontyRepl::call_function", callable, arg_values);
                vm.heap.tracker.on_execution_stop();
                // Same host-boundary epilogue as `run_external`: a limit
                // overshoot the call swallowed must fail the call, not
                // return a truncated value.
                let eval_result = vm.finish_host_turn(eval_result);

                let result = match eval_result {
                    Ok(value) => Ok(MontyObject::new(value, vm)),
                    Err(e) => {
                        Err(e.into_python_exception(&self.interns, |fname| self.sources.get(fname).map(String::as_str)))
                    }
                };

                self.globals = vm.take_globals();

                result
            },
        )
    }

    /// Evaluates one expression against the session, with `bindings` visible
    /// to it and to nothing after it, and hands back its value.
    ///
    /// Differs from [`probe_start`](Self::probe_start) in two ways, and both
    /// are what make it usable from inside a suspension (see
    /// [`ReplFunctionCall::probe`]): it runs to completion rather than
    /// suspending, so an expression reaching for a name only the host could
    /// answer raises `NameError`; and a binding is scoped to this one
    /// expression rather than left in the namespace, so the session cannot
    /// later resolve a name it never defined to something the host once
    /// supplied.
    ///
    /// # Errors
    /// Returns `MontyException` for a rejected expression, one that raises, or
    /// one that reaches a name the bindings do not supply.
    pub fn probe_scoped(
        &mut self,
        expr: &str,
        bindings: Vec<(String, MontyObject)>,
        print: PrintWriter<'_>,
    ) -> Result<MontyObject, MontyException> {
        probe_scoped_in(
            SessionParts {
                global_names: &mut self.global_names,
                interns: &mut self.interns,
                heap: &mut self.heap,
                globals: &mut self.globals,
            },
            expr,
            bindings,
            print,
            &self.script_name,
            self.options,
            &self.sources,
        )
    }

    /// Pins the value bound to `name` and hands back the reference that names
    /// it, so the host can hold it across turns and give it back later.
    ///
    /// This is how a value with no boundary representation reaches the host at
    /// all. A type object, a class, a template, a generator crosses today as
    /// its `repr`, which can be printed but not asked anything; a reference can
    /// be handed back into a later feed or probe as an ordinary input, and the
    /// session's own semantics answer whatever the host wants to know
    /// (`__origin__`, `__args__`, `.interpolations`, ...). Values that DO have
    /// a boundary representation are worth exporting too, when the host wants
    /// identity rather than a copy.
    ///
    /// The reference survives this session's dump and load, because it names a
    /// place in the heap that the dump carries.
    ///
    /// Every export must be released by [`release`](Self::release), including
    /// repeated exports of one value: the pin is what keeps the value alive,
    /// and nothing inside the sandbox can drop it.
    ///
    /// Returns `None` when the session binds no such name.
    pub fn export_global(&mut self, name: &str, print: PrintWriter<'_>) -> Option<MontyObject> {
        let slot = self
            .interns
            .get_string_id_by_name(name)
            .and_then(|name_id| self.global_names.get(name_id))?
            .index();
        if !matches!(self.globals.get(slot), Some(value) if !matches!(value, Value::Undefined)) {
            return None;
        }
        let assert_repr_max_bytes = self.options.assert_message_annotations.max_bytes();
        HeapReader::with(
            &mut self.heap,
            &mut (&self.interns, print),
            |reader, (interns, print)| {
                let vm = &mut VM::new(
                    mem::take(&mut self.globals),
                    reader,
                    interns,
                    print.reborrow(),
                    assert_repr_max_bytes,
                );
                let value = vm.globals[slot].clone_with_heap(vm);
                defer_drop!(value, vm);
                let exported = match *value {
                    Value::Ref(id) => {
                        // A `repr` runs sandbox code, so it is taken before the
                        // pin rather than after: a `__repr__` that raises must
                        // leave the export table as it found it, and a failed
                        // one still leaves a usable reference.
                        let repr = match value.py_repr(vm) {
                            Ok(rendered) => {
                                defer_drop!(rendered, vm);
                                rendered.to_str(vm).map(str::to_owned).unwrap_or_default()
                            }
                            Err(_) => format!("<{} object, error on repr()>", value.py_type_name(vm)),
                        };
                        MontyObject::SessionRef {
                            id: vm.heap.export(id),
                            repr,
                        }
                    }
                    // An immediate value is its own identity: there is nothing
                    // in the heap to pin, and it crosses losslessly by value.
                    ref value => MontyObject::from_value(value, vm),
                };
                self.globals = vm.take_globals();
                Some(exported)
            },
        )
    }

    /// Whether a value with no copy representation crosses out of this session
    /// as a reference rather than as its `repr` string.
    #[must_use]
    pub fn cross_by_reference(&self) -> bool {
        self.heap.cross_by_reference()
    }

    /// Makes a value with no copy representation cross out as a reference
    /// rather than as its `repr` string.
    ///
    /// A class object, a `list[int]`, a `t"..."` template, a generator all
    /// reach the host today as text, which can be printed and nothing else.
    /// In this mode each crosses as a [`MontyObject::SessionRef`] the host can
    /// hand back into a later feed or probe, so the session's own semantics
    /// answer whatever the host wants to know about it. This is what lets a
    /// value the host cannot represent be *passed through* the host: a
    /// contract handed to a host call, held, and given back later.
    ///
    /// Off by default, and the reason is ownership: every reference pins its
    /// value until [`release`](Self::release) drops it, and a host that
    /// ignores the references it is handed would grow the session's heap
    /// without bound. Turn it on where the host is written to release.
    ///
    /// Travels with a dump, so a session woken from one keeps the mode.
    pub fn set_cross_by_reference(&mut self, on: bool) {
        self.heap.set_cross_by_reference(on);
    }

    /// Releases one of the host's references to an exported value, letting the
    /// session free it once nothing else holds it.
    ///
    /// Returns whether the token named a live export. A token released twice,
    /// or one this session never minted, answers `false` rather than taking a
    /// reference that belongs to someone else.
    pub fn release(&mut self, token: u64) -> bool {
        self.heap.release_export(token)
    }

    /// Returns a list of all callable function names defined in the session.
    ///
    /// Includes functions, closures, and functions with default arguments.
    /// Does not include builtins or external functions.
    #[must_use]
    pub fn function_names(&self) -> Vec<&str> {
        self.global_names
            .iter()
            .filter_map(|(ns_id, name_id)| {
                let idx = ns_id.index();
                if idx < self.globals.len() && is_callable(&self.globals[idx], &self.heap) {
                    Some(self.interns.get_str(name_id))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Returns whether a function with the given name exists in the session.
    #[must_use]
    pub fn has_function(&self, name: &str) -> bool {
        let Some(name_id) = self.interns.get_string_id_by_name(name) else {
            return false;
        };
        self.global_names.get(name_id).is_some_and(|ns_id| {
            let idx = ns_id.index();
            idx < self.globals.len() && is_callable(&self.globals[idx], &self.heap)
        })
    }

    /// Grows the globals vector to at least `size` slots.
    ///
    /// Newly introduced slots are initialized to `Undefined` to keep slot alignment
    /// with the compiler's global-name map.
    fn ensure_globals_size(&mut self, size: usize) {
        if self.globals.len() < size {
            self.globals.resize_with(size, || Value::Undefined);
        }
    }

    /// Returns the generated filename for the next interactive snippet.
    ///
    /// CPython labels interactive snippets as `<python-input-N>` and increments
    /// N for each feed attempt. Matching this improves traceback ergonomics and
    /// makes REPL errors easier to correlate with user input history. A probe
    /// takes a name of its own from the same counter, so a traceback says which
    /// of the two produced the frame and no two snippets ever share a name.
    /// The filename this snippet's frames carry, the caller's when it supplied
    /// one and a generated `<python-input-N>` otherwise.
    ///
    /// A caller-supplied name is used verbatim, including when an earlier
    /// snippet already used it: the source text kept for rendering tracebacks
    /// is keyed by this name, so reusing one makes the older snippet's frames
    /// render against the newer text. Names are the caller's to keep distinct.
    fn snippet_script_name(&mut self, script_name: Option<&str>, kind: SnippetKind) -> String {
        match script_name {
            Some(name) if !name.is_empty() => name.to_owned(),
            _ => self.next_input_script_name(kind),
        }
    }

    fn next_input_script_name(&mut self, kind: SnippetKind) -> String {
        let input_id = self.next_input_id;
        self.next_input_id += 1;
        match kind {
            SnippetKind::Feed => format!("<python-input-{input_id}>"),
            SnippetKind::Probe => format!("<probe-{input_id}>"),
        }
    }
}

impl Drop for MontyRepl {
    fn drop(&mut self) {
        self.globals.drain(..).drop_with(&mut self.heap);
    }
}

// ---------------------------------------------------------------------------
// ReplProgress and per-variant structs
// ---------------------------------------------------------------------------

/// Which of the two things a snippet is, for the sake of the filename its
/// frames carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SnippetKind {
    /// A chunk of source fed to the session.
    Feed,
    /// One expression evaluated against the session's namespace.
    Probe,
}

/// Result of a single suspendable REPL snippet execution.
///
/// This mirrors `RunProgress` but returns the updated `MontyRepl` on completion
/// so callers can continue feeding additional snippets without replaying prior code.
/// Each variant (except `Complete`) wraps a dedicated struct with only the relevant
/// resume methods.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub enum ReplProgress {
    /// Execution paused at an external function call or dataclass method call.
    FunctionCall(ReplFunctionCall),
    /// Execution paused for an OS-level operation.
    OsCall(ReplOsCall),
    /// All async tasks are blocked waiting for external futures to resolve.
    ResolveFutures(ReplResolveFutures),
    /// Execution paused for an unresolved name lookup.
    NameLookup(ReplNameLookup),
    /// Snippet execution completed with the updated REPL and its outcome.
    Complete {
        /// Updated REPL session state to continue feeding snippets.
        repl: MontyRepl,
        /// What the snippet produced, and whether a `return` is what ended it.
        outcome: FeedOutcome,
    },
}

/// Error returned when a REPL snippet raises a Python exception during `start()` or `resume()`.
///
/// Unlike syntax/compile errors which consume the REPL, runtime errors preserve
/// the full session state so the caller can inspect the error and continue feeding
/// subsequent snippets. Any global mutations that occurred before the exception
/// remain visible in the returned `repl`.
#[derive(Debug)]
pub struct ReplStartError {
    /// REPL session state after the failed snippet — ready for further use.
    pub repl: MontyRepl,
    /// The Python exception that was raised.
    pub error: MontyException,
}

impl ReplProgress {
    /// Consumes the progress and returns the `ReplFunctionCall` struct.
    #[must_use]
    pub fn into_function_call(self) -> Option<ReplFunctionCall> {
        match self {
            Self::FunctionCall(call) => Some(call),
            _ => None,
        }
    }

    /// Consumes the progress and returns the `ReplResolveFutures` struct.
    #[must_use]
    pub fn into_resolve_futures(self) -> Option<ReplResolveFutures> {
        match self {
            Self::ResolveFutures(state) => Some(state),
            _ => None,
        }
    }

    /// Consumes the progress and returns the `ReplNameLookup` struct.
    #[must_use]
    pub fn into_name_lookup(self) -> Option<ReplNameLookup> {
        match self {
            Self::NameLookup(lookup) => Some(lookup),
            _ => None,
        }
    }

    /// Consumes the progress and returns the completed REPL and its outcome.
    #[must_use]
    pub fn into_complete(self) -> Option<(MontyRepl, FeedOutcome)> {
        match self {
            Self::Complete { repl, outcome } => Some((repl, outcome)),
            _ => None,
        }
    }

    /// Extracts the REPL session from any progress variant, discarding
    /// the in-flight execution state.
    ///
    /// Use this to recover the REPL when you need to abandon the current
    /// snippet (e.g. because `feed_run` doesn't support async futures).
    /// The REPL state reflects any mutations that occurred before the
    /// snapshot was taken.
    #[must_use]
    pub fn into_repl(self) -> MontyRepl {
        match self {
            Self::FunctionCall(call) => call.into_repl(),
            Self::OsCall(call) => call.into_repl(),
            Self::ResolveFutures(state) => state.into_repl(),
            Self::NameLookup(lookup) => lookup.into_repl(),
            Self::Complete { repl, .. } => repl,
        }
    }

    /// Evaluates one expression against the session, whatever the progress
    /// state, and leaves that state as it was.
    ///
    /// Suspended, this is the only way to read the frame that is asking, and
    /// the moment a host most needs to: it is deciding what to answer. See
    /// [`ReplFunctionCall::probe`]. Complete, it is
    /// [`MontyRepl::probe_scoped`], so a host driving progress in a loop needs
    /// no special case for the arm it landed on.
    ///
    /// # Errors
    /// Returns `MontyException` for a rejected expression, one that raises, or
    /// one reaching a name `bindings` does not supply. The progress is
    /// untouched either way.
    pub fn probe(
        &mut self,
        expr: &str,
        bindings: Vec<(String, MontyObject)>,
        print: PrintWriter<'_>,
    ) -> Result<MontyObject, MontyException> {
        match self {
            Self::FunctionCall(call) => call.probe(expr, bindings, print),
            Self::OsCall(call) => call.probe(expr, bindings, print),
            Self::ResolveFutures(state) => state.probe(expr, bindings, print),
            Self::NameLookup(lookup) => lookup.probe(expr, bindings, print),
            Self::Complete { repl, .. } => repl.probe_scoped(expr, bindings, print),
        }
    }

    /// Releases one of the host's references to an exported value, whatever
    /// the progress state; see [`MontyRepl::release`].
    pub fn release(&mut self, token: u64) -> bool {
        match self {
            Self::FunctionCall(call) => call.release(token),
            Self::OsCall(call) => call.release(token),
            Self::ResolveFutures(state) => state.release(token),
            Self::NameLookup(lookup) => lookup.release(token),
            Self::Complete { repl, .. } => repl.release(token),
        }
    }

    /// Returns the session's resource tracker, whatever the progress state.
    ///
    /// Lets hosts read resource accounting — e.g. cumulative execution time
    /// for `max_duration` budgeting — at any suspension point without
    /// consuming the progress.
    pub fn tracker(&self) -> &ResourceTracker {
        match self {
            Self::FunctionCall(call) => call.snapshot.repl.tracker(),
            Self::OsCall(call) => call.snapshot.repl.tracker(),
            Self::ResolveFutures(state) => state.repl.tracker(),
            Self::NameLookup(lookup) => lookup.snapshot.repl.tracker(),
            Self::Complete { repl, .. } => repl.tracker(),
        }
    }
}

// ---------------------------------------------------------------------------
// ReplFunctionCall
// ---------------------------------------------------------------------------

/// REPL execution paused at an external function call or dataclass method call.
///
/// Resume with `resume(result, print)` to provide the return value and continue,
/// or `resume_pending(print)` to push a `Future` for async resolution.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ReplFunctionCall {
    /// The name of the function or method being called.
    pub function_name: String,
    /// The positional arguments passed to the function.
    pub args: Vec<MontyObject>,
    /// The keyword arguments passed to the function (key, value pairs).
    pub kwargs: Vec<(MontyObject, MontyObject)>,
    /// Unique identifier for this call (used for async correlation).
    pub call_id: u32,
    /// Whether this is a dataclass method call (first arg is `self`).
    pub method_call: bool,
    /// Internal REPL execution snapshot.
    snapshot: ReplSnapshot,
}

impl ReplFunctionCall {
    /// Releases one of the host's references to an exported value, as
    /// [`MontyRepl::release`] does.
    ///
    /// Available here because the export table is heap state and the heap is
    /// the session's throughout: an export made before this suspension, or by
    /// the very call it is suspended in, is releasable without waiting for the
    /// rung to finish.
    pub fn release(&mut self, token: u64) -> bool {
        self.snapshot.repl.release(token)
    }

    /// Evaluates one expression against the suspended session, with
    /// `bindings` visible to it and to nothing after it, and leaves this
    /// suspension resumable.
    ///
    /// The host is answering a call the sandbox made, and some answers can
    /// only be decided by looking at the frame that is asking. Nothing is
    /// running while it asks, so the frame is readable; the expression sees
    /// the globals exactly as the suspended rung left them.
    ///
    /// Runs to completion, so an expression reaching a name the bindings do
    /// not supply raises `NameError` rather than suspending again.
    ///
    /// # Errors
    /// Returns `MontyException` for a rejected expression or one that raises.
    /// Either way the suspension is untouched and still resumable.
    pub fn probe(
        &mut self,
        expr: &str,
        bindings: Vec<(String, MontyObject)>,
        print: PrintWriter<'_>,
    ) -> Result<MontyObject, MontyException> {
        self.snapshot.probe_scoped(expr, bindings, print)
    }

    /// Extracts the REPL session, discarding the in-flight execution state.
    ///
    /// Restores globals from the VM snapshot so the REPL remains usable.
    #[must_use]
    pub fn into_repl(self) -> MontyRepl {
        self.snapshot.into_repl()
    }

    /// Resumes snippet execution with an external result.
    pub fn resume(
        self,
        result: impl Into<ExtFunctionResult>,
        print: PrintWriter<'_>,
    ) -> Result<ReplProgress, Box<ReplStartError>> {
        self.snapshot.run(result, print)
    }

    /// Resumes execution by pushing a `Future` for async resolution.
    ///
    /// Uses `self.call_id` internally — no need to pass it again.
    pub fn resume_pending(self, print: PrintWriter<'_>) -> Result<ReplProgress, Box<ReplStartError>> {
        self.snapshot.run(ExtFunctionResult::Future(self.call_id), print)
    }
}

// ---------------------------------------------------------------------------
// ReplOsCall
// ---------------------------------------------------------------------------

/// REPL execution paused for an OS-level operation.
///
/// Resume with `resume(result, print)` to provide the OS call result and continue.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ReplOsCall {
    /// Typed OS-call dispatch value (variant + args).
    pub function_call: OsFunctionCall,
    /// Unique identifier for this call (used for async correlation).
    pub call_id: u32,
    /// Internal REPL execution snapshot.
    snapshot: ReplSnapshot,
}

impl ReplOsCall {
    /// Releases one of the host's references to an exported value, as
    /// [`MontyRepl::release`] does.
    ///
    /// Available here because the export table is heap state and the heap is
    /// the session's throughout: an export made before this suspension, or by
    /// the very call it is suspended in, is releasable without waiting for the
    /// rung to finish.
    pub fn release(&mut self, token: u64) -> bool {
        self.snapshot.repl.release(token)
    }

    /// Evaluates one expression against the suspended session, with
    /// `bindings` visible to it and to nothing after it, and leaves this
    /// suspension resumable.
    ///
    /// The host is answering a call the sandbox made, and some answers can
    /// only be decided by looking at the frame that is asking. Nothing is
    /// running while it asks, so the frame is readable; the expression sees
    /// the globals exactly as the suspended rung left them.
    ///
    /// Runs to completion, so an expression reaching a name the bindings do
    /// not supply raises `NameError` rather than suspending again.
    ///
    /// # Errors
    /// Returns `MontyException` for a rejected expression or one that raises.
    /// Either way the suspension is untouched and still resumable.
    pub fn probe(
        &mut self,
        expr: &str,
        bindings: Vec<(String, MontyObject)>,
        print: PrintWriter<'_>,
    ) -> Result<MontyObject, MontyException> {
        self.snapshot.probe_scoped(expr, bindings, print)
    }

    /// Extracts the REPL session, discarding the in-flight execution state.
    ///
    /// Restores globals from the VM snapshot so the REPL remains usable.
    #[must_use]
    pub fn into_repl(self) -> MontyRepl {
        self.snapshot.into_repl()
    }

    /// Resumes snippet execution with the OS call result.
    pub fn resume(
        self,
        result: impl Into<ExtFunctionResult>,
        print: PrintWriter<'_>,
    ) -> Result<ReplProgress, Box<ReplStartError>> {
        self.snapshot.run(result.into(), print)
    }

    /// REPL mirror of [`crate::OsCall::resume_with`] — dispatches the call
    /// to `handler` (which receives the [`OsFunctionCall`] by value, so
    /// write payloads move without cloning) and resumes with its result.
    pub fn resume_with(
        self,
        print: PrintWriter<'_>,
        handler: impl FnOnce(OsFunctionCall) -> ExtFunctionResult,
    ) -> Result<ReplProgress, Box<ReplStartError>> {
        let result = handler(self.function_call);
        self.snapshot.run(result, print)
    }
}

// ---------------------------------------------------------------------------
// ReplNameLookup
// ---------------------------------------------------------------------------

/// REPL execution paused for an unresolved name lookup.
///
/// The host should check if the name corresponds to a known external function or
/// value. Call `resume(result, print)` with the appropriate `NameLookupResult`.
/// The namespace slot and scope are managed internally.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ReplNameLookup {
    /// The name being looked up.
    pub name: String,
    /// The namespace slot where the resolved value should be cached.
    namespace_slot: u16,
    /// Whether this is a global slot or a local/function slot.
    is_global: bool,
    /// Internal REPL execution snapshot.
    snapshot: ReplSnapshot,
}

impl ReplNameLookup {
    /// Releases one of the host's references to an exported value, as
    /// [`MontyRepl::release`] does.
    ///
    /// Available here because the export table is heap state and the heap is
    /// the session's throughout: an export made before this suspension, or by
    /// the very call it is suspended in, is releasable without waiting for the
    /// rung to finish.
    pub fn release(&mut self, token: u64) -> bool {
        self.snapshot.repl.release(token)
    }

    /// Evaluates one expression against the suspended session, with
    /// `bindings` visible to it and to nothing after it, and leaves this
    /// suspension resumable.
    ///
    /// The host is answering a call the sandbox made, and some answers can
    /// only be decided by looking at the frame that is asking. Nothing is
    /// running while it asks, so the frame is readable; the expression sees
    /// the globals exactly as the suspended rung left them.
    ///
    /// Runs to completion, so an expression reaching a name the bindings do
    /// not supply raises `NameError` rather than suspending again.
    ///
    /// # Errors
    /// Returns `MontyException` for a rejected expression or one that raises.
    /// Either way the suspension is untouched and still resumable.
    pub fn probe(
        &mut self,
        expr: &str,
        bindings: Vec<(String, MontyObject)>,
        print: PrintWriter<'_>,
    ) -> Result<MontyObject, MontyException> {
        self.snapshot.probe_scoped(expr, bindings, print)
    }

    /// Extracts the REPL session, discarding the in-flight execution state.
    ///
    /// Restores globals from the VM snapshot so the REPL remains usable.
    #[must_use]
    pub fn into_repl(self) -> MontyRepl {
        self.snapshot.into_repl()
    }

    /// Resumes execution after name resolution.
    ///
    /// Caches the resolved value in the namespace slot before restoring the VM,
    /// then either pushes the value onto the stack or raises `NameError`.
    pub fn resume(self, result: NameLookupResult, print: PrintWriter<'_>) -> Result<ReplProgress, Box<ReplStartError>> {
        let Self {
            name,
            namespace_slot,
            is_global,
            snapshot,
        } = self;

        let ReplSnapshot {
            mut repl,
            executor,
            vm_state,
        } = snapshot;

        match HeapReader::with(&mut repl.heap, &mut (&executor, print), |reader, (executor, print)| {
            // Restore the VM first, then convert inside its lifetime
            let mut vm = VM::restore(
                vm_state,
                &executor.module_code,
                reader,
                &executor.interns,
                print.reborrow(),
                executor.assert_repr_max_bytes,
            );

            // Resolve the name lookup result with the VM alive
            let vm_result = match result {
                NameLookupResult::Value(obj) => {
                    let value = match obj.to_value(&mut vm) {
                        Ok(v) => v,
                        Err(e) => {
                            repl.globals = vm.take_globals();
                            return Err(MontyException::runtime_error(format!(
                                "invalid name lookup result: {e}"
                            )));
                        }
                    };

                    // Cache the resolved value in the appropriate slot
                    let slot_idx = namespace_slot as usize;
                    let cloned = value.clone_with_heap(&vm);
                    let slot = if is_global {
                        &mut vm.globals[slot_idx]
                    } else {
                        let stack_base = vm.current_stack_base();
                        &mut vm.stack[stack_base + slot_idx]
                    };
                    let old = mem::replace(slot, cloned);
                    old.drop_with(&mut vm);

                    vm.push(value);
                    vm.run_external()
                }
                NameLookupResult::Undefined => {
                    let err: RunError = ExcType::name_error(&name).into();
                    vm.resume_with_exception(err)
                }
            };

            // Convert while VM alive, then snapshot or reclaim globals
            let converted = convert_frame_exit(vm_result, &mut vm);
            let vm_state = if converted.needs_snapshot() {
                Some(vm.snapshot())
            } else {
                repl.globals = vm.take_globals();
                None
            };
            Ok((converted, vm_state))
        }) {
            Ok((converted, vm_state)) => build_repl_progress(converted, vm_state, executor, repl),
            Err(error) => Err(Box::new(ReplStartError { repl, error })),
        }
    }
}

// ---------------------------------------------------------------------------
// ReplResolveFutures
// ---------------------------------------------------------------------------

/// REPL execution state blocked on unresolved external futures.
///
/// This is the REPL-aware counterpart to `ResolveFutures`.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ReplResolveFutures {
    /// Persistent REPL session state while this snippet is suspended.
    repl: MontyRepl,
    /// Compiled snippet and intern/function tables for this execution.
    executor: Executor,
    /// VM stack/frame state at suspension.
    vm_state: VMSnapshot,
    /// Pending call IDs expected by this snapshot.
    pending_call_ids: Vec<u32>,
}

impl ReplResolveFutures {
    /// Releases one of the host's references to an exported value, as
    /// [`MontyRepl::release`] does; see [`ReplFunctionCall::release`].
    pub fn release(&mut self, token: u64) -> bool {
        self.repl.release(token)
    }

    /// Evaluates one expression against the suspended session, with
    /// `bindings` visible to it and to nothing after it, and leaves this
    /// suspension resumable.
    ///
    /// The host is answering a call the sandbox made, and some answers can
    /// only be decided by looking at the frame that is asking. Nothing is
    /// running while it asks, so the frame is readable; the expression sees
    /// the globals exactly as the suspended rung left them.
    ///
    /// Runs to completion, so an expression reaching a name the bindings do
    /// not supply raises `NameError` rather than suspending again.
    ///
    /// # Errors
    /// Returns `MontyException` for a rejected expression or one that raises.
    /// Either way the suspension is untouched and still resumable.
    pub fn probe(
        &mut self,
        expr: &str,
        bindings: Vec<(String, MontyObject)>,
        print: PrintWriter<'_>,
    ) -> Result<MontyObject, MontyException> {
        probe_scoped_in(
            SessionParts {
                global_names: &mut self.executor.globals,
                interns: &mut self.executor.interns,
                heap: &mut self.repl.heap,
                globals: &mut self.vm_state.globals,
            },
            expr,
            bindings,
            print,
            &self.repl.script_name,
            self.repl.options,
            &self.repl.sources,
        )
    }

    /// Extracts the REPL session, restoring globals from the suspended VM state.
    ///
    /// As with the other REPL snapshot types, globals live inside the VM
    /// snapshot while execution is suspended. Recovering the REPL for a
    /// cancelled or abandoned async snippet must put those globals back so
    /// previously defined REPL bindings remain available.
    #[must_use]
    pub fn into_repl(self) -> MontyRepl {
        let Self { mut repl, vm_state, .. } = self;
        repl.globals = vm_state.globals;
        repl
    }

    /// Returns unresolved call IDs for this suspended state.
    #[must_use]
    pub fn pending_call_ids(&self) -> &[u32] {
        &self.pending_call_ids
    }

    /// Resumes snippet execution with zero or more resolved futures.
    ///
    /// Supports incremental resolution: callers can provide only a subset of
    /// pending call IDs and continue resolving over multiple resumes.
    ///
    /// All errors — including API misuse (unknown `call_id`) and Python-level
    /// runtime failures — are returned as `Err(Box<ReplStartError>)` so the REPL
    /// session is always preserved.
    pub fn resume(
        self,
        results: Vec<(u32, ExtFunctionResult)>,
        print: PrintWriter<'_>,
    ) -> Result<ReplProgress, Box<ReplStartError>> {
        let Self {
            mut repl,
            executor,
            vm_state,
            pending_call_ids,
        } = self;

        let invalid_call_id = results
            .iter()
            .find(|(call_id, _)| !pending_call_ids.contains(call_id))
            .map(|(call_id, _)| *call_id);

        match HeapReader::with(&mut repl.heap, &mut (&executor, print), |reader, (executor, print)| {
            let mut vm = VM::restore(
                vm_state,
                &executor.module_code,
                reader,
                &executor.interns,
                print.reborrow(),
                executor.assert_repr_max_bytes,
            );

            if let Some(call_id) = invalid_call_id {
                repl.globals = vm.take_globals();
                return Err(MontyException::runtime_error(format!(
                    "unknown call_id {call_id}, expected one of: {pending_call_ids:?}"
                )));
            }

            let vm_result = vm.resume_with_resolved_futures(results);

            // Convert while VM alive, then snapshot or reclaim globals
            let converted = convert_frame_exit(vm_result, &mut vm);
            let vm_state = if converted.needs_snapshot() {
                Some(vm.snapshot())
            } else {
                repl.globals = vm.take_globals();
                None
            };
            Ok((converted, vm_state))
        }) {
            Ok((converted, vm_state)) => build_repl_progress(converted, vm_state, executor, repl),
            Err(error) => Err(Box::new(ReplStartError { repl, error })),
        }
    }
}

// ---------------------------------------------------------------------------
// ReplContinuationMode — public utility for interactive input collection
// ---------------------------------------------------------------------------

/// Parse-derived continuation state for interactive REPL input collection.
///
/// `monty-runtime` uses this to decide whether to execute the buffered snippet
/// immediately, keep collecting continuation lines, or require a terminating
/// blank line for block statements (`if:`, `def:`, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplContinuationMode {
    /// The current snippet is syntactically complete and can run now.
    Complete,
    /// The snippet is incomplete and needs more continuation lines.
    IncompleteImplicit,
    /// The snippet opened an indented block and should wait for a trailing blank
    /// line before execution, matching CPython interactive behavior.
    IncompleteBlock,
}

/// Detects whether REPL source is complete or needs more input.
///
/// This mirrors CPython's broad interactive behavior:
/// - Incomplete bracketed / parenthesized / triple-quoted constructs continue.
/// - Clause headers (`if:`, `def:`, etc.) require an indented body and then a
///   terminating blank line before execution.
/// - All other parse outcomes are treated as complete (either valid code or a
///   syntax error that should be shown immediately).
#[must_use]
pub fn detect_repl_continuation_mode(source: &str) -> ReplContinuationMode {
    match parse_module(source) {
        Ok(_) => ReplContinuationMode::Complete,
        Err(error) => continuation_mode_of(source, &error),
    }
}

/// Classifies a parse failure as incomplete input or a real error.
///
/// Split from [`detect_repl_continuation_mode`] so a caller that already holds
/// the failure (having parsed for other reasons) draws the same line without
/// parsing the source a second time. `source` is the text the failure came
/// from: one case is decided by what stands at the error, not by its type.
pub(crate) fn continuation_mode_of(source: &str, error: &ParseError) -> ReplContinuationMode {
    match &error.error {
        ParseErrorType::OtherError(msg) => {
            if msg.starts_with("Expected an indented block after ") {
                ReplContinuationMode::IncompleteBlock
            } else {
                ReplContinuationMode::Complete
            }
        }
        // A string the lexer never saw closed is unfinished input when it was
        // opened with a triple quote, and a plain error otherwise: a `'` with
        // no partner ends at the newline and no further line can close it.
        // The two arrive as one error type, so the opening quote decides.
        ParseErrorType::Lexical(LexicalErrorType::UnclosedStringError)
            if opens_triple_quoted(source, error.location.start().to_usize()) =>
        {
            ReplContinuationMode::IncompleteImplicit
        }
        ParseErrorType::Lexical(LexicalErrorType::Eof)
        | ParseErrorType::ExpectedToken {
            found: TokenKind::EndOfFile,
            ..
        }
        | ParseErrorType::FStringError(InterpolatedStringErrorType::UnterminatedTripleQuotedString)
        | ParseErrorType::TStringError(InterpolatedStringErrorType::UnterminatedTripleQuotedString) => {
            ReplContinuationMode::IncompleteImplicit
        }
        _ => ReplContinuationMode::Complete,
    }
}

/// Whether the string literal starting at `offset` opens with a triple quote.
///
/// The offset is the whole token's start, so any prefix letters (`r`, `rb`,
/// `f`) come first; what follows them is the quote run.
fn opens_triple_quoted(source: &str, offset: usize) -> bool {
    let rest = source[offset.min(source.len())..].trim_start_matches(char::is_alphabetic);
    rest.starts_with("\"\"\"") || rest.starts_with("'''")
}

// ---------------------------------------------------------------------------
// ReplSnapshot — internal execution state for suspend/resume
// ---------------------------------------------------------------------------

/// REPL execution state that can be resumed after an external call.
///
/// This is the REPL-aware counterpart to `Snapshot`. It is `pub(crate)` —
/// callers interact with the per-variant structs (`ReplFunctionCall`, etc.).
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct ReplSnapshot {
    /// Persistent REPL session state while this snippet is suspended.
    repl: MontyRepl,
    /// Compiled snippet and intern/function tables for this execution.
    executor: Executor,
    /// VM stack/frame state at suspension.
    vm_state: VMSnapshot,
}

impl ReplSnapshot {
    /// Extracts the REPL session, restoring globals from the VM snapshot.
    ///
    /// When a snapshot is taken, globals live inside the `VMSnapshot`.
    /// This method creates an empty snapshot from just the globals so the REPL
    /// can be used for further snippets.
    fn into_repl(self) -> MontyRepl {
        let Self { mut repl, vm_state, .. } = self;
        repl.globals = vm_state.globals;
        repl
    }

    /// Evaluates one expression against the suspended session, leaving the
    /// suspension resumable.
    ///
    /// Nothing is running: the worker is stopped at the exit it took, waiting
    /// for the host's answer. So the frame is readable, and the only reason it
    /// was not is that the session's three pieces are split across the
    /// suspension while it lasts. [`SessionParts`] names them; the expression
    /// sees the globals as the rung left them, including whatever it bound
    /// before it suspended.
    ///
    /// This is what lets a host decide its answer BY looking at the frame that
    /// is asking, which is the only moment some questions can be asked at all.
    fn probe_scoped(
        &mut self,
        expr: &str,
        bindings: Vec<(String, MontyObject)>,
        print: PrintWriter<'_>,
    ) -> Result<MontyObject, MontyException> {
        probe_scoped_in(
            SessionParts {
                global_names: &mut self.executor.globals,
                interns: &mut self.executor.interns,
                heap: &mut self.repl.heap,
                globals: &mut self.vm_state.globals,
            },
            expr,
            bindings,
            print,
            &self.repl.script_name,
            self.repl.options,
            &self.repl.sources,
        )
    }

    /// Continues snippet execution with an external result.
    fn run(
        self,
        result: impl Into<ExtFunctionResult>,
        print: PrintWriter<'_>,
    ) -> Result<ReplProgress, Box<ReplStartError>> {
        let Self {
            mut repl,
            executor,
            vm_state,
        } = self;

        let ext_result = result.into();

        let (converted, vm_state) =
            HeapReader::with(&mut repl.heap, &mut (&executor, print), |reader, (executor, print)| {
                let mut vm = VM::restore(
                    vm_state,
                    &executor.module_code,
                    reader,
                    &executor.interns,
                    print.reborrow(),
                    executor.assert_repr_max_bytes,
                );

                let vm_result = match ext_result {
                    ExtFunctionResult::Return(obj) => vm.resume(obj),
                    ExtFunctionResult::Error(exc) => vm.resume_with_exception(exc.into()),
                    ExtFunctionResult::Future(raw_call_id) => {
                        let call_id = CallId::new(raw_call_id);
                        vm.add_pending_call(call_id);
                        vm.run_external()
                    }
                    ExtFunctionResult::NotFound(function_name) => {
                        vm.resume_with_exception(ExtFunctionResult::not_found_exc(&function_name))
                    }
                };

                // Convert while VM alive, then snapshot or reclaim globals
                let converted = convert_frame_exit(vm_result, &mut vm);
                let vm_state = if converted.needs_snapshot() {
                    Some(vm.snapshot())
                } else {
                    repl.globals = vm.take_globals();
                    None
                };
                (converted, vm_state)
            });
        build_repl_progress(converted, vm_state, executor, repl)
    }
}

// ---------------------------------------------------------------------------
// Private helper functions
// ---------------------------------------------------------------------------

/// Injects input values into the VM's global namespace slots.
///
/// Converts each `MontyObject` to a `Value` while the VM is alive, then
/// stores it at the namespace slot that `Executor::new_repl_snippet`
/// pre-resolved for the corresponding input name. Each store is O(1) — the
/// per-input name → slot lookup happens once at snippet construction, not
/// here on the call path.
fn inject_inputs_into_vm(
    executor: &Executor,
    input_values: Vec<MontyObject>,
    vm: &mut VM<'_>,
) -> Result<(), MontyException> {
    for (&slot, obj) in executor.input_slots.iter().zip(input_values) {
        let value = obj
            .to_value(vm)
            .map_err(|e| MontyException::runtime_error(format!("invalid input type: {e}")))?;
        let old = mem::replace(&mut vm.globals[slot.index()], value);
        old.drop_with(vm);
    }
    Ok(())
}

/// The mutable session state one expression needs to run against, borrowed
/// from wherever it currently lives.
///
/// While a snippet is suspended the session is split: the heap stays on the
/// [`MontyRepl`], the live name map and interns are on the in-flight
/// [`Executor`], and the globals are inside the [`VMSnapshot`]. Idle, all four
/// are on the repl. Naming the four pieces is what lets one implementation
/// serve both.
struct SessionParts<'a> {
    global_names: &'a mut NameMap,
    interns: &'a mut Interns,
    heap: &'a mut Heap,
    globals: &'a mut Vec<Value>,
}

/// Evaluates one expression against `parts`, with `bindings` visible to it and
/// to nothing else, and leaves the session as it was.
///
/// The expression binds nothing: `check_probe_expression` refuses source that
/// could, and each binding's slot is put back the way it was found afterwards,
/// so a name supplied here does not become a name the session has. That is the
/// difference from an input to a feed, and from a name resolved through the
/// host: both of those stay in the namespace, where a later snippet finds them
/// and a missing definition is silently answered by them.
///
/// Runs to completion. An expression that reaches for something only the host
/// can answer raises `NameError` rather than suspending, because this is
/// itself reachable from inside a suspension and a second one would have
/// nowhere to go. Anything the expression needs is a binding.
fn probe_scoped_in(
    parts: SessionParts<'_>,
    expr: &str,
    bindings: Vec<(String, MontyObject)>,
    print: PrintWriter<'_>,
    script_name: &str,
    options: CompileOptions,
    sources: &AHashMap<String, String>,
) -> Result<MontyObject, MontyException> {
    let SessionParts {
        global_names,
        interns,
        heap,
        globals,
    } = parts;
    check_probe_expression(expr, script_name)?;
    let (binding_names, binding_values): (Vec<_>, Vec<_>) = bindings.into_iter().unzip();
    let executor = Executor::new_repl_snippet(
        expr.to_owned(),
        script_name,
        global_names.clone(),
        interns,
        &binding_names,
        options,
    )?;
    globals.resize_with(globals.len().max(executor.namespace_size()), || Value::Undefined);

    let result = HeapReader::with(heap, &mut (&executor, print), |reader, (executor, print)| {
        let vm = &mut VM::new(
            mem::take(globals),
            reader,
            &executor.interns,
            print.reborrow(),
            executor.assert_repr_max_bytes,
        );
        // Taken before injection and put back after, so the binding is visible
        // to this expression and to nothing that runs later.
        let displaced: Vec<Value> = executor
            .input_slots
            .iter()
            .map(|slot| vm.globals[slot.index()].clone_with_heap(vm))
            .collect();

        // `run_to_completion` opens and closes the execution window itself,
        // so the time budget advances here as it does for a feed.
        let result = inject_inputs_into_vm(executor, binding_values, vm).and_then(|()| {
            executor
                .run_to_completion(vm)
                .map_err(|e| e.into_python_exception(&executor.interns, |fname| sources.get(fname).map(String::as_str)))
        });

        for (slot, old) in executor.input_slots.iter().zip(displaced) {
            let injected = mem::replace(&mut vm.globals[slot.index()], old);
            injected.drop_with(vm);
        }
        *globals = vm.take_globals();
        result
    });

    // Committed whether or not the expression succeeded: the interner is
    // append-only, and a value the expression left in the heap can name a
    // string only this compilation interned. The name map goes with it so a
    // second probe reuses the slots this one claimed instead of adding more.
    let Executor {
        globals: probe_globals,
        interns: probe_interns,
        ..
    } = executor;
    *global_names = probe_globals;
    *interns = probe_interns;
    result
}

/// Assembles a `ReplProgress` from already-converted data.
///
/// This is the REPL equivalent of `build_run_progress`. On completion/error,
/// compiler metadata is committed to the REPL so subsequent snippets see
/// updated intern tables and name maps.
fn build_repl_progress(
    converted: ConvertedExit,
    vm_state: Option<VMSnapshot>,
    executor: Executor,
    mut repl: MontyRepl,
) -> Result<ReplProgress, Box<ReplStartError>> {
    macro_rules! new_repl_snapshot {
        () => {
            ReplSnapshot {
                repl,
                executor,
                vm_state: vm_state.expect("snapshot should exist"),
            }
        };
    }

    match converted {
        ConvertedExit::Complete(outcome) => {
            let Executor {
                globals: snippet_globals,
                interns,
                ..
            } = executor;
            repl.global_names = snippet_globals;
            repl.interns = interns;
            Ok(ReplProgress::Complete { repl, outcome })
        }
        ConvertedExit::FunctionCall {
            function_name,
            args,
            kwargs,
            call_id,
            method_call,
        } => Ok(ReplProgress::FunctionCall(ReplFunctionCall {
            function_name,
            args,
            kwargs,
            call_id,
            method_call,
            snapshot: new_repl_snapshot!(),
        })),
        ConvertedExit::OsCall { function_call, call_id } => Ok(ReplProgress::OsCall(ReplOsCall {
            function_call,
            call_id,
            snapshot: new_repl_snapshot!(),
        })),
        ConvertedExit::ResolveFutures(pending_call_ids) => Ok(ReplProgress::ResolveFutures(ReplResolveFutures {
            repl,
            executor,
            vm_state: vm_state.expect("snapshot should exist for ResolveFutures"),
            pending_call_ids,
        })),
        ConvertedExit::NameLookup {
            name,
            namespace_slot,
            is_global,
        } => Ok(ReplProgress::NameLookup(ReplNameLookup {
            name,
            namespace_slot,
            is_global,
            snapshot: new_repl_snapshot!(),
        })),
        ConvertedExit::Error(err) => {
            // Resolve traceback frames against every snippet the REPL has
            // seen, not just the currently-executing one. `executor.interns`
            // is still required because it holds the StringIds referenced by
            // the in-flight frames; `repl.sources` holds every snippet's
            // source text and is what owns any older snippets' sources.
            let error =
                err.into_python_exception(&executor.interns, |fname| repl.sources.get(fname).map(String::as_str));
            // Commit compiler metadata even on runtime errors, matching feed() behavior.
            // Snippets can create new variables or functions before raising, and those
            // values may reference FunctionId/StringId values from the new tables.
            let Executor {
                globals: snippet_globals,
                interns,
                ..
            } = executor;
            repl.global_names = snippet_globals;
            repl.interns = interns;
            Err(Box::new(ReplStartError { repl, error }))
        }
    }
}

/// Converts `Vec<MontyObject>` to internal `ArgValues` for function calls.
fn convert_args(args: Vec<MontyObject>, vm: &mut VM<'_>) -> Result<ArgValues, MontyException> {
    match args.len() {
        0 => Ok(ArgValues::Empty),
        1 => {
            let value = args
                .into_iter()
                .next()
                .expect("checked len")
                .to_value(vm)
                .map_err(|e| MontyException::runtime_error(format!("invalid argument type: {e}")))?;
            Ok(ArgValues::One(value))
        }
        2 => {
            let mut iter = args.into_iter();
            let a = iter
                .next()
                .expect("checked len")
                .to_value(vm)
                .map_err(|e| MontyException::runtime_error(format!("invalid argument type: {e}")))?;
            match iter.next().expect("checked len").to_value(vm) {
                Ok(b) => Ok(ArgValues::Two(a, b)),
                Err(e) => {
                    a.drop_with(&mut *vm);
                    Err(MontyException::runtime_error(format!("invalid argument type: {e}")))
                }
            }
        }
        _ => {
            let mut values = Vec::with_capacity(args.len());
            for arg in args {
                match arg.to_value(vm) {
                    Ok(value) => values.push(value),
                    Err(e) => {
                        values.drain(..).drop_with(&mut *vm);
                        return Err(MontyException::runtime_error(format!("invalid argument type: {e}")));
                    }
                }
            }
            Ok(ArgValues::ArgsKargs {
                args: values,
                kwargs: KwargsValues::Empty,
            })
        }
    }
}

/// Whether a session global should be surfaced as a "function" by
/// [`function_names`](MontyRepl::function_names) / [`has_function`](MontyRepl::has_function).
///
/// Deliberately narrower than [`Value::is_callable`]: it keeps only plain
/// function-like values and omits `Class`, `NamedTupleClass`, and `BoundMethod`,
/// which are callable but not what a host means by "a function it can invoke".
fn is_callable(value: &Value, heap: &Heap) -> bool {
    match value {
        Value::Builtin(_) | Value::ModuleFunction(_) | Value::DefFunction(_) => true,
        // A `Class`, `NamedTupleClass`, or `BoundMethod` is also callable but is
        // not a function, so keep only the function-like heap variants.
        Value::Ref(id) => matches!(
            heap.get(*id),
            HeapData::Closure(_) | HeapData::FunctionDefaults(_) | HeapData::ExtFunction(_)
        ),
        _ => false,
    }
}
