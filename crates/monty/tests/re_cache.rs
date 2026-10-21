//! Tests for the cache of compiled patterns across a call of the host: the
//! snapshot that the call pauses on carries the cache to the VM that resumes it.

use monty::{MontyRepl, ReplFunctionCall, ReplProgress};
use monty_types::{CompileOptions, MontyObject, PrintWriter, ResourceLimits, ResourceTracker};

fn called(progress: ReplProgress) -> ReplFunctionCall {
    let ReplProgress::FunctionCall(call) = progress else {
        panic!("expected a call of the host, got {progress:?}")
    };
    call
}

/// How many compiled patterns the paused run holds, as its debug form says.
fn held(call: &ReplFunctionCall) -> String {
    let shown = format!("{call:?}");
    let at = shown.find("RePatternCache(").expect("the snapshot shows its cache");
    shown[at..].split(')').next().unwrap().to_owned() + ")"
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
}
