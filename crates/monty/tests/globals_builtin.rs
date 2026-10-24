//! Tests for `globals()` where Monty's namespace model diverges from CPython's,
//! so the dual-run `test_cases/builtin__eval_exec.py` cannot cover them.
//!
//! Module globals are dense slots, not a dict, so `globals()` at module scope
//! is a fresh snapshot of the bound ones rather than the module namespace
//! itself. Under an `exec()` / `eval()` globals dict there is a real dict and
//! `globals()` hands it back; that case matches CPython and is covered by the
//! dual-run fixture.

use monty::MontyRun;
use monty_types::{CompileOptions, MontyObject};

/// Runs `code` to completion with no resource limits and returns the value of
/// its final expression.
fn eval(code: &str) -> MontyObject {
    MontyRun::new(code.to_owned(), "test.py", vec![], CompileOptions::default())
        .unwrap()
        .run_no_limits(vec![])
        .unwrap()
}

#[test]
fn snapshot_holds_the_bound_globals_only() {
    // CPython also carries `__name__`, `__builtins__` and the rest of the
    // module dunders; Monty resolves those on read and stores none of them.
    assert_eq!(
        eval("x = 1\ndef f(): pass\nsorted(globals())"),
        MontyObject::list(vec![
            MontyObject::string("f".to_owned()),
            MontyObject::string("x".to_owned())
        ])
    );
}

#[test]
fn writes_to_the_snapshot_do_not_bind_a_global() {
    assert_eq!(
        eval("g = globals()\ng['added'] = 1\n'added' in globals()"),
        MontyObject::bool(false)
    );
}

#[test]
fn a_later_binding_is_absent_from_an_earlier_snapshot() {
    assert_eq!(eval("g = globals()\nlater = 1\n'later' in g"), MontyObject::bool(false));
}

#[test]
fn each_call_builds_a_new_dict() {
    assert_eq!(eval("globals() is globals()"), MontyObject::bool(false));
    assert_eq!(eval("x = 1\nglobals() == globals()"), MontyObject::bool(true));
    // At module scope CPython returns the one module namespace for both.
    assert_eq!(eval("globals() is locals()"), MontyObject::bool(false));
}

#[test]
fn a_function_sees_a_snapshot_of_the_module_globals() {
    assert_eq!(
        eval("x = 1\ndef f():\n    return globals()['x']\nf()"),
        MontyObject::int(1)
    );
    assert_eq!(
        eval("def f():\n    globals()['added'] = 1\n    return 'added' in globals()\nf()"),
        MontyObject::bool(false)
    );
}
