//! What `object` refuses, which `test_cases/builtins__object.py` cannot pin
//! because that file runs on CPython too and CPython allows both.
//!
//! Monty gives `object` no instances and no parameters: a bare instance of it
//! carries nothing the sandbox can read, and CPython refuses the subscript as
//! well. Everything the two interpreters agree on lives in the test case.

use monty::{MontyRepl, MontyRun};
use monty_types::{CompileOptions, MontyObject, MontyType, PrintWriter, ResourceTracker};

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

/// `object` is not `type`, and the boundary says so.
///
/// Inside a session the two are different values: they answer differently to
/// `__name__` and are not identical. Crossing both as one type object told a
/// host they were the same, and left it unable to hand `object` back.
#[test]
fn object_crosses_as_itself_and_not_as_type() {
    let mut repl = MontyRepl::new("object.py", ResourceTracker::default(), CompileOptions::default());
    let feed = |repl: &mut MontyRepl, code: &str| repl.feed_run(code, vec![], PrintWriter::Stdout).unwrap().value;

    let crossed = feed(&mut repl, "object");
    assert_eq!(crossed, MontyObject::Type(MontyType::Object));
    let MontyObject::Type(ty) = &crossed else {
        unreachable!("just matched")
    };
    assert_eq!(ty.name(), "object", "the host is told what the session calls it");
    assert_eq!(feed(&mut repl, "object.__name__"), MontyObject::String("object".to_owned()));
    assert_eq!(feed(&mut repl, "object is type"), MontyObject::Bool(false));

    // And what crossed out is what comes back: the session's own `object`,
    // not a twin of it.
    assert_eq!(
        repl.feed_run(
            "held is object",
            vec![("held".to_owned(), MontyObject::Type(MontyType::Object))],
            PrintWriter::Stdout,
        )
        .unwrap()
        .value,
        MontyObject::Bool(true)
    );
}
