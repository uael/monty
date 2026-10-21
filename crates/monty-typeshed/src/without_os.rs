//! The stubs of a session with no operating system, derived from the vendored
//! typeshed when [`file_system_without_os`](crate::file_system_without_os) is
//! first called.
//!
//! Such a session lacks the `os` module, the builtin `open` and the disk
//! methods of `pathlib.Path`, so its stubs lack them too, and code that names
//! one fails type checking as it would fail at runtime. The derivation edits
//! the vendored stubs rather than holding a second copy of them, so the two
//! views cannot drift apart.

use std::{
    io::{Cursor, Read, Write},
    path::Path,
};

use ruff_python_ast::{
    Stmt,
    visitor::{Visitor, walk_stmt},
};
use ruff_python_parser::parse_module;
use ruff_text_size::{Ranged, TextRange};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::FileOptions};

/// Where the module `os` lies in the vendored stubs.
const OS_STUB: &str = "stdlib/os.pyi";

/// Where the types of `os` move. Other stubs take `PathLike` and `stat_result`
/// from `os`, and a class whose base does not resolve accepts any attribute,
/// so the types must stay resolvable. `_typeshed` exists for the type checker
/// alone, so a session that imports it fails at runtime as with `os`.
const OS_TYPES_STUB: &str = "stdlib/_typeshed/_os.pyi";

/// The module name of [`OS_TYPES_STUB`].
const OS_TYPES_MODULE: &str = "_typeshed._os";

/// Builds the zip of the stubs of a session with no operating system from the
/// zip of the vendored stubs.
///
/// # Panics
/// Panics when the vendored stubs no longer have the shape this derivation
/// edits, so that a typeshed update cannot silently give `open` or a disk
/// method back to a session without an operating system.
pub(crate) fn derive(typeshed: &'static [u8]) -> Vec<u8> {
    let mut archive = ZipArchive::new(Cursor::new(typeshed)).expect("the vendored typeshed is a zip");
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = FileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .unix_permissions(0o644);
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).expect("the vendored typeshed reads");
        let name = entry.name().to_owned();
        if entry.is_dir() {
            zip.add_directory(name, options).expect("an in-memory zip writes");
            continue;
        }
        let mut source = String::new();
        entry.read_to_string(&mut source).expect("the vendored stubs are UTF-8");
        let (name, source) = if name == OS_STUB {
            (OS_TYPES_STUB.to_owned(), source)
        } else if name == "stdlib/VERSIONS" {
            (name, without_os_version(&source))
        } else if Path::new(&name).extension().is_some_and(|extension| extension == "pyi") {
            let source = without_os_stubs(&name, &source);
            (name, source)
        } else {
            (name, source)
        };
        zip.start_file(name, options).expect("an in-memory zip writes");
        zip.write_all(source.as_bytes()).expect("an in-memory zip writes");
    }
    zip.finish().expect("an in-memory zip writes").into_inner()
}

/// The `VERSIONS` file less the line of `os`, so that `import os` does not
/// resolve. The types of `os` need no line: `_typeshed` covers its submodules.
fn without_os_version(versions: &str) -> String {
    let (os, kept): (Vec<&str>, Vec<&str>) = versions.lines().partition(|line| line.starts_with("os:"));
    assert_eq!(os.len(), 1, "`stdlib/VERSIONS` lists `os` once");
    kept.join("\n") + "\n"
}

/// One stub as a session with no operating system sees it: its imports from
/// `os` take the types from [`OS_TYPES_MODULE`], `builtins` has no `open`, and
/// `pathlib.Path` keeps only what needs no disk.
fn without_os_stubs(name: &str, source: &str) -> String {
    let parsed = parse_module(source).unwrap_or_else(|error| panic!("`{name}` parses: {error}"));
    let body = &parsed.syntax().body;
    let mut edits = OsImports::default().edits(body);
    match name {
        "stdlib/builtins.pyi" => {
            let before = edits.len();
            drop_defs(body, &|def| def == "open", &mut edits);
            assert_ne!(edits.len(), before, "`{name}` defines `open`");
        }
        "stdlib/pathlib/__init__.pyi" => {
            let path = body
                .iter()
                .find_map(|stmt| match stmt {
                    Stmt::ClassDef(class) if class.name.as_str() == "Path" => Some(class),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("`{name}` defines `Path`"));
            // `PurePath` holds the pure methods; what `Path` adds reads or
            // writes the disk, but for its constructor and `cwd`, which the
            // sandbox answers from its own working directory.
            drop_defs(&path.body, &|def| !matches!(def, "__new__" | "cwd"), &mut edits);
        }
        _ => {}
    }
    apply(source, edits)
}

/// One replacement of a range of a stub's text.
struct Edit {
    range: TextRange,
    text: String,
}

/// Collects the edits that point each import of `os`, at any depth of a stub,
/// at [`OS_TYPES_MODULE`].
#[derive(Default)]
struct OsImports {
    edits: Vec<Edit>,
}

impl OsImports {
    /// The edits for every import of `os` in `body`.
    fn edits(mut self, body: &[Stmt]) -> Vec<Edit> {
        self.visit_body(body);
        self.edits
    }
}

impl<'a> Visitor<'a> for OsImports {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::ImportFrom(import) if import.level == 0 => {
                if let Some(module) = &import.module {
                    let module_name = module.as_str();
                    assert!(
                        !module_name.starts_with("os."),
                        "no vendored stub imports from a submodule of `os`, so none is moved"
                    );
                    if module_name == "os" {
                        self.edits.push(Edit {
                            range: module.range(),
                            text: OS_TYPES_MODULE.to_owned(),
                        });
                    }
                }
            }
            Stmt::Import(import) => {
                for alias in &import.names {
                    if alias.name.as_str() == "os" {
                        // `import os` still binds `os` in the stub, now to the types.
                        let text = match &alias.asname {
                            Some(_) => OS_TYPES_MODULE.to_owned(),
                            None => format!("{OS_TYPES_MODULE} as os"),
                        };
                        self.edits.push(Edit {
                            range: alias.name.range(),
                            text,
                        });
                    }
                }
            }
            _ => walk_stmt(self, stmt),
        }
    }
}

/// Removes each function `drop` names from `body`, and from the branches of
/// each `if` in it, at any depth. A body left with no statement holds `...`,
/// so the stub stays valid Python.
fn drop_defs(body: &[Stmt], drop: &dyn Fn(&str) -> bool, edits: &mut Vec<Edit>) {
    let dropped = |stmt: &Stmt| matches!(stmt, Stmt::FunctionDef(def) if drop(def.name.as_str()));
    let emptied = body.iter().all(dropped);
    for (index, stmt) in body.iter().enumerate() {
        if dropped(stmt) {
            // A decorated function's range starts at its first decorator, so
            // an `@overload` goes with the function it decorates.
            let text = if emptied && index == 0 { "..." } else { "" };
            edits.push(Edit {
                range: stmt.range(),
                text: text.to_owned(),
            });
        } else if let Stmt::If(branch) = stmt {
            drop_defs(&branch.body, drop, edits);
            for clause in &branch.elif_else_clauses {
                drop_defs(&clause.body, drop, edits);
            }
        }
    }
}

/// Applies `edits`, which do not overlap, to `source`.
fn apply(source: &str, mut edits: Vec<Edit>) -> String {
    edits.sort_by_key(|edit| edit.range.start());
    let mut out = String::with_capacity(source.len());
    let mut at = 0;
    for edit in edits {
        out.push_str(&source[at..edit.range.start().to_usize()]);
        out.push_str(&edit.text);
        at = edit.range.end().to_usize();
    }
    out.push_str(&source[at..]);
    out
}
