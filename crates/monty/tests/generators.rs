//! Generators, where Monty answers differently on purpose.
//!
//! What matches CPython is proven in `test_cases/generator__basics.py`, which
//! runs against both interpreters. Here is the boundary: what a generator body
//! may not contain, and what the saved frame survives.

use monty::{Dump, MontyRepl, Session, SessionRef, dump};
use monty_types::{CompileOptions, MontyObject, PrintWriter, ResourceTracker};

fn session() -> MontyRepl {
    MontyRepl::new("gen.py", ResourceTracker::default(), CompileOptions::default())
}

fn feed(repl: &mut MontyRepl, code: &str) -> MontyObject {
    repl.feed_run(code, vec![], PrintWriter::Stdout).unwrap()
}

fn feed_err(repl: &mut MontyRepl, code: &str) -> String {
    repl.feed_run(code, vec![], PrintWriter::Stdout)
        .unwrap_err()
        .to_string()
}

fn t(value: bool) -> MontyObject {
    MontyObject::bool(value)
}

/// A generator's return value belongs on the `StopIteration` it raises, and an
/// exception here carries a message rather than a Python object. Refused while
/// that is true, so nobody reads a value that was never carried.
#[test]
fn a_generator_may_not_return_a_value() {
    let mut repl = session();
    let error = feed_err(&mut repl, "def f():\n    yield 1\n    return 2");
    assert!(
        error.contains("returning a value from a generator is not supported"),
        "{error}"
    );
    // A bare return, and falling off the end, are the ordinary way to stop.
    feed(&mut repl, "def g():\n    yield 1\n    return");
    assert_eq!(feed(&mut repl, "list(g()) == [1]"), t(true));
    // `return None` is the same statement written out.
    feed(&mut repl, "def h():\n    yield 1\n    return None");
    assert_eq!(feed(&mut repl, "list(h()) == [1]"), t(true));
}

/// An `async def` that yields is an async generator, which needs the
/// `__aiter__` / `__anext__` protocol rather than this one.
#[test]
fn an_async_function_may_not_yield() {
    let mut repl = session();
    let error = feed_err(&mut repl, "async def f():\n    yield 1");
    assert!(
        error.contains("'yield' inside an async function is not supported"),
        "{error}"
    );
}

/// `yield` is only meaningful where there is a frame to suspend, which is
/// CPython's rule and its wording. A class body is refused earlier still, by
/// the parser's own limit on what a class body may hold.
#[test]
fn yield_outside_a_function_is_refused() {
    let mut repl = session();
    let error = feed_err(&mut repl, "yield 1");
    assert!(error.contains("'yield' outside function"), "{error}");
}

/// The surface is `__iter__`, `__next__` and `send`; `close` and `throw` are
/// not here yet, and say so rather than being quietly absent.
#[test]
fn the_surface_is_iter_next_and_send() {
    let mut repl = session();
    feed(&mut repl, "def f():\n    yield 1\ng = f()");
    assert_eq!(feed(&mut repl, "g.__next__() == 1"), t(true));
    for method in ["close", "throw", "gi_frame", "gi_running"] {
        let error = feed_err(&mut repl, &format!("f().{method}"));
        assert!(
            error.contains(&format!("'generator' object has no attribute '{method}'")),
            "{error}"
        );
    }
}

/// Resuming a generator from inside itself is the one re-entrancy its frame
/// cannot answer, because the frame is already on the stack.
#[test]
fn a_generator_cannot_resume_itself() {
    let mut repl = session();
    feed(
        &mut repl,
        "def f():\n    yield next(holder[0])\nholder = [None]\nholder[0] = f()",
    );
    let error = feed_err(&mut repl, "next(holder[0])");
    assert!(error.contains("generator already executing"), "{error}");
}

/// A suspended generator is a frame and its values, so it survives a dump and
/// carries on from the `yield` it stopped at.
#[test]
fn a_suspended_generator_survives_dump_and_load() {
    let mut repl = session();
    feed(
        &mut repl,
        "def counter():\n    total = 0\n    for i in range(4):\n        total += i\n        yield total\ng = counter()",
    );
    assert_eq!(feed(&mut repl, "next(g) == 0"), t(true));
    assert_eq!(feed(&mut repl, "next(g) == 1"), t(true));

    let bytes = dump("gen.py", None, SessionRef::Idle(&repl)).unwrap();
    let mut back = match Dump::load(&bytes).unwrap().state {
        Session::Idle(repl) => *repl,
        _ => panic!("dumped an idle session, loaded something else"),
    };
    // The running total is frame state, so it proves the locals came back too.
    assert_eq!(feed(&mut back, "next(g) == 3"), t(true));
    assert_eq!(feed(&mut back, "next(g) == 6"), t(true));
    assert_eq!(feed(&mut back, "list(g) == []"), t(true));
}

/// A generator expression is still a list comprehension, so it is eager and is
/// a `list`. Only `def`-with-`yield` builds a generator today.
#[test]
fn a_generator_expression_is_not_yet_lazy() {
    let mut repl = session();
    assert_eq!(feed(&mut repl, "type(x for x in [1, 2]).__name__ == 'list'"), t(true));
    feed(&mut repl, "def f():\n    yield 1");
    assert_eq!(feed(&mut repl, "type(f()).__name__ == 'generator'"), t(true));
}
