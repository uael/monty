//! The step budget: deterministic where the duration limit cannot be.

use monty::{MontyRepl, MontyRun};
use monty_types::{CompileOptions, MontyObject, PrintWriter, ResourceLimits, ResourceTracker};

fn run_with(code: &str, limits: ResourceLimits) -> Result<MontyObject, String> {
    let runner = MontyRun::new(code.to_owned(), "steps.py", vec![], CompileOptions::default()).unwrap();
    runner
        .run(vec![], ResourceTracker::new(limits), PrintWriter::Disabled)
        .map_err(|e| e.to_string())
}

#[test]
fn a_spin_trips_the_step_budget_with_a_named_error() {
    let err = run_with(
        "n = 0\nwhile True:\n    n += 1\n",
        ResourceLimits::default().max_steps(100_000),
    )
    .unwrap_err();
    assert!(err.contains("step limit exceeded"), "got: {err}");
}

#[test]
fn two_identical_spins_trip_at_the_same_count() {
    // The trip message names both the limit and the executed count, so two
    // byte-identical messages prove the count is deterministic across runs.
    let code = "n = 0\nwhile True:\n    n += 1\n";
    let a = run_with(code, ResourceLimits::default().max_steps(50_000)).unwrap_err();
    let b = run_with(code, ResourceLimits::default().max_steps(50_000)).unwrap_err();
    assert!(a.contains("step limit exceeded"), "got: {a}");
    assert_eq!(a, b, "the two trips name different counts");
}

#[test]
fn a_budget_wide_enough_never_fires() {
    let code = "total = 0\nfor i in range(50_000):\n    total += i * i\ntotal\n";
    let got = run_with(code, ResourceLimits::default().max_steps(100_000_000)).unwrap();
    let again = run_with(code, ResourceLimits::default().max_steps(100_000_000)).unwrap();
    assert_eq!(got, again);
}

#[test]
fn no_step_limit_changes_nothing() {
    let got = run_with("sum(range(100))\n", ResourceLimits::default()).unwrap();
    assert_eq!(got, MontyObject::Int(4950));
}

#[test]
fn the_budget_is_not_catchable() {
    // A step budget is enforcement, not a condition sandboxed code may outrun:
    // a bare `except` around the spin must not swallow it.
    let err = run_with(
        "n = 0\ntry:\n    while True:\n        n += 1\nexcept BaseException:\n    n = -1\n",
        ResourceLimits::default().max_steps(50_000),
    )
    .unwrap_err();
    assert!(err.contains("step limit exceeded"), "got: {err}");
}

/// A budget for one call, taken from the record that asked for it: the session
/// keeps its own ceiling, and the per-call bound is measured from the steps
/// already spent so the same source under the same budget always trips at the
/// same count.
#[test]
fn a_per_call_budget_bounds_one_feed_at_a_time() {
    let spin = "n = 0\nwhile n < 200_000:\n    n += 1\n";
    let mut repl = MontyRepl::new("repl.py", ResourceTracker::default(), CompileOptions::default());

    repl.tracker().begin_call_steps(Some(5_000));
    let first = repl
        .feed_run(spin, vec![], PrintWriter::Disabled)
        .unwrap_err()
        .to_string();
    assert!(first.contains("call step limit exceeded"), "got: {first}");

    // A second feed of the same source under the same budget must fail with
    // the same message: the count is call-relative, so what the first spent
    // does not leak into it.
    repl.tracker().begin_call_steps(Some(5_000));
    let second = repl
        .feed_run(spin, vec![], PrintWriter::Disabled)
        .unwrap_err()
        .to_string();
    assert_eq!(first, second);

    // Disarmed, the session runs on: the overrun ended a feed, not the session.
    repl.tracker().begin_call_steps(None);
    repl.feed_run(spin, vec![], PrintWriter::Disabled).unwrap();
    let after = repl.feed_run("n\n", vec![], PrintWriter::Disabled).unwrap();
    assert_eq!(after.value, MontyObject::Int(200_000));
}

/// The session's own ceiling keeps applying underneath a per-call budget, so a
/// generous per-call bound cannot buy more than the session has left.
#[test]
fn the_session_ceiling_still_bounds_a_generously_budgeted_call() {
    let mut repl = MontyRepl::new(
        "repl.py",
        ResourceTracker::new(ResourceLimits::default().max_steps(50_000)),
        CompileOptions::default(),
    );
    repl.tracker().begin_call_steps(Some(u64::MAX));
    let err = repl
        .feed_run("n = 0\nwhile True:\n    n += 1\n", vec![], PrintWriter::Disabled)
        .unwrap_err()
        .to_string();
    assert!(err.contains("step limit exceeded"), "got: {err}");
    assert!(
        !err.contains("call step"),
        "the session ceiling must report itself: {err}"
    );
}
