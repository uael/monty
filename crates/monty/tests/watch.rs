//! Tests for a watched function: each return of a call of it goes to the host
//! first, with the value and the frames that stand at the call, and the caller
//! takes the value whatever the host answers.

use monty::{Dump, MontyRepl, ReplProgress, Session, SessionRef, dump};
use monty_types::{CompileOptions, MontyObject, PrintWriter, ResourceTracker};

/// A session whose first snippet defines `make` in a file named `<string>`, as
/// `exec` names it, and a function of the same name in a file of its own.
fn session() -> MontyRepl {
    let mut repl = MontyRepl::new("repl.py", ResourceTracker::default(), CompileOptions::default());
    let made = "class Name(str):\n    pass\n\ndef make(n):\n    return Name(f'act{n}')";
    let other = "def make(n):\n    return n";
    let feed =
        format!("ns = {{}}\nexec({made:?}, ns)\nmake = ns['make']\nexec(compile({other:?}, 'other', 'exec'), {{}})");
    repl.feed_run(&feed, vec![], PrintWriter::Stdout).unwrap();
    repl
}

/// What a word compiled under the name `rung3` gives, and each return that went
/// to the host: the value and the line of the word where its call stood. The
/// host answers each with nothing, which the caller does not take.
fn ran(repl: MontyRepl) -> (MontyObject, Vec<(String, u32)>) {
    let word = "a = make(1)\nb = [make(i) for i in (2, 3)]\nc = type(a).__name__";
    let feed = format!("exec(compile({word:?}, 'rung3', 'exec'))\n(a, b, c)");
    let mut progress = repl.feed_start(&feed, vec![], PrintWriter::Stdout).unwrap();
    let mut seen = Vec::new();
    loop {
        match progress {
            ReplProgress::FunctionCall(call) => {
                assert_eq!(call.function_name, "make");
                let value = call.args.args().next().unwrap().as_str().unwrap().to_owned();
                let line = call
                    .stack()
                    .iter()
                    .rfind(|frame| frame.filename == "rung3")
                    .unwrap()
                    .start
                    .line;
                seen.push((value, line));
                progress = call.resume(MontyObject::none(), PrintWriter::Stdout).unwrap();
            }
            ReplProgress::Complete { value, .. } => return (value, seen),
            other => panic!("unexpected progress: {other:?}"),
        }
    }
}

#[test]
fn a_watched_return_goes_to_the_host_and_the_caller_takes_the_value() {
    let mut repl = session();
    assert_eq!(repl.watch("<string>", "make"), 1);
    let (value, seen) = ran(repl);
    let made = |text: &str| MontyObject::string(text);
    let shown = MontyObject::tuple([
        made("act1"),
        MontyObject::list([made("act2"), made("act3")]),
        made("Name"),
    ]);
    assert_eq!(value, shown);
    let at = |text: &str, line| (text.to_owned(), line);
    assert_eq!(seen, [at("act1", 1), at("act2", 2), at("act3", 2)]);
}

#[test]
fn a_dump_of_the_session_keeps_the_watch() {
    let mut repl = session();
    repl.watch("<string>", "make");
    let bytes = dump("repl.py", None, &[], SessionRef::Idle(&repl)).unwrap();
    let Session::Idle(loaded) = Dump::load(&bytes).unwrap().state else {
        panic!("dumped an idle session, loaded something else");
    };
    assert_eq!(ran(*loaded).1.len(), 3);
}

#[test]
fn a_session_with_no_watch_calls_no_host() {
    assert_eq!(ran(session()).1, []);
}

#[test]
fn a_run_that_no_host_answers_takes_a_watched_return_at_once() {
    let mut repl = session();
    repl.watch("<string>", "make");
    let value = repl.feed_run("make(4)", vec![], PrintWriter::Stdout).unwrap();
    assert_eq!(value, MontyObject::string("act4"));
}
