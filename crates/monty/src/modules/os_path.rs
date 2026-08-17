//! Implementation of the `os.path` submodule.
//!
//! Only the pure lexical `normpath` so far: it touches no filesystem, so unlike
//! the `os` functions it needs no host round-trip. The sandbox presents a POSIX
//! view on every host, so this is `posixpath` regardless of where Monty runs.
//!
//! `os.path` is also reachable as an attribute of `os` (set by
//! [`super::os::create_module`]), which is what makes `import os.path` followed
//! by `os.path.normpath(...)` work.

use crate::{
    args::{ArgValues, FromArgs},
    bytecode::VM,
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunResult},
    heap::{HeapData, HeapId},
    intern::StaticStrings,
    modules::ModuleFunctions,
    os_dispatch::value_to_owned_string,
    types::{Module, Type, str::allocate_string},
    value::Value,
};

/// `os.path` module functions — each variant is a Python-visible callable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::Display, serde::Serialize, serde::Deserialize)]
#[strum(serialize_all = "snake_case")]
pub(crate) enum OsPathFunctions {
    Normpath,
}

/// Static mapping of attribute names to functions for module creation.
const OS_PATH_FUNCTIONS: &[(StaticStrings, OsPathFunctions)] = &[(StaticStrings::Normpath, OsPathFunctions::Normpath)];

/// Creates the `os.path` module on the heap.
///
/// # Panics
/// Panics if the required strings have not been pre-interned during prepare phase.
pub fn create_module(vm: &mut VM<'_>) -> HeapId {
    let mut module = Module::new(StaticStrings::OsPath);
    for (name, func) in OS_PATH_FUNCTIONS {
        module.set_attr(*name, Value::ModuleFunction(ModuleFunctions::OsPath(*func)), vm);
    }
    vm.heap.allocate(HeapData::Module(Box::new(module)))
}

/// Dispatches a call to an `os.path` module function.
pub(super) fn call(vm: &mut VM<'_>, function: OsPathFunctions, args: ArgValues) -> RunResult<Value> {
    match function {
        OsPathFunctions::Normpath => call_normpath(vm, args),
    }
}

/// Argument shape for `normpath(path)`.
///
/// CPython 3.14 resolves `posixpath.normpath` to `posix._path_normpath`, whose
/// `PyArg_ParseTupleAndKeywords` format carries the name — so every error names
/// the C function rather than `normpath`, and arity counts positionals plus
/// keywords together (`style = c_named` + `at_most_total`).
#[derive(FromArgs)]
#[from_args(name = "_path_normpath", style = c_named, at_most_total)]
struct NormpathArgs {
    path: Value,
}

/// `os.path.normpath(path)` — collapse `.`, `..` and repeated separators.
///
/// Purely lexical: symlinks are never resolved, so the result can name a
/// different file than the input, exactly as in CPython.
fn call_normpath(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let NormpathArgs { path } = NormpathArgs::from_args(args, vm)?;
    defer_drop!(path, vm);
    let Some(text) = value_to_owned_string(path, vm.heap, vm.interns) else {
        // `bytes` paths are the one kind CPython takes here that Monty never
        // will, so naming them in the accepted set would list the very type
        // just refused — mirroring `os.rs`'s `PathAccepts` narrowing.
        let accepts = if path.py_type_heap(vm.heap) == Type::Bytes {
            "string or os.PathLike"
        } else {
            "string, bytes or os.PathLike"
        };
        let type_name = path.py_type_name_heap(vm.heap, vm.interns);
        return Err(ExcType::type_error_os_path(
            "_path_normpath",
            "path",
            accepts,
            &type_name,
        ));
    };
    Ok(allocate_string(normpath_posix(&text), vm.heap))
}

/// The lexical normalization itself, over an already-extracted path string.
///
/// Follows `posixpath.normpath`'s pure-Python body: split the root off, drop
/// empty and `.` components, and let `..` pop a component unless it would climb
/// past the root or past another `..`. The result is bounded by the input's
/// length, so a plain `String` needs no `StringBuilder` preflight.
fn normpath_posix(path: &str) -> String {
    if path.is_empty() {
        return ".".to_owned();
    }
    let (root, rest) = split_root(path);
    let mut comps: Vec<&str> = Vec::new();
    for comp in rest.split('/') {
        if comp.is_empty() || comp == "." {
            continue;
        }
        // A `..` is kept when there is nothing to climb out of: a relative path
        // with no component yet, or one whose last component is itself `..`.
        // Under a root it is dropped outright, since `/..` is `/` on POSIX.
        if comp != ".." || (root.is_empty() && comps.is_empty()) || comps.last() == Some(&"..") {
            comps.push(comp);
        } else if !comps.is_empty() {
            comps.pop();
        }
    }
    let normalized = format!("{root}{}", comps.join("/"));
    if normalized.is_empty() {
        ".".to_owned()
    } else {
        normalized
    }
}

/// Splits POSIX's leading-slash root off `path`, as `posixpath.splitroot` does.
///
/// POSIX reserves *exactly* two leading slashes for an implementation-defined
/// meaning, so `//a` keeps both while `///a` collapses to one.
fn split_root(path: &str) -> (&str, &str) {
    // Every index below lands on a `/`, so slicing cannot split a character.
    if !path.starts_with('/') {
        ("", path)
    } else if !path[1..].starts_with('/') || path[2..].starts_with('/') {
        ("/", &path[1..])
    } else {
        ("//", &path[2..])
    }
}
