//! Replay identity, and the portability of a dump across architectures.
//!
//! An embedder's whole reason for running Python here rather than in CPython
//! is that a run can be replayed exactly: the same program, fed the same host
//! answers, must produce the same result *and* the same interleaving, and a
//! session paused on one machine must resume on another. Neither property is
//! observable from a single run, so both are asserted here.
//!
//! The two cross-machine tests are driven by environment variables so CI can
//! split them across runners of different architectures. With neither set they
//! do the whole exchange through a temporary directory, so a plain `cargo test`
//! still asserts the same thing on one machine.

use std::{env, fs, path::Path};

use monty::{Dump, MontyRun, RunProgress, Session, SessionRef, dump};
use monty_types::{CompileOptions, MontyObject, NameLookupResult, PrintWriter, ResourceTracker};

/// Directory a run writes its dump into, one file per architecture.
const WRITE_DIR: &str = "MONTY_DETERMINISM_DUMP_WRITE";
/// Directory holding dumps written by other runs, every one of which must
/// resume here to the same result.
const READ_DIR: &str = "MONTY_DETERMINISM_DUMP_READ";

/// Three coroutines whose order is decided by the in-sandbox clock, each
/// pausing on a host call part-way through.
///
/// The timers are what make the interleaving worth asserting: `b` has the
/// shortest sleep and finishes first, so the recorded order is a property of
/// the scheduler rather than of the source order. `fetch` is resolved by the
/// host, so the run suspends three times and there is a mid-flight state to
/// dump. The whole event log is folded into the returned value, which is what
/// lets a resumed run on another machine be compared against a full run here.
const PROGRAM: &str = "\
import asyncio

order = []


async def step(tag, delay):
    await asyncio.sleep(delay)
    order.append(tag)
    order.append(fetch(tag))
    return tag


async def main():
    done = await asyncio.gather(step('a', 3), step('b', 1), step('c', 2))
    return '|'.join(done) + '#' + ','.join(order)


result = asyncio.run(main())
print(result)
result
";

/// Runs [`PROGRAM`] to completion, answering every host call the same way.
///
/// `dump_at_first_suspension` receives the bytes of the paused session the
/// first time the run hands control back, which is the state the cross-machine
/// tests exchange.
fn run(printed: &mut String, mut dump_at_first_suspension: Option<&mut Vec<u8>>) -> String {
    let runner = MontyRun::new(PROGRAM.to_owned(), "determinism.py", vec![], CompileOptions::default())
        .expect("program should compile");
    let mut progress = runner
        .start(vec![], ResourceTracker::default(), PrintWriter::collect_string(printed))
        .expect("program should start");
    loop {
        if let Some(out) = dump_at_first_suspension.take() {
            if !matches!(progress, RunProgress::Complete(_)) {
                *out = dump("determinism.py", None, SessionRef::Running(&progress)).expect("dump should serialize");
            }
        }
        progress = advance(progress, printed);
        if let RunProgress::Complete(value) = progress {
            let MontyObject::String(result) = value else {
                panic!("program should return a string, got {value:?}");
            };
            return result;
        }
    }
}

/// Answers one host call. Every answer is a pure function of the request, so
/// the host contributes nothing that could differ between two runs.
fn advance(progress: RunProgress, printed: &mut String) -> RunProgress {
    let print = PrintWriter::collect_string(printed);
    match progress {
        complete @ RunProgress::Complete(_) => complete,
        RunProgress::NameLookup(lookup) => {
            assert_eq!(lookup.name, "fetch", "the program looks up no other name");
            let result = NameLookupResult::Value(MontyObject::Function {
                name: lookup.name.clone(),
                docstring: None,
            });
            lookup.resume(result, print).expect("name lookup should resume")
        }
        RunProgress::FunctionCall(call) => {
            let MontyObject::String(tag) = &call.args[0] else {
                panic!("fetch takes a string tag, got {:?}", call.args);
            };
            let answer = MontyObject::String(format!("{tag}!"));
            call.resume(answer, print).expect("function call should resume")
        }
        RunProgress::ResolveFutures(state) => {
            panic!("no call is resumed as a future, so the run never parks on one: {state:?}")
        }
        RunProgress::OsCall(call) => panic!("the program makes no OS call: {:?}", call.function_call),
    }
}

/// Round-trips a paused run through the dump format and resumes it.
fn resume_dump(bytes: &[u8], printed: &mut String) -> String {
    let Session::Running(progress) = Dump::load(bytes).expect("dump should load").state else {
        panic!("a running session was dumped, so a running session must load")
    };
    let mut progress = *progress;
    loop {
        progress = advance(progress, printed);
        if let RunProgress::Complete(value) = progress {
            let MontyObject::String(result) = value else {
                panic!("program should return a string, got {value:?}");
            };
            return result;
        }
    }
}

/// A run's own name for its dump, so several architectures can write into one
/// directory and every reader can tell whose dump it is resuming.
fn dump_name() -> String {
    format!("{}-{}.dump", env::consts::ARCH, env::consts::OS)
}

#[test]
fn the_same_program_replays_identically() {
    let mut first_printed = String::new();
    let first = run(&mut first_printed, None);
    let mut second_printed = String::new();
    let second = run(&mut second_printed, None);

    assert_eq!(first, second, "two runs of one program returned different values");
    assert_eq!(
        first_printed, second_printed,
        "two runs of one program printed different output"
    );

    // Pinned rather than only compared with itself: a change that made both
    // runs agree on a *different* interleaving would otherwise pass. `b` sleeps
    // least and finishes first, `a` most and finishes last, and each task's
    // host call lands before the next task wakes.
    assert_eq!(first, "a|b|c#b,b!,c,c!,a,a!");
    assert_eq!(first_printed, "a|b|c#b,b!,c,c!,a,a!\n");
}

#[test]
fn a_paused_run_resumes_to_the_same_result() {
    let mut whole_printed = String::new();
    let whole = run(&mut whole_printed, None);

    let mut bytes = Vec::new();
    let mut paused_printed = String::new();
    let paused = run(&mut paused_printed, Some(&mut bytes));
    assert_eq!(paused, whole);
    assert!(!bytes.is_empty(), "the run should suspend at least once");

    let mut resumed_printed = String::new();
    assert_eq!(resume_dump(&bytes, &mut resumed_printed), whole);
    assert_eq!(
        resumed_printed, whole_printed,
        "the resumed run printed something the whole run did not"
    );
}

#[test]
fn a_dump_written_on_another_machine_resumes_here() {
    let expected = run(&mut String::new(), None);

    match (env::var_os(WRITE_DIR), env::var_os(READ_DIR)) {
        (Some(write), None) => {
            let mut bytes = Vec::new();
            run(&mut String::new(), Some(&mut bytes));
            let path = Path::new(&write).join(dump_name());
            fs::create_dir_all(&write).expect("dump directory should be creatable");
            fs::write(&path, &bytes).expect("dump should be writable");
            println!("wrote {} ({} bytes)", path.display(), bytes.len());
        }
        (None, Some(read)) => {
            let mut resumed = 0;
            for entry in fs::read_dir(&read).expect("dump directory should be readable") {
                let path = entry.expect("directory entry should be readable").path();
                if path.extension().is_none_or(|ext| ext != "dump") {
                    continue;
                }
                let bytes = fs::read(&path).expect("dump should be readable");
                let mut printed = String::new();
                assert_eq!(
                    resume_dump(&bytes, &mut printed),
                    expected,
                    "{} resumed to a different result",
                    path.display()
                );
                println!("resumed {}", path.display());
                resumed += 1;
            }
            assert!(resumed > 0, "{} held no dumps to resume", read.to_string_lossy());
        }
        (None, None) => {
            // No exchange configured: do both halves here, so the test asserts
            // the same thing rather than quietly passing.
            let dir = tempfile::tempdir().expect("temp directory should be creatable");
            let mut bytes = Vec::new();
            run(&mut String::new(), Some(&mut bytes));
            let path = dir.path().join(dump_name());
            fs::write(&path, &bytes).expect("dump should be writable");
            let read_back = fs::read(&path).expect("dump should be readable");
            assert_eq!(resume_dump(&read_back, &mut String::new()), expected);
        }
        (Some(_), Some(_)) => panic!("set {WRITE_DIR} or {READ_DIR}, not both"),
    }
}
