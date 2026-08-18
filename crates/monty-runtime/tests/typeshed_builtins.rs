//! The vendored `builtins.pyi` and the interpreter's builtin namespace must
//! describe the same vocabulary.
//!
//! `crates/monty-typeshed/update.py` trims upstream's `builtins.pyi` down to
//! two hand-written allow lists, and nothing regenerates them when the
//! interpreter grows a builtin. Drift is silent in the worse direction: a name
//! the sandbox resolves but the stub omits makes `monty -t` report
//! `unresolved-reference` on code that runs, so the type checker rejects the
//! sandbox's own vocabulary. The opposite drift is quieter still: a stubbed
//! name the interpreter never binds type-checks clean and then raises
//! `NameError`.
//!
//! This crate is the only one that depends on both the interpreter and the
//! type checker, so it is the only place the two surfaces can be compared.

use monty::MontyRun;
use monty_type_checking::{SourceFile, TypeChecker};
use monty_types::{CompileOptions, PrintWriter, ResourceTracker, TypeCheckingConfig};

/// The vendored stub, read from the same file `build.rs` zips into the crate.
const BUILTINS_STUB: &str = include_str!("../../monty-typeshed/vendor/typeshed/stdlib/builtins.pyi");

/// Names the stub binds that the interpreter deliberately does not.
///
/// `object` is the root every other stub class inherits from and
/// `UnicodeError` is the declared base of the two codec errors, so filtering
/// either away would leave the classes that name it describing nothing. Both
/// are recorded in `limitations/builtins.md` as names only the type checker
/// resolves.
const STUB_ONLY: [&str; 2] = ["object", "UnicodeError"];

/// Every name the builtin namespace binds, straight from the interpreter:
/// `vars(builtins)` is assembled from the same three sources a bare name
/// resolves against, so it cannot drift from what code can actually reference.
fn builtin_namespace() -> Vec<String> {
    let code = "import builtins\nfor name in sorted(vars(builtins)):\n    print(name)";
    let run = MontyRun::new(code.to_owned(), "builtins.py", vec![], CompileOptions::default()).unwrap();
    let mut printed = String::new();
    run.run(
        vec![],
        ResourceTracker::default(),
        PrintWriter::collect_string(&mut printed),
    )
    .unwrap();
    printed.lines().map(str::to_owned).collect()
}

/// Module-level `class` / `def` names bound by the stub.
///
/// Indentation is tracked rather than matched at column zero because the
/// version-conditional blocks upstream writes (`if sys.version_info >= ...`)
/// indent real module-level definitions; only definitions inside an open
/// `class` body are members rather than names.
fn stub_namespace() -> Vec<String> {
    let mut names = Vec::new();
    let mut open_classes: Vec<usize> = Vec::new();
    for line in BUILTINS_STUB.lines() {
        let trimmed = line.trim_start();
        let Some(keyword) = trimmed.strip_prefix("class ").or_else(|| trimmed.strip_prefix("def ")) else {
            continue;
        };
        let indent = line.len() - trimmed.len();
        while open_classes.last().is_some_and(|open| *open >= indent) {
            open_classes.pop();
        }
        let is_member = !open_classes.is_empty();
        if trimmed.starts_with("class ") {
            open_classes.push(indent);
        }
        let name = keyword
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .next()
            .unwrap_or_default();
        if !is_member && !name.is_empty() && !name.starts_with('_') {
            names.push(name.to_owned());
        }
    }
    names.sort();
    names.dedup();
    names
}

/// Whether the type checker resolves a bare reference to `name`.
fn resolves(name: &str) -> bool {
    let code = format!("_ = {name}\n");
    let mut checker = TypeChecker::default();
    checker
        .run(
            &SourceFile::new(&code, "reference.py"),
            None,
            TypeCheckingConfig::default(),
        )
        .expect("type check should not fail internally")
        .is_none()
}

#[test]
fn every_builtin_the_interpreter_binds_is_typed() {
    let untyped: Vec<String> = builtin_namespace().into_iter().filter(|n| !resolves(n)).collect();
    assert!(
        untyped.is_empty(),
        "these builtins run but fail type checking; add them to ALLOWED_FUNCTIONS / \
         ALLOWED_CLASSES in crates/monty-typeshed/update.py and run `make update-typeshed`: {untyped:?}"
    );
}

#[test]
fn the_stub_binds_no_name_the_interpreter_lacks() {
    let namespace = builtin_namespace();
    let extra: Vec<String> = stub_namespace()
        .into_iter()
        .filter(|n| !namespace.contains(n) && !STUB_ONLY.contains(&n.as_str()))
        .collect();
    assert!(
        extra.is_empty(),
        "these names type-check and then raise NameError at runtime; drop them from \
         crates/monty-typeshed/update.py or implement them: {extra:?}"
    );
}

#[test]
fn the_stub_only_names_are_still_stub_only() {
    let namespace = builtin_namespace();
    for name in STUB_ONLY {
        assert!(
            resolves(name),
            "`{name}` is documented as resolving for the type checker but no longer does"
        );
        assert!(
            !namespace.contains(&name.to_owned()),
            "`{name}` is now a real builtin; drop it from STUB_ONLY and from the \
             stub-only note in limitations/builtins.md"
        );
    }
}
