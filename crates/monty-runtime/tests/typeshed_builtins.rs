//! The vendored `builtins.pyi` and the interpreter bind the same builtin names.
//!
//! `crates/monty-typeshed/update.py` trims upstream's `builtins.pyi` to hand-written allow lists, and nothing
//! regenerates them when the interpreter grows a builtin. A name the interpreter binds and the stub lacks makes
//! `monty -t` refuse code that runs; a name the stub binds and the interpreter lacks passes `monty -t` and then
//! raises `NameError`.
//!
//! This crate is the one that depends on both the interpreter and the type checker, so the two are compared here,
//! for a session with the host's operating system and for one with none, which lacks `open` on both sides.

use monty::MontyRun;
use monty_type_checking::{SourceFile, TypeChecker};
use monty_types::{BuiltinsFunctions, CompileOptions, ExcType, MontyType, SandboxOs, TypeCheckingConfig};
use strum::VariantNames;

/// The vendored stub, read from the same file `build.rs` zips into the crate.
const BUILTINS_STUB: &str = include_str!("../../monty-typeshed/vendor/typeshed/stdlib/builtins.pyi");

/// Every name the stub binds at its top: each `class`, `def` and assignment outside a class body, whatever version
/// or platform block it stands in.
fn stub_names() -> Vec<String> {
    let mut names = Vec::new();
    let mut classes: Vec<usize> = Vec::new();
    for line in BUILTINS_STUB.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('@') {
            continue;
        }
        let indent = line.len() - trimmed.len();
        while classes.last().is_some_and(|open| *open >= indent) {
            classes.pop();
        }
        if !classes.is_empty() {
            continue;
        }
        let rest = trimmed
            .strip_prefix("class ")
            .or_else(|| trimmed.strip_prefix("def "))
            .or_else(|| trimmed.strip_prefix("async def "));
        if trimmed.starts_with("class ") {
            classes.push(indent);
        }
        let head = rest.unwrap_or(trimmed);
        let name: String = head.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        // `else:` has the shape of an annotation and binds nothing.
        let assigns = name != "else" && head[name.len()..].trim_start().starts_with([':', '=']);
        if !name.is_empty() && (rest.is_some() || assigns) {
            names.push(name);
        }
    }
    names
}

/// Every name the checker or the interpreter could bind as a builtin: the names of the interpreter's own builtin
/// functions, exception types and types, and every name the stub binds.
fn candidates() -> Vec<String> {
    let mut names: Vec<String> = BuiltinsFunctions::VARIANTS
        .iter()
        .chain(ExcType::VARIANTS)
        .chain(MontyType::VARIANTS)
        .map(|name| (*name).to_owned())
        .chain(stub_names())
        .filter(|name| !name.starts_with('_') && name.chars().all(|c| c.is_alphanumeric() || c == '_'))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Whether the interpreter runs a bare reference to `name` in a session with the given `os`.
fn runs(name: &str, os: SandboxOs) -> bool {
    let code = format!("got = {name}\n");
    let options = CompileOptions {
        os,
        ..CompileOptions::default()
    };
    let mut run = MontyRun::new(code, "reference.py", vec![], options).unwrap();
    match run.run_no_limits(vec![]) {
        Ok(_) => true,
        Err(no) => {
            assert!(
                no.to_string().contains("NameError"),
                "{name} raised what no missing name raises: {no}"
            );
            false
        }
    }
}

/// Whether the type checker of a session with the given `os` resolves a bare reference to `name`.
fn resolves(name: &str, os: SandboxOs) -> bool {
    let code = format!("got = {name}\n");
    TypeChecker::new(os)
        .run(
            &SourceFile::new(&code, "reference.py"),
            None,
            TypeCheckingConfig::default(),
        )
        .expect("type check should not fail internally")
        .is_none()
}

#[test]
fn the_stub_binds_a_builtin_exactly_when_the_interpreter_binds_it() {
    assert_eq!(names_apart(SandboxOs::Host), Vec::new(), "{KEEP_IN_STEP}");
}

#[test]
fn without_os_the_stub_binds_a_builtin_exactly_when_the_interpreter_binds_it() {
    assert_eq!(names_apart(SandboxOs::Absent), Vec::new(), "{KEEP_IN_STEP}");
    assert!(runs("open", SandboxOs::Host) && !runs("open", SandboxOs::Absent));
}

/// What to do when the stub and the interpreter disagree.
const KEEP_IN_STEP: &str = "the stub and the interpreter disagree on these names, as (name, runs, type-checks); keep \
    ALLOWED_FUNCTIONS, ALLOWED_CLASSES and ALLOWED_NAMES in crates/monty-typeshed/update.py in step with the \
    interpreter, and what a session without an operating system lacks in step between `Builtins::find` and \
    crates/monty-typeshed/src/without_os.rs";

/// The candidates that the interpreter and the type checker of a session with the given `os` disagree on, as
/// `(name, runs, type-checks)`.
fn names_apart(os: SandboxOs) -> Vec<(String, bool, bool)> {
    candidates()
        .into_iter()
        .map(|name| {
            let (ran, typed) = (runs(&name, os), resolves(&name, os));
            (name, ran, typed)
        })
        .filter(|(_, ran, typed)| ran != typed)
        .collect()
}

#[test]
fn the_candidates_hold_a_name_of_every_source() {
    // A function, an exception type and a type of the interpreter, and an assignment of the stub.
    let names = candidates();
    for name in ["getattr", "GeneratorExit", "list", "Ellipsis"] {
        assert!(names.contains(&name.to_owned()), "{name} is no candidate: {names:?}");
    }
}
