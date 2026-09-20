//! Implementation of the compile(), eval() and exec() builtin functions.
//!
//! `eval()` and `exec()` compile their source at runtime against the session's
//! intern tables and push a frame for it, so the snippet runs like a called
//! function: it can suspend to the host, and its result lands on the caller's
//! stack. `compile()` only validates its source and hands back a [`Code`] that
//! carries it, because a snippet is compiled against the namespace it runs in
//! and that namespace is not known until `eval()` or `exec()`.

use std::{str, sync::Arc};

use crate::{
    args::{ArgValues, FromArgs, Signature},
    bytecode::{CallResult, Compiler, FrameNamespace, VM},
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunError, RunResult, SimpleException},
    expressions::{Identifier, Node},
    function::Function,
    heap::{DropGuard, HeapData, HeapId},
    intern::{CompileInterns, FunctionId, StaticStrings},
    modules::ast::PY_CF_ALLOW_TOP_LEVEL_AWAIT,
    name_map::NameMap,
    parse::{CodeRange, parse_expression_with_interner, parse_module_with_filename_id},
    prepare::{SnippetNames, prepare_snippet},
    types::{Code, CodeMode, Type, py_trait::PyTrait},
    value::Value,
};

/// Arguments of `eval(source, /, globals=None, locals=None)`.
#[derive(FromArgs)]
#[from_args(name = "eval", at_most_total)]
struct EvalArgs {
    #[from_args(pos_only)]
    source: Value,
    #[from_args(default = Value::None)]
    globals: Value,
    #[from_args(default = Value::None)]
    locals: Value,
}

/// Arguments of `exec(source, /, globals=None, locals=None, *, closure=None)`.
#[derive(FromArgs)]
#[from_args(name = "exec")]
struct ExecArgs {
    #[from_args(pos_only)]
    source: Value,
    #[from_args(default = Value::None)]
    globals: Value,
    #[from_args(default = Value::None)]
    locals: Value,
    #[from_args(kw_only, default = Value::None)]
    closure: Value,
}

/// Implementation of the `eval()` builtin function.
///
/// Parses `source` as one expression and runs it in the given (or the
/// caller's) namespace; the expression's value is the call's result.
pub fn builtin_eval(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let EvalArgs {
        source,
        globals,
        locals,
    } = EvalArgs::from_args(args, vm)?;
    defer_drop!(source, vm);
    defer_drop!(globals, vm);
    defer_drop!(locals, vm);

    let snippet = snippet(Builtin::Eval, source, vm)?;
    let globals = globals_dict(globals, Builtin::Eval, vm)?;
    let locals = locals_dict(locals, Builtin::Eval, vm)?;
    run_snippet(Builtin::Eval, &snippet, globals, locals, vm)
}

/// Implementation of the `exec()` builtin function.
///
/// Compiles `source` as a module and runs it in the given (or the caller's)
/// namespace; the call's result is `None`.
pub fn builtin_exec(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let ExecArgs {
        source,
        globals,
        locals,
        closure,
    } = ExecArgs::from_args(args, vm)?;
    defer_drop!(source, vm);
    defer_drop!(globals, vm);
    defer_drop!(locals, vm);
    defer_drop!(closure, vm);

    if !matches!(closure, Value::None) {
        return Err(ExcType::type_error(
            "closure can only be used when source is a code object",
        ));
    }
    let snippet = snippet(Builtin::Exec, source, vm)?;
    let globals = globals_dict(globals, Builtin::Exec, vm)?;
    let locals = locals_dict(locals, Builtin::Exec, vm)?;
    run_snippet(Builtin::Exec, &snippet, globals, locals, vm)
}

/// Which builtin is running: the two default to different parse modes and word
/// their argument errors differently.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Builtin {
    Eval,
    Exec,
}

impl Builtin {
    /// The name this builtin puts in its own error messages.
    fn name(self) -> &'static str {
        match self {
            Self::Eval => "eval",
            Self::Exec => "exec",
        }
    }
}

/// What `eval()` / `exec()` were handed: the text to run and the mode to parse
/// it in.
///
/// A string argument takes the builtin's own mode, one expression for `eval()`
/// and a module body for `exec()`. A code object carries the mode `compile()`
/// gave it instead, so `eval()` of an `'exec'` code object runs a module body
/// and `exec()` of an `'eval'` one evaluates an expression, as in CPython.
struct Snippet {
    text: Arc<str>,
    mode: CodeMode,
    /// Whether the body may `await` at its top level, which makes running it
    /// hand back a coroutine rather than running it here; only a code object
    /// compiled with `ast.PyCF_ALLOW_TOP_LEVEL_AWAIT` sets it.
    top_level_await: bool,
}

/// Compiles `source` in the namespace `globals` / `locals` describe (borrowed
/// dict ids, validated by the caller) and pushes its frame.
fn run_snippet(
    builtin: Builtin,
    snippet: &Snippet,
    globals: Option<HeapId>,
    locals: Option<HeapId>,
    vm: &mut VM<'_>,
) -> RunResult<CallResult> {
    let source = &snippet.text;
    if source.contains('\0') {
        return Err(
            SimpleException::new_msg(ExcType::SyntaxError, "source code string cannot contain null bytes").into(),
        );
    }
    for dict in globals.into_iter().chain(locals) {
        vm.heap.inc_ref(dict);
    }
    let (names, namespace) = vm.snippet_namespace(globals, locals)?;
    let globals_len = vm.global_names.len();
    let result = compile_and_push(builtin, snippet, names, namespace, vm);
    if result.is_err() {
        vm.global_names.truncate(globals_len);
    }
    result
}

/// Compiles privately and publishes only after the snippet's frame is admitted.
fn compile_and_push(
    builtin: Builtin,
    snippet: &Snippet,
    names: SnippetNames,
    namespace: Box<FrameNamespace>,
    vm: &mut VM<'_>,
) -> RunResult<CallResult> {
    let source = &snippet.text;
    let mut namespace_guard = DropGuard::new(namespace, vm);
    let (_, vm) = namespace_guard.as_parts_mut();
    let mut overlay = CompileInterns::new(vm.interns);
    let filename_id = overlay.add_eval_source(Arc::clone(source));
    let nodes = match snippet.mode {
        CodeMode::Exec => parse_module_with_filename_id(source, filename_id, &mut overlay),
        // The expression is the body's value for `eval()` and is thrown away for
        // `exec()`, which answers `None` however its source was compiled.
        CodeMode::Eval => {
            let trimmed = source.trim_start();
            let skipped = u32::try_from(source.len() - trimmed.len()).unwrap_or(u32::MAX);
            parse_expression_with_interner(trimmed, filename_id, &mut overlay)
                .map(|expr| match builtin {
                    Builtin::Eval => vec![Node::Return(Some(expr))],
                    Builtin::Exec => vec![Node::Expr(expr)],
                })
                .map_err(|e| e.shifted(skipped))
        }
    }
    .map_err(|e| e.into_run_error(source))?;

    let options = vm.env.options;
    let globals_by_name = names == SnippetNames::NameOverDict;
    let mut scratch = NameMap::new();
    let globals = if globals_by_name {
        &mut scratch
    } else {
        &mut *vm.global_names
    };
    let nodes = prepare_snippet(nodes, &overlay, globals, names).map_err(|e| e.into_run_error(source))?;
    let code = Compiler::compile_snippet(
        &nodes,
        &mut overlay,
        globals,
        options,
        globals_by_name,
        snippet.top_level_await,
    )
    .map_err(|e| e.into_run_error(source))?;

    let position = CodeRange {
        filename: filename_id,
        start_byte: 0,
        end_byte: u32::try_from(source.len()).unwrap_or(u32::MAX),
    };
    let awaits = code.is_coroutine();
    let function = Function::new(
        Identifier::new(overlay.intern_static(StaticStrings::Module), position),
        Signature::default(),
        0,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        0,
        awaits,
        code,
    );
    let index = overlay.functions_len();
    let func_id = u16::try_from(index).map(FunctionId::from_index).map_err(|_| {
        SimpleException::new_msg(
            ExcType::SyntaxError,
            format!("session defines too many functions; maximum is {}", u16::MAX),
        )
    })?;

    overlay.push_function(function);
    let (namespace, vm) = namespace_guard.into_parts();
    // CPython sets `CO_COROUTINE` on a body the flag let await, and only on
    // one that does: a snippet that awaits nothing runs where it stands, as it
    // would without the flag.
    if awaits {
        let coroutine = vm.snippet_coroutine(func_id, overlay, namespace);
        vm.globals.resize_with(vm.global_names.len(), || Value::Undefined);
        return Ok(CallResult::Value(coroutine));
    }
    vm.push_snippet_frame(func_id, overlay, namespace)?;
    vm.globals.resize_with(vm.global_names.len(), || Value::Undefined);
    Ok(CallResult::FramePushed)
}

/// What to run and how to parse it, from `eval()` / `exec()`'s first argument.
///
/// Text takes the calling builtin's mode; a code object brings its own.
fn snippet(builtin: Builtin, source: &Value, vm: &mut VM<'_>) -> RunResult<Snippet> {
    let name = builtin.name();
    if let Value::Ref(id) = source
        && let HeapData::Code(code) = vm.heap.get(*id)
    {
        return Ok(Snippet {
            text: Arc::from(code.source()),
            mode: code.mode(),
            top_level_await: code.top_level_await(),
        });
    }
    let mode = match builtin {
        Builtin::Eval => CodeMode::Eval,
        Builtin::Exec => CodeMode::Exec,
    };
    Ok(Snippet {
        text: snippet_source(name, source, vm)?,
        mode,
        top_level_await: false,
    })
}

/// The snippet's text: a `str`, or `bytes` decoded as UTF-8.
fn snippet_source(name: &str, source: &Value, vm: &mut VM<'_>) -> RunResult<Arc<str>> {
    let bytes: &[u8] = match source {
        Value::InternString(id) => return Ok(Arc::from(vm.interns.get_str(*id))),
        Value::InternBytes(id) => vm.interns.get_bytes(*id),
        Value::Ref(id) => match vm.heap.get(*id) {
            HeapData::Str(s) => return Ok(Arc::from(s.as_str())),
            HeapData::Bytes(b) => b.as_slice(),
            _ => return Err(source_type_error(name)),
        },
        _ => return Err(source_type_error(name)),
    };
    match str::from_utf8(bytes) {
        Ok(text) => Ok(Arc::from(text)),
        Err(err) => {
            let bad = bytes[err.valid_up_to()];
            let line = bytes[..err.valid_up_to()].split(|b| *b == b'\n').count();
            Err(SimpleException::new_msg(
                ExcType::SyntaxError,
                format!(
                    "Non-UTF-8 code starting with '\\x{bad:02x}' on line {line}, but no encoding declared; \
                     see https://peps.python.org/pep-0263/ for details (<string>, line {line})"
                ),
            )
            .into())
        }
    }
}

/// `TypeError` for a `source` that is neither text nor bytes.
///
/// `compile()` names an AST object where `eval()` / `exec()` name a code
/// object, exactly as CPython does, although Monty has neither to offer.
fn source_type_error(name: &str) -> RunError {
    let takes = if name == "compile" { "AST" } else { "code" };
    ExcType::type_error(format!("{name}() arg 1 must be a string, bytes or {takes} object"))
}

/// Validates the `globals` argument: `None`, or a dict whose id is returned (borrowed).
fn globals_dict(globals: &Value, builtin: Builtin, vm: &mut VM<'_>) -> RunResult<Option<HeapId>> {
    match globals {
        Value::None => Ok(None),
        Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::Dict(_)) => Ok(Some(*id)),
        other => {
            let ty = other.py_type(vm);
            Err(match builtin {
                // CPython distinguishes a non-dict mapping (anything subscriptable) from the rest.
                Builtin::Eval
                    if matches!(
                        ty,
                        Type::List | Type::Tuple | Type::Str | Type::Bytes | Type::Range | Type::Deque
                    ) =>
                {
                    ExcType::type_error("globals must be a real dict; try eval(expr, {}, mapping)")
                }
                Builtin::Eval => ExcType::type_error("globals must be a dict"),
                Builtin::Exec => ExcType::type_error(format!(
                    "exec() globals must be a dict, not {}",
                    ty.name(vm.heap, vm.interns)
                )),
            })
        }
    }
}

/// Validates the `locals` argument: `None`, or a dict whose id is returned (borrowed).
fn locals_dict(locals: &Value, builtin: Builtin, vm: &mut VM<'_>) -> RunResult<Option<HeapId>> {
    match locals {
        Value::None => Ok(None),
        Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::Dict(_)) => Ok(Some(*id)),
        other => Err(match builtin {
            Builtin::Eval => ExcType::type_error("locals must be a mapping"),
            Builtin::Exec => ExcType::type_error(format!(
                "locals must be a mapping or None, not {}",
                other.py_type(vm).name(vm.heap, vm.interns)
            )),
        }),
    }
}

/// Arguments of `compile(source, filename, mode, flags=0, dont_inherit=False, optimize=-1)`.
#[derive(FromArgs)]
#[from_args(name = "compile")]
struct CompileArgs {
    source: Value,
    filename: Value,
    mode: Value,
    #[from_args(default = Value::Int(0))]
    flags: Value,
    #[from_args(default = Value::Bool(false))]
    dont_inherit: Value,
    #[from_args(default = Value::Int(-1))]
    optimize: Value,
}

/// Implementation of the `compile()` builtin function.
///
/// Parses `source` in `mode`, so a syntax error reaches the caller here rather
/// than where the code runs, and answers a [`Code`] carrying the source. Monty
/// compiles a snippet against the namespace it runs in, which `compile()` does
/// not know, so the bytecode is built by `eval()` / `exec()` instead; see
/// `limitations/eval_exec.md`.
pub fn builtin_compile(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let CompileArgs {
        source,
        filename,
        mode,
        flags,
        dont_inherit,
        optimize,
    } = CompileArgs::from_args(args, vm)?;
    defer_drop!(source, vm);
    defer_drop!(filename, vm);
    defer_drop!(mode, vm);
    defer_drop!(flags, vm);
    defer_drop!(dont_inherit, vm);
    defer_drop!(optimize, vm);

    // CPython's clinic converts `filename` before the body reads `mode` or
    // `source`, so a bad filename wins over both.
    let filename = compile_filename(filename, vm)?;
    let mode = compile_mode(mode, vm)?;
    let top_level_await = compile_flags(flags, vm)?;
    compile_optimize(optimize, vm)?;
    // `dont_inherit` only governs which `__future__` features the caller passes
    // down, and Monty has none, so any value is accepted and does nothing.
    let _ = dont_inherit;

    let text = snippet_source("compile", source, vm)?;
    parse_for_compile(&text, mode, vm)?;
    let code = Code::new(Box::from(&*text), filename, mode, top_level_await);
    Ok(Value::Ref(vm.heap.allocate(HeapData::Code(Box::new(code)))))
}

/// Parses `text` and throws the result away, so `compile()` raises a
/// `SyntaxError` where CPython does.
///
/// The overlay it parses into is dropped with it: nothing is interned, and the
/// source is parsed again wherever the code object is run.
fn parse_for_compile(text: &Arc<str>, mode: CodeMode, vm: &mut VM<'_>) -> RunResult<()> {
    if text.contains('\0') {
        return Err(
            SimpleException::new_msg(ExcType::SyntaxError, "source code string cannot contain null bytes").into(),
        );
    }
    let mut overlay = CompileInterns::new(vm.interns);
    let filename_id = overlay.add_eval_source(Arc::clone(text));
    match mode {
        CodeMode::Exec => parse_module_with_filename_id(text, filename_id, &mut overlay).map(|_| ()),
        CodeMode::Eval => {
            let trimmed = text.trim_start();
            let skipped = u32::try_from(text.len() - trimmed.len()).unwrap_or(u32::MAX);
            parse_expression_with_interner(trimmed, filename_id, &mut overlay)
                .map(|_| ())
                .map_err(|e| e.shifted(skipped))
        }
    }
    .map_err(|e| e.into_run_error(text))
}

/// Validates `compile()`'s `filename`: a `str`, or `bytes` decoded as UTF-8.
///
/// CPython also takes an `os.PathLike`; Monty's `Path` is not one a snippet can
/// build, so text is the whole surface.
fn compile_filename(filename: &Value, vm: &mut VM<'_>) -> RunResult<Box<str>> {
    match filename {
        Value::InternString(id) => Ok(Box::from(vm.interns.get_str(*id))),
        Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::Str(_)) => {
            let HeapData::Str(s) = vm.heap.get(*id) else {
                unreachable!("matched above")
            };
            Ok(Box::from(s.as_str()))
        }
        other => {
            // `snippet_source` gives the same decoding and the same errors.
            let text = snippet_source("compile", other, vm).map_err(|_| {
                ExcType::type_error(format!(
                    "expected str, bytes or os.PathLike object, not {}",
                    other.py_type(vm).name(vm.heap, vm.interns)
                ))
            })?;
            Ok(Box::from(&*text))
        }
    }
}

/// Validates `compile()`'s `mode`. `'single'` is refused: it would have to echo
/// the value of an expression statement, which Monty has no `sys.displayhook`
/// for.
fn compile_mode(mode: &Value, vm: &mut VM<'_>) -> RunResult<CodeMode> {
    let bad_mode = || ExcType::value_error("compile() mode must be 'exec', 'eval' or 'single'");
    let Some(text) = mode.as_either_str(vm.heap) else {
        return Err(bad_mode());
    };
    match text.as_str(vm.interns) {
        "exec" => Ok(CodeMode::Exec),
        "eval" => Ok(CodeMode::Eval),
        "single" => Err(ExcType::not_implemented("compile() does not yet support the 'single' mode").into()),
        _ => Err(bad_mode()),
    }
}

/// Reads `compile()`'s `flags`, reporting whether the body may `await` at its
/// top level.
///
/// `ast.PyCF_ALLOW_TOP_LEVEL_AWAIT` is the one flag Monty takes; every other
/// flag CPython has selects a `__future__` feature or an AST form Monty does
/// not have.
fn compile_flags(flags: &Value, vm: &mut VM<'_>) -> RunResult<bool> {
    match flags {
        Value::Int(0) => Ok(false),
        Value::Int(PY_CF_ALLOW_TOP_LEVEL_AWAIT) => Ok(true),
        Value::Int(_) | Value::Bool(_) => Err(ExcType::value_error("compile(): unrecognised flags")),
        other => Err(ExcType::type_error_bad_arg_named(
            "compile",
            "flags",
            "int",
            other.py_type(vm).cpython_arg_name(vm.heap, vm.interns),
        )),
    }
}

/// Validates `compile()`'s `optimize`. Monty compiles one way, so only the
/// default `-1` ("use the interpreter's own level") means anything.
fn compile_optimize(optimize: &Value, vm: &mut VM<'_>) -> RunResult<()> {
    match optimize {
        Value::Int(-1) => Ok(()),
        Value::Int(0..=2) => {
            Err(ExcType::not_implemented("compile() does not yet support the optimize argument").into())
        }
        Value::Int(_) | Value::Bool(_) => Err(ExcType::value_error("compile(): invalid optimize value")),
        other => Err(ExcType::type_error_bad_arg_named(
            "compile",
            "optimize",
            "int",
            other.py_type(vm).cpython_arg_name(vm.heap, vm.interns),
        )),
    }
}
