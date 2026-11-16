//! Tests for the cache of compiled patterns across a call of the host: the
//! snapshot that the call pauses on carries the cache to the VM that resumes it.

use std::fmt::Debug;

use monty::{MontyRepl, ReplFunctionCall, ReplProgress};
use monty_types::{CompileOptions, MontyObject, PrintWriter, ResourceLimits, ResourceTracker};

fn called(progress: ReplProgress) -> ReplFunctionCall {
    let ReplProgress::FunctionCall(call) = progress else {
        panic!("expected a call of the host, got {progress:?}")
    };
    call
}

/// How many compiled patterns a session or its paused run holds, as its debug form says:
/// the cache of the session is empty while a run holds it.
fn held(state: &impl Debug) -> String {
    let shown = format!("{state:?}");
    let counts = shown.split("RePatternCache(").skip(1);
    let held: usize = counts
        .map(|one| one.split(')').next().unwrap().parse::<usize>().unwrap())
        .sum();
    format!("RePatternCache({held})")
}

#[test]
fn a_call_of_the_host_carries_the_compiled_patterns_of_the_run() {
    let repl = MontyRepl::new(
        "re_cache.py",
        ResourceTracker::new(ResourceLimits::default()),
        CompileOptions::default(),
    );
    let code = "import re\nre.search('a+', 'aa')\nfirst()\nre.search('b+', 'bb')\nsecond()";
    let call = called(repl.feed_start(code, vec![], PrintWriter::Disabled).unwrap());
    assert_eq!(call.function_name, "first");
    assert_eq!(held(&call), "RePatternCache(1)");
    let call = called(call.resume(MontyObject::none(), PrintWriter::Disabled).unwrap());
    assert_eq!(call.function_name, "second");
    // The resumed run holds the pattern it compiled before the call beside the one it compiled after.
    assert_eq!(held(&call), "RePatternCache(2)");
    drop(call.into_repl());
}

#[test]
fn a_session_carries_the_compiled_patterns_from_one_feed_to_the_next() {
    let mut repl = MontyRepl::new(
        "re_cache.py",
        ResourceTracker::new(ResourceLimits::default()),
        CompileOptions::default(),
    );
    repl.feed_run("import re\nre.search('a+', 'aa')", vec![], PrintWriter::Disabled)
        .unwrap();
    assert_eq!(held(&repl), "RePatternCache(1)");
    repl.feed_run("re.search('a+', 'aaa')", vec![], PrintWriter::Disabled)
        .unwrap();
    // The second feed found the pattern of the first and compiled nothing.
    assert_eq!(held(&repl), "RePatternCache(1)");
    repl.feed_run("re.search('b+', 'bb')", vec![], PrintWriter::Disabled)
        .unwrap();
    assert_eq!(held(&repl), "RePatternCache(2)");
}
