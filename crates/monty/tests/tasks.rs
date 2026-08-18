//! Source compiled for the session's own event loop.
//!
//! A host can compile a snippet against a namespace and be handed the coroutine
//! that runs it, having run nothing. What starts it is sandboxed code, so the
//! snippet becomes one more task in the one loop: it binds the namespace it was
//! compiled against, its own host calls suspend the session as any task's do,
//! and its value reaches whoever awaited it.

use monty::{Dump, MontyRepl, ReplProgress, ScopeId, Session, SessionRef, dump};
use monty_types::{CompileOptions, ExtFunctionResult, FeedOutcome, MontyObject, PrintWriter, ResourceTracker};

/// The module the session runs: an event loop whose main takes coroutines the
/// host leaves in `queue` and runs each as a task of its own, recording what it
/// produced.
///
/// `await step()` is a host call the host answers with a future, so every turn
/// of the loop ends with every task parked and the host holding the session;
/// that is where it adds to `queue`. Answering `True` ends the loop.
const LOOP_SOURCE: &str = r"
import asyncio

queue = []
seen = []

async def watch(coro):
    try:
        value, returned = await coro
        seen.append(('value', value, returned))
    except Exception as e:
        seen.append(('raised', type(e).__name__, str(e)))

async def main():
    while True:
        while queue:
            asyncio.create_task(watch(queue.pop(0)))
        await asyncio.sleep(0)
        if await step():
            return len(seen)

asyncio.run(main())
";

fn session() -> MontyRepl {
    MontyRepl::new("tasks.py", ResourceTracker::default(), CompileOptions::default())
}

fn feed(repl: &mut MontyRepl, code: &str) -> MontyObject {
    repl.feed_run(code, vec![], PrintWriter::Stdout).unwrap().value
}

/// Feeds `code` to `scope`, leaving the session selecting whatever it did.
fn feed_in(repl: &mut MontyRepl, scope: ScopeId, code: &str) -> MontyObject {
    let was = repl.namespace();
    repl.select_namespace(scope).unwrap();
    let out = repl.feed_run(code, vec![], PrintWriter::Stdout);
    repl.select_namespace(was).unwrap();
    out.unwrap().value
}

/// Hands a coroutine to the running loop and gives up the host's reference to
/// it, which the queue has taken over.
fn enqueue(progress: &mut ReplProgress, task: &MontyObject) {
    let MontyObject::SessionRef { id, .. } = *task else {
        panic!("a task crosses as a reference, got {task:?}");
    };
    progress
        .run_in(
            None,
            "queue.append(task)",
            vec![(
                "task".to_owned(),
                MontyObject::SessionRef {
                    id,
                    repr: String::new(),
                },
            )],
            PrintWriter::Stdout,
        )
        .unwrap();
    assert!(progress.release(id), "the queue owns it now");
}

/// What one run of [`LOOP_SOURCE`] left behind.
struct Run {
    repl: MontyRepl,
    outcome: FeedOutcome,
    /// Every host call made from inside the loop other than `step`, in the
    /// order the host saw them: what a task asked for, and with what.
    calls: Vec<(String, Vec<MontyObject>)>,
}

/// Starts the loop and answers its host calls until it completes.
///
/// `turn` is called once per turn of the loop, with the session held at the
/// point where every task in it is parked on the host, and says whether the
/// loop should stop. A host call from inside a task is answered with `None`, so
/// a task that calls out keeps running.
fn drive(repl: MontyRepl, mut turn: impl FnMut(&mut ReplProgress, usize) -> bool) -> Run {
    let mut progress = repl
        .feed_start(LOOP_SOURCE, vec![], PrintWriter::Stdout)
        .expect("the loop suspends at its first host call");
    let mut calls = Vec::new();
    let mut parked_step = None;
    let mut turns = 0;
    loop {
        if let ReplProgress::Complete { .. } = progress {
            let (repl, outcome) = progress.into_complete().expect("just matched");
            return Run { repl, outcome, calls };
        }
        let mut stop = false;
        if matches!(progress, ReplProgress::ResolveFutures(_)) {
            assert!(turns < 100, "the loop was never told to stop");
            stop = turn(&mut progress, turns);
            turns += 1;
        }
        progress = match progress {
            ReplProgress::FunctionCall(call) => {
                if call.function_name == "step" {
                    parked_step = Some(call.call_id);
                    call.resume_pending(PrintWriter::Stdout)
                } else {
                    calls.push((call.function_name.clone(), call.args.clone()));
                    call.resume(MontyObject::None, PrintWriter::Stdout)
                }
            }
            ReplProgress::ResolveFutures(state) => {
                let call_id = parked_step.take().expect("the loop parks its main on `step`");
                state.resume(
                    vec![(call_id, ExtFunctionResult::Return(MontyObject::Bool(stop)))],
                    PrintWriter::Stdout,
                )
            }
            other => panic!("unexpected suspension: {other:?}"),
        }
        .expect("the loop keeps running");
    }
}

/// Answers whatever the loop asks until it completes, telling it to stop at the
/// first chance.
///
/// Every test that suspends ends through here: a suspension dropped rather than
/// finished leaves the values on its stack unreleased, which the
/// reference-counting checks report, and that is true of any suspension.
fn finish(progress: ReplProgress) -> MontyRepl {
    let mut progress = progress;
    loop {
        progress = match progress {
            ReplProgress::Complete { repl, .. } => return repl,
            ReplProgress::FunctionCall(call) => {
                if call.function_name == "step" {
                    call.resume_pending(PrintWriter::Stdout)
                } else {
                    call.resume(MontyObject::None, PrintWriter::Stdout)
                }
            }
            ReplProgress::ResolveFutures(state) => {
                let answers = state
                    .pending_call_ids()
                    .iter()
                    .map(|call_id| (*call_id, ExtFunctionResult::Return(MontyObject::Bool(true))))
                    .collect();
                state.resume(answers, PrintWriter::Stdout)
            }
            other => panic!("unexpected suspension: {other:?}"),
        }
        .expect("the loop runs out");
    }
}

fn seen(repl: &mut MontyRepl) -> Vec<MontyObject> {
    match feed(repl, "seen") {
        MontyObject::List(items) => items,
        other => panic!("`seen` is a list, got {other:?}"),
    }
}

fn value(v: MontyObject, returned: bool) -> MontyObject {
    MontyObject::Tuple(vec![
        MontyObject::String("value".to_owned()),
        v,
        MontyObject::Bool(returned),
    ])
}

fn text(s: &str) -> MontyObject {
    MontyObject::String(s.to_owned())
}

// ---------------------------------------------------------------------------
// What a task is
// ---------------------------------------------------------------------------

/// The whole shape: a snippet compiled while the session is suspended, started
/// by sandboxed code, suspending to the host from inside its own body, and
/// handing its value to what awaited it.
#[test]
fn a_task_runs_in_the_loop_and_calls_out_to_the_host() {
    let mut repl = session();
    feed(&mut repl, "log = []");

    let mut run = drive(repl, |progress, turn| {
        if turn == 0 {
            let task = progress
                .feed_task_in(
                    None,
                    "log.append(ask('from the task'))\n'done'",
                    vec![],
                    None,
                    None,
                    PrintWriter::Stdout,
                )
                .unwrap();
            enqueue(progress, &task);
        }
        turn > 1
    });

    // The task's own call reached the host as an ordinary suspension of the
    // session, carrying what the task passed it.
    assert_eq!(run.calls, vec![("ask".to_owned(), vec![text("from the task")])]);
    // Its value reached what awaited it, and its effect landed in the namespace.
    assert_eq!(seen(&mut run.repl), vec![value(text("done"), false)]);
    assert_eq!(feed(&mut run.repl, "log"), MontyObject::List(vec![MontyObject::None]));
    assert_eq!(run.outcome.value, MontyObject::Int(1), "one task ran and was recorded");
}

// ---------------------------------------------------------------------------
// What awaiting one produces
// ---------------------------------------------------------------------------

/// A written `return` is told apart from a trailing expression and from running
/// out of statements, in the value, because the value is all the awaiter gets.
#[test]
fn a_written_return_is_told_apart_from_falling_off_the_end() {
    let repl = session();
    let mut run = drive(repl, |progress, turn| {
        if turn == 0 {
            for code in ["x = 1\nreturn 'said'", "x = 2\n'trailing'", "x = 3"] {
                let task = progress
                    .feed_task_in(None, code, vec![], None, None, PrintWriter::Stdout)
                    .unwrap();
                enqueue(progress, &task);
            }
        }
        turn > 1
    });

    assert_eq!(
        seen(&mut run.repl),
        vec![
            value(text("said"), true),
            value(text("trailing"), false),
            value(MontyObject::None, false),
        ]
    );
}

/// A `return` under a `try` still says it returned, and the `finally` still runs.
#[test]
fn a_return_out_of_a_try_is_still_a_return() {
    let repl = session();
    let mut run = drive(repl, |progress, turn| {
        if turn == 0 {
            let task = progress
                .feed_task_in(
                    None,
                    "marks = []\ntry:\n    return 'early'\nfinally:\n    marks.append('cleaned')\n",
                    vec![],
                    None,
                    None,
                    PrintWriter::Stdout,
                )
                .unwrap();
            enqueue(progress, &task);
        }
        turn > 1
    });

    assert_eq!(seen(&mut run.repl), vec![value(text("early"), true)]);
    assert_eq!(feed(&mut run.repl, "marks"), MontyObject::List(vec![text("cleaned")]));
}

/// An exception the source raises is raised at whoever awaits it, unswallowed,
/// and the loop carries on.
#[test]
fn an_exception_reaches_whoever_awaits_the_task() {
    let repl = session();
    let mut run = drive(repl, |progress, turn| {
        if turn == 0 {
            for code in ["raise ValueError('by hand')", "'after'"] {
                let task = progress
                    .feed_task_in(None, code, vec![], None, None, PrintWriter::Stdout)
                    .unwrap();
                enqueue(progress, &task);
            }
        }
        turn > 1
    });

    assert_eq!(
        seen(&mut run.repl),
        vec![
            MontyObject::Tuple(vec![text("raised"), text("ValueError"), text("by hand")]),
            value(text("after"), false),
        ]
    );
}

// ---------------------------------------------------------------------------
// Which namespace it binds
// ---------------------------------------------------------------------------

/// What a task binds is its namespace's afterwards, as a feed's is, and a later
/// task compiled against the same namespace reads it.
#[test]
fn what_a_task_binds_is_the_namespace_afterwards() {
    let repl = session();
    let mut run = drive(repl, |progress, turn| {
        if turn == 0 {
            let task = progress
                .feed_task_in(None, "made = 7\nreturn made", vec![], None, None, PrintWriter::Stdout)
                .unwrap();
            enqueue(progress, &task);
        }
        if turn == 1 {
            let task = progress
                .feed_task_in(None, "made * 6", vec![], None, None, PrintWriter::Stdout)
                .unwrap();
            enqueue(progress, &task);
        }
        turn > 2
    });

    assert_eq!(
        seen(&mut run.repl),
        vec![value(MontyObject::Int(7), true), value(MontyObject::Int(42), false)]
    );
    assert_eq!(feed(&mut run.repl, "made"), MontyObject::Int(7));
}

/// A task compiled against another namespace binds that one, and reads it: the
/// loop switches namespace to run it and back again for everything else.
#[test]
fn a_task_runs_in_the_namespace_it_was_compiled_against() {
    let mut repl = session();
    feed(&mut repl, "who = 'the loop'");
    let other = repl.copy_namespace(repl.namespace()).unwrap();
    feed_in(&mut repl, other, "who = 'the other'");

    let mut run = drive(repl, |progress, turn| {
        if turn == 0 {
            let task = progress
                .feed_task_in(
                    Some(other),
                    "mine = who\nreturn who",
                    vec![],
                    None,
                    None,
                    PrintWriter::Stdout,
                )
                .unwrap();
            enqueue(progress, &task);
        }
        turn > 1
    });

    assert_eq!(seen(&mut run.repl), vec![value(text("the other"), true)]);
    assert_eq!(feed_in(&mut run.repl, other, "mine"), text("the other"));
    assert_eq!(feed(&mut run.repl, "who"), text("the loop"), "the loop kept its own");
    let err = run.repl.probe_scoped("mine", vec![], PrintWriter::Stdout).unwrap_err();
    assert!(
        err.to_string().contains("NameError"),
        "the binding landed in the other namespace only, got {err}"
    );
}

/// Inputs land in the namespace the task runs in, as a feed's inputs do, so
/// what the host supplies is still there when the task is finally started.
#[test]
fn inputs_are_the_namespace_the_task_runs_in() {
    let repl = session();
    let mut run = drive(repl, |progress, turn| {
        if turn == 0 {
            let task = progress
                .feed_task_in(
                    None,
                    "supplied * 2",
                    vec![("supplied".to_owned(), MontyObject::Int(21))],
                    None,
                    None,
                    PrintWriter::Stdout,
                )
                .unwrap();
            enqueue(progress, &task);
        }
        turn > 1
    });

    assert_eq!(seen(&mut run.repl), vec![value(MontyObject::Int(42), false)]);
    assert_eq!(feed(&mut run.repl, "supplied"), MontyObject::Int(21));
}

/// A handle naming no namespace is refused, and nothing is compiled.
#[test]
fn a_task_for_a_namespace_the_session_does_not_have_is_refused() {
    let mut repl = session();
    let gone = repl.create_namespace().unwrap();
    assert!(repl.release_namespace(gone));

    let err = repl
        .feed_task_in(Some(gone), "1", vec![], None, None, PrintWriter::Stdout)
        .unwrap_err()
        .to_string();
    assert!(err.contains("no such namespace"), "got {err}");
}

/// Source that does not compile is refused where it is compiled, not where it
/// would have run.
#[test]
fn source_that_does_not_compile_is_refused_at_once() {
    let mut repl = session();
    let err = repl
        .feed_task_in(None, "def (:", vec![], None, None, PrintWriter::Stdout)
        .unwrap_err()
        .to_string();
    assert!(err.contains("SyntaxError"), "got {err}");
}

// ---------------------------------------------------------------------------
// Where a host can ask for one
// ---------------------------------------------------------------------------

/// Every arm a host can be holding compiles one, and asking does not move the
/// suspension it was asked at.
///
/// The [`ReplProgress::ResolveFutures`] arm is where a host driving an event
/// loop spends its time, and so where a task is most often added: the loop's
/// own tasks are all parked on the host, and the one added here is ready the
/// moment the host answers.
#[test]
fn a_task_is_compiled_at_whatever_suspension_the_host_holds() {
    let mut held = Vec::new();
    let run = drive(session(), |progress, turn| {
        assert!(
            matches!(progress, ReplProgress::ResolveFutures(_)),
            "a turn of the loop ends with every task parked on the host"
        );
        if turn == 0 {
            let task = progress
                .feed_task_in(
                    None,
                    "'made while nothing could run'",
                    vec![],
                    None,
                    None,
                    PrintWriter::Stdout,
                )
                .unwrap();
            enqueue(progress, &task);
        }
        held.push(turn);
        turn > 1
    });
    let mut repl = run.repl;
    assert_eq!(held, vec![0, 1, 2]);
    assert_eq!(
        seen(&mut repl),
        vec![value(text("made while nothing could run"), false)]
    );

    // The three arms an idle loop never reaches, each holding a session that
    // has not finished a snippet.
    for (code, expect) in [
        ("ask(1)", "a call out to the host"),
        ("unbound_name", "a name only the host can resolve"),
        ("open('/nonexistent/for/monty')", "a call out to the operating system"),
    ] {
        let mut progress = repl
            .feed_start(code, vec![], PrintWriter::Stdout)
            .unwrap_or_else(|_| panic!("{expect} suspends"));
        assert!(
            !matches!(progress, ReplProgress::Complete { .. }),
            "{expect} suspends rather than completing"
        );
        let task = progress
            .feed_task_in(
                None,
                "'compiled at a suspension'",
                vec![],
                None,
                None,
                PrintWriter::Stdout,
            )
            .unwrap();
        assert!(matches!(task, MontyObject::SessionRef { .. }), "{expect}");
        let MontyObject::SessionRef { id, .. } = task else {
            unreachable!("just matched")
        };
        assert!(progress.release(id));
        repl = progress.into_repl();
    }
}

/// An idle session compiles one too, for a loop that has not started yet.
#[test]
fn a_task_can_be_compiled_before_the_loop_starts() {
    let mut repl = session();
    let task = repl
        .feed_task_in(None, "'made before the loop'", vec![], None, None, PrintWriter::Stdout)
        .unwrap();
    let MontyObject::SessionRef { id, .. } = task else {
        panic!("a task crosses as a reference");
    };
    repl.feed_run(
        "waiting = []\nwaiting.append(task)",
        vec![(
            "task".to_owned(),
            MontyObject::SessionRef {
                id,
                repr: String::new(),
            },
        )],
        PrintWriter::Stdout,
    )
    .unwrap();
    assert!(repl.release(id));

    assert_eq!(
        feed(&mut repl, "import asyncio\nasyncio.run(waiting.pop())"),
        MontyObject::Tuple(vec![text("made before the loop"), MontyObject::Bool(false)])
    );
}

/// A coroutine the host is still holding survives a dump: it names a place in
/// the heap, and the heap is what the dump carries.
#[test]
fn a_task_survives_a_dump_of_the_session_holding_it() {
    let mut repl = session();
    let task = repl
        .feed_task_in(None, "'made before the dump'", vec![], None, None, PrintWriter::Stdout)
        .unwrap();
    let MontyObject::SessionRef { id, .. } = task else {
        panic!("a task crosses as a reference");
    };

    let bytes = dump("tasks.py", None, SessionRef::Idle(&repl)).unwrap();
    drop(repl);
    let Session::Idle(mut woken) = Dump::load(&bytes).unwrap().state else {
        panic!("dumped an idle session, loaded something else");
    };

    woken
        .feed_run(
            "held = task",
            vec![(
                "task".to_owned(),
                MontyObject::SessionRef {
                    id,
                    repr: String::new(),
                },
            )],
            PrintWriter::Stdout,
        )
        .unwrap();
    assert!(woken.release(id));
    assert_eq!(
        feed(&mut woken, "import asyncio\nasyncio.run(held)"),
        MontyObject::Tuple(vec![text("made before the dump"), MontyObject::Bool(false)])
    );
}

/// A task started twice is refused as any coroutine is, rather than running
/// twice against the namespace.
#[test]
fn a_task_runs_once() {
    let mut repl = session();
    let task = repl
        .feed_task_in(
            None,
            "count = count + 1\ncount",
            vec![],
            None,
            None,
            PrintWriter::Stdout,
        )
        .unwrap();
    let MontyObject::SessionRef { id, .. } = task else {
        panic!("a task crosses as a reference");
    };
    feed(&mut repl, "count = 0");
    repl.feed_run(
        "held = task",
        vec![(
            "task".to_owned(),
            MontyObject::SessionRef {
                id,
                repr: String::new(),
            },
        )],
        PrintWriter::Stdout,
    )
    .unwrap();
    assert!(repl.release(id));

    assert_eq!(
        feed(&mut repl, "import asyncio\nasyncio.run(held)"),
        MontyObject::Tuple(vec![MontyObject::Int(1), MontyObject::Bool(false)])
    );
    let err = repl
        .feed_run("asyncio.run(held)", vec![], PrintWriter::Stdout)
        .unwrap_err()
        .to_string();
    assert!(err.contains("coroutine"), "got {err}");
    assert_eq!(feed(&mut repl, "count"), MontyObject::Int(1));
}

// ---------------------------------------------------------------------------
// A budget of its own
// ---------------------------------------------------------------------------

/// Fuel given to one task bounds that task and nothing else: the overrun is
/// raised where it can be caught, the loop keeps running, and a task beside it
/// with no budget of its own runs as long as it likes.
#[test]
fn a_task_can_be_given_fuel_without_bounding_the_session() {
    let repl = session();
    let mut run = drive(repl, |progress, turn| {
        if turn == 0 {
            let bounded = progress
                .feed_task_in(
                    None,
                    "spun = 0\nwhile True:\n    spun = spun + 1\n",
                    vec![],
                    None,
                    Some(1_000),
                    PrintWriter::Stdout,
                )
                .unwrap();
            enqueue(progress, &bounded);
            // Beside it, one that spins as long as the same budget would allow
            // and is not bounded by it.
            let free = progress
                .feed_task_in(
                    None,
                    "n = 0\nwhile n < 20000:\n    n = n + 1\nn",
                    vec![],
                    None,
                    None,
                    PrintWriter::Stdout,
                )
                .unwrap();
            enqueue(progress, &free);
        }
        turn > 2
    });

    let seen = seen(&mut run.repl);
    let MontyObject::Tuple(overrun) = &seen[0] else {
        panic!("the bounded task ended with something recorded, got {:?}", seen[0]);
    };
    assert_eq!(overrun[0], text("raised"));
    assert_eq!(overrun[1], text("RuntimeError"));
    let MontyObject::String(message) = &overrun[2] else {
        panic!("a message, got {:?}", overrun[2]);
    };
    assert!(
        message.contains("task step limit exceeded") && message.contains("> 1000"),
        "got {message}"
    );
    assert_eq!(seen[1], value(MontyObject::Int(20000), false), "its neighbour ran on");
    // The session itself is unharmed and still binding what the bounded task
    // wrote before it ran out.
    assert!(
        matches!(feed(&mut run.repl, "spun > 0"), MontyObject::Bool(true)),
        "the bounded task ran until it ran out"
    );
}

/// The budget is the task's own, not the session's: two tasks compiled from the
/// same source with the same budget each get it in full.
#[test]
fn each_task_is_given_its_own_fuel() {
    let repl = session();
    let mut run = drive(repl, |progress, turn| {
        if turn == 0 {
            for _ in 0..2 {
                let task = progress
                    .feed_task_in(
                        None,
                        "n = 0\nwhile n < 3000:\n    n = n + 1\nn",
                        vec![],
                        None,
                        Some(200_000),
                        PrintWriter::Stdout,
                    )
                    .unwrap();
                enqueue(progress, &task);
            }
        }
        turn > 2
    });

    assert_eq!(
        seen(&mut run.repl),
        vec![
            value(MontyObject::Int(3000), false),
            value(MontyObject::Int(3000), false),
        ]
    );
}

/// Catching the overrun does not buy more of it: the budget is exhausted, so
/// the next one cannot be caught and ends the run.
#[test]
fn catching_the_overrun_does_not_buy_more_of_it() {
    let repl = session();
    let mut progress = repl
        .feed_start(LOOP_SOURCE, vec![], PrintWriter::Stdout)
        .expect("the loop suspends at its first host call");
    let ReplProgress::FunctionCall(call) = progress else {
        panic!("the loop suspends at `step`");
    };
    let call_id = call.call_id;
    progress = call.resume_pending(PrintWriter::Stdout).unwrap();

    let task = progress
        .feed_task_in(
            None,
            "n = 0\nwhile True:\n    try:\n        while n < 1000000:\n            n = n + 1\n    except Exception:\n        n = 0\n",
            vec![],
            None,
            Some(1_000),
            PrintWriter::Stdout,
        )
        .unwrap();
    enqueue(&mut progress, &task);

    let ReplProgress::ResolveFutures(state) = progress else {
        panic!("the loop is parked on the host");
    };
    let error = state
        .resume(
            vec![(call_id, ExtFunctionResult::Return(MontyObject::Bool(false)))],
            PrintWriter::Stdout,
        )
        .expect_err("a budget nothing may catch ends the run")
        .error
        .to_string();
    assert!(error.contains("task step limit exceeded"), "got {error}");
}

/// A budget survives a dump taken while the task it bounds is parked, and what
/// the woken session charges is what was left of it.
///
/// The task spins, calls out, and spins again for exactly as long. Each stretch
/// costs about 9,000 instructions and the budget is 12,000, so the second
/// stretch on its own would fit: the task can only run out if the dump carried
/// what the first stretch had already spent. Losing the budget, or starting it
/// over, both show up as the task completing.
#[test]
fn a_budget_survives_a_dump_of_the_task_it_bounds() {
    let repl = session();
    let mut progress = repl
        .feed_start(LOOP_SOURCE, vec![], PrintWriter::Stdout)
        .expect("the loop suspends at its first host call");
    let ReplProgress::FunctionCall(call) = progress else {
        panic!("the loop suspends at `step`");
    };
    let call_id = call.call_id;
    progress = call.resume_pending(PrintWriter::Stdout).unwrap();

    let task = progress
        .feed_task_in(
            None,
            "n = 0\nwhile n < 1000:\n    n = n + 1\nwaited = ask(n)\nwhile n < 2000:\n    n = n + 1\nn",
            vec![],
            None,
            Some(12_000),
            PrintWriter::Stdout,
        )
        .unwrap();
    enqueue(&mut progress, &task);

    let ReplProgress::ResolveFutures(state) = progress else {
        panic!("the loop is parked on the host");
    };
    let progress = state
        .resume(
            vec![(call_id, ExtFunctionResult::Return(MontyObject::Bool(false)))],
            PrintWriter::Stdout,
        )
        .unwrap();

    // Dump the session while the budgeted task is parked in its own host call.
    let ReplProgress::FunctionCall(ref call) = progress else {
        panic!("the task calls out before it has spent its fuel");
    };
    assert_eq!(call.function_name, "ask");
    let bytes = dump("tasks.py", None, SessionRef::Suspended(&progress)).unwrap();
    // Dumping reads; the session it was taken from is still the host's to run
    // out, and a suspension left unfinished is one whose values nothing
    // released.
    drop(finish(progress));

    let Session::Suspended(woken) = Dump::load(&bytes).unwrap().state else {
        panic!("dumped a suspended session, loaded something else");
    };
    let mut repl = finish(*woken);
    let seen = seen(&mut repl);
    let MontyObject::Tuple(overrun) = &seen[0] else {
        panic!("the task ended with something recorded, got {:?}", seen[0]);
    };
    assert_eq!(overrun[0], text("raised"), "the woken task ran out of what was left");
    assert_eq!(overrun[1], text("RuntimeError"));
}

// ---------------------------------------------------------------------------
// The names a task can reach
// ---------------------------------------------------------------------------

/// A builtin the session has never mentioned is still the builtin when a task
/// names it.
///
/// A name a task is the first to say takes a fresh slot, and nothing has ever
/// bound it, so what answers is the builtin fallback: it reads the name back
/// from the session's own strings. Compiling while the session is suspended is
/// what makes that reading hard, since the code that is running was compiled
/// against a shorter table than the one the name was just added to.
#[test]
fn a_task_names_a_builtin_the_session_never_said() {
    let repl = session();
    let mut run = drive(repl, |progress, turn| {
        if turn == 0 {
            let task = progress
                .feed_task_in(None, "BaseException.__name__", vec![], None, None, PrintWriter::Stdout)
                .unwrap();
            enqueue(progress, &task);
        }
        turn > 1
    });
    assert_eq!(
        seen(&mut run.repl),
        vec![value(text("BaseException"), false)],
        "the task read the builtin, not an unbound global"
    );
}

/// The same, for a name said inside a function the host defined while the
/// session was suspended.
///
/// This is the shape a host uses when it wants the coroutine to catch whatever
/// its own body raised: source run now defines the function, and what the host
/// carries away is the coroutine. The builtin is named in that body, so it is
/// read when the body runs, long after the source mentioning it stopped being
/// what the session was compiling.
#[test]
fn a_function_the_host_defined_while_suspended_names_a_builtin() {
    let mut repl = session();
    // A coroutine has no copy representation, so it reaches the host as a
    // reference only when the session is told to hand such values over.
    repl.set_cross_by_reference(true);
    feed(&mut repl, "log = []");

    let mut run = drive(repl, |progress, turn| {
        if turn == 0 {
            progress
                .run_in(
                    None,
                    "async def cmd():\n    try:\n        raise ValueError('boom')\n    except BaseException as e:\n        log.append(type(e).__name__)\n    return 'ok', True\n\njob = cmd()\n",
                    vec![],
                    PrintWriter::Stdout,
                )
                .unwrap();
            let task = progress.probe_in(None, "job", vec![], PrintWriter::Stdout).unwrap();
            enqueue(progress, &task);
        }
        turn > 1
    });
    assert_eq!(
        feed(&mut run.repl, "log"),
        MontyObject::List(vec![text("ValueError")]),
        "the function caught through the builtin, not an unbound global"
    );
}

/// A task says where its source came from, so a traceback from it names the
/// snippet rather than the session.
///
/// It matters more for a task than for a feed: the frames run long after the
/// call that compiled them returned, so by the time anything raises, the name
/// is all there is to say which snippet this was.
#[test]
fn a_task_carries_the_name_its_host_gave_it() {
    let mut repl = session();
    // A coroutine reaches the host as a reference, which is how it is handed
    // back in to be awaited.
    repl.set_cross_by_reference(true);
    let task = repl
        .feed_task_in(
            None,
            "raise ValueError('boom')",
            vec![],
            Some("rung://7"),
            None,
            PrintWriter::Stdout,
        )
        .unwrap();

    let raised = repl
        .feed_run(
            "import asyncio\n\nasync def __await_it():\n    await job\n\nasyncio.run(__await_it())\n",
            vec![("job".to_owned(), task)],
            PrintWriter::Stdout,
        )
        .expect_err("the task raises where it is awaited");
    let said = format!("{raised}");
    assert!(said.contains("rung://7"), "the traceback names the rung: {said}");
    assert!(!said.contains("<task-"), "no generated name was spent on it: {said}");
}
