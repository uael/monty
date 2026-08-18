//! What `object` refuses, which `test_cases/builtins__object.py` cannot pin
//! because that file runs on CPython too and CPython allows both.
//!
//! Monty gives `object` no instances and no parameters: a bare instance of it
//! carries nothing the sandbox can read, and CPython refuses the subscript as
//! well. Everything the two interpreters agree on lives in the test case.

use monty::MontyRun;
use monty_types::{CompileOptions, PrintWriter, ResourceTracker};

/// The `TypeError` message `code` raises.
fn raises(code: &str) -> String {
    let run = MontyRun::new(code.to_owned(), "object.py", vec![], CompileOptions::default()).unwrap();
    let error = run
        .run(vec![], ResourceTracker::default(), PrintWriter::Stdout)
        .expect_err("expected a TypeError");
    error.summary()
}

#[test]
fn object_constructs_nothing() {
    assert_eq!(raises("object()"), "TypeError: cannot create 'object' instances");
}

#[test]
fn object_takes_no_parameters() {
    assert_eq!(raises("object[int]"), "TypeError: type 'object' is not subscriptable");
}
