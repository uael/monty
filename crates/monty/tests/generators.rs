//! Generators, where Monty answers differently on purpose.
//!
//! What matches CPython is proven in `test_cases/generator__basics.py`, which
//! runs against both interpreters. Here is the boundary: what a generator body
//! may not contain, and what the saved frame survives.

use monty::{Dump, MontyRepl, MontyRun, RunProgress, Session, SessionRef, dump};
use monty_types::{CompileOptions, ExtFunctionResult, MontyObject, NameLookupResult, PrintWriter, ResourceTracker};

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

/// Starts `code` and resolves every leading name lookup to a host function.
fn start(code: &str) -> RunProgress {
    let run = MontyRun::new(code.to_owned(), "gen.py", vec![], CompileOptions::default()).unwrap();
    let mut progress = run
        .start(vec![], ResourceTracker::default(), PrintWriter::Stdout)
        .unwrap();
    while let RunProgress::NameLookup(lookup) = progress {
        let name = lookup.name.clone();
        progress = lookup
            .resume(
                NameLookupResult::Value(MontyObject::function(name, None)),
                PrintWriter::Stdout,
            )
            .unwrap();
    }
    progress
}

/// A generator's return value reaches a `yield from` that waits for it, which
/// carries it inside the interpreter. It does not reach a resumer that hears
/// `StopIteration` instead: an exception here carries a message rather than a
/// Python object, so there is nothing for the value to travel on.
#[test]
fn a_returned_value_is_not_on_the_stop_iteration() {
    let mut repl = session();
    feed(&mut repl, "def f():\n    yield 1\n    return 2");
    // The waiter of a delegation is given it.
    feed(&mut repl, "def uses():\n    got = yield from f()\n    yield got");
    assert_eq!(feed(&mut repl, "list(uses()) == [1, 2]"), t(true));
    // The `StopIteration` a resumer catches carries nothing.
    feed(
        &mut repl,
        "g = f()\nnext(g)\ntry:\n    next(g)\nexcept StopIteration as e:\n    caught = e",
    );
    assert_eq!(feed(&mut repl, "caught.args == ()"), t(true));
    let error = feed_err(&mut repl, "caught.value");
    assert!(
        error.contains("'StopIteration' object has no attribute 'value'"),
        "{error}"
    );
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

/// The surface is `__iter__`, `__next__`, `send`, `close` and `throw`. The
/// introspection attributes are not here, and say so rather than being
/// quietly absent.
#[test]
fn the_surface_is_the_five_methods() {
    let mut repl = session();
    feed(&mut repl, "def f():\n    yield 1\ng = f()");
    assert_eq!(feed(&mut repl, "g.__next__() == 1"), t(true));
    assert_eq!(feed(&mut repl, "f().close() is None"), t(true));
    for attr in ["gi_frame", "gi_running", "gi_code", "gi_yieldfrom"] {
        let error = feed_err(&mut repl, &format!("f().{attr}"));
        assert!(
            error.contains(&format!("'generator' object has no attribute '{attr}'")),
            "{error}"
        );
    }
}

/// Closing or throwing into a generator that is mid-step is the one
/// re-entrancy its frame cannot answer, because the frame is already on the
/// stack. CPython answers the same way.
#[test]
fn a_running_generator_can_be_neither_closed_nor_thrown_into() {
    let mut repl = session();
    feed(
        &mut repl,
        "holder = [None]\ndef f():\n    holder[0].close()\n    yield 1\nholder[0] = f()",
    );
    let error = feed_err(&mut repl, "next(holder[0])");
    assert!(error.contains("generator already executing"), "{error}");

    feed(
        &mut repl,
        "box = [None]\ndef g():\n    box[0].throw(ValueError('x'))\n    yield 1\nbox[0] = g()",
    );
    let thrown = feed_err(&mut repl, "next(box[0])");
    assert!(thrown.contains("generator already executing"), "{thrown}");
}

/// `GeneratorExit` is a `BaseException`, so a broad `except Exception` cannot
/// be used to refuse a close. Asserted by catching rather than by
/// `issubclass`, which this build does not have.
#[test]
fn generator_exit_is_not_an_exception() {
    let mut repl = session();
    feed(
        &mut repl,
        "def catches_base():\n    try:\n        raise GeneratorExit()\n    except Exception:\n        return 'exception'\n    except BaseException:\n        return 'base'",
    );
    assert_eq!(feed(&mut repl, "catches_base() == 'base'"), t(true));
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

/// A generator suspended inside a `yield from` keeps the delegation across a
/// dump, so the receiver is still reachable from the waiter that comes back.
#[test]
fn a_delegating_generator_survives_dump_and_load() {
    let mut repl = session();
    feed(
        &mut repl,
        "log = []\ndef inner():\n    try:\n        yield 1\n        yield 2\n    finally:\n        log.append('closed')",
    );
    feed(&mut repl, "def outer():\n    yield from inner()\ng = outer()");
    assert_eq!(feed(&mut repl, "next(g) == 1"), t(true));

    let bytes = dump("gen.py", None, SessionRef::Idle(&repl)).unwrap();
    let mut back = match Dump::load(&bytes).unwrap().state {
        Session::Idle(repl) => *repl,
        _ => panic!("dumped an idle session, loaded something else"),
    };
    // The delegation carried across: the receiver still gives its own values,
    // and the exit still reaches it.
    assert_eq!(feed(&mut back, "next(g) == 2"), t(true));
    feed(&mut back, "g.close()");
    assert_eq!(feed(&mut back, "log == ['closed']"), t(true));
}

/// A generator expression is a generator, and it is lazy: its body runs one
/// element at a time, and the outermost iterable alone is read when the
/// expression is written.
#[test]
fn a_generator_expression_is_lazy() {
    let mut repl = session();
    assert_eq!(
        feed(&mut repl, "type(x for x in [1, 2]).__name__ == 'generator'"),
        t(true)
    );
    feed(&mut repl, "seen = []");
    feed(&mut repl, "def note(x):\n    seen.append(x)\n    return x");
    feed(&mut repl, "g = (note(x) for x in range(3))");
    assert_eq!(feed(&mut repl, "seen == []"), t(true));
    assert_eq!(feed(&mut repl, "next(g) == 0"), t(true));
    assert_eq!(feed(&mut repl, "seen == [0]"), t(true));
    assert_eq!(feed(&mut repl, "list(g) == [1, 2]"), t(true));
}

/// A generator expression stops at a call to the host the way any other
/// generator does: the frame it suspends in is saved with the rest of them.
#[test]
fn a_generator_expression_survives_a_host_call() {
    let code = "\
g = (fetch(x) for x in [1])
first = g.send(None)
first == 42 and next(g, 'end') == 'end'
";
    let call = start(code).into_function_call().expect("the host is asked");
    assert_eq!(call.function_name, "fetch");
    let done = call
        .resume(ExtFunctionResult::Return(MontyObject::int(42)), PrintWriter::Stdout)
        .unwrap();
    assert_eq!(done.into_complete(), Some(t(true)));
}

/// A delegation that spans a call to the host comes back whole. The frame that
/// waits on the receiver is saved with the rest of them, so what it waits on is
/// still known once the host answers.
#[test]
fn a_delegation_survives_a_host_call() {
    let code = "\
log = []

def inner():
    try:
        yield fetch('a')
    finally:
        log.append('receiver closed')

def outer():
    yield from inner()

g = outer()
first = g.send(None)
g.close()
first == 42 and log == ['receiver closed']
";
    let call = start(code).into_function_call().expect("the host is asked");
    assert_eq!(call.function_name, "fetch");
    let done = call
        .resume(ExtFunctionResult::Return(MontyObject::int(42)), PrintWriter::Stdout)
        .unwrap();
    assert_eq!(done.into_complete(), Some(t(true)));
}

/// Answers each call to the host, `fetch(x)`, with `x * 10`, and each lookup
/// of a name with a host function, until the run ends. With `dumping`, the run
/// goes through a dump and a load at each call. Gives the value of the last
/// line and the argument of each call, in order.
fn answer(mut progress: RunProgress, dumping: bool) -> (MontyObject, Vec<i64>) {
    let mut asked = Vec::new();
    loop {
        if dumping && matches!(progress, RunProgress::FunctionCall(_)) {
            let bytes = dump("gen.py", None, SessionRef::Running(&progress)).unwrap();
            let Session::Running(back) = Dump::load(&bytes).unwrap().state else {
                panic!("dumped a run, loaded something else")
            };
            progress = *back;
        }
        progress = match progress {
            RunProgress::NameLookup(lookup) => {
                let name = lookup.name.clone();
                lookup
                    .resume(
                        NameLookupResult::Value(MontyObject::function(name, None)),
                        PrintWriter::Stdout,
                    )
                    .unwrap()
            }
            RunProgress::FunctionCall(call) => {
                assert_eq!(call.function_name, "fetch");
                let x: i64 = call.args.arg(0).unwrap().try_into().unwrap();
                asked.push(x);
                call.resume(ExtFunctionResult::Return(MontyObject::int(x * 10)), PrintWriter::Stdout)
                    .unwrap()
            }
            done => return (done.into_complete().expect("the run completes"), asked),
        };
    }
}

/// A `for` reads a generator on the VM's own loop, as `send()` does, so a call
/// to the host in its body suspends the run, and the loop goes on when the
/// host answers. A comprehension is a `for` too.
#[test]
fn a_for_reads_a_generator_across_a_host_call() {
    let code = "\
total = 0
for v in (fetch(x) for x in [1, 2, 3]):
    total += v
total == 60 and [v for v in (fetch(x) for x in [4])] == [40]
";
    assert_eq!(answer(start(code), false), (t(true), vec![1, 2, 3, 4]));
}

/// A builtin or an instruction that reads a generator whole has it fill a list
/// on the VM's own loop first, then runs on the list, so a call to the host in
/// the body suspends the run.
#[test]
fn a_whole_read_of_a_generator_crosses_a_host_call() {
    for (code, asked) in [
        ("sum(fetch(x) for x in [1, 2]) == 30", vec![1, 2]),
        ("''.join(str(fetch(x)) for x in [1, 2]) == '1020'", vec![1, 2]),
        ("list(fetch(x) for x in [1, 2]) == [10, 20]", vec![1, 2]),
        ("tuple(fetch(x) for x in [1]) == (10,)", vec![1]),
        ("sorted(fetch(x) for x in [2, 1]) == [10, 20]", vec![2, 1]),
        ("max(fetch(x) for x in [1, 2]) == 20", vec![1, 2]),
        ("set(fetch(x) for x in [1]) == {10}", vec![1]),
        ("dict((x, fetch(x)) for x in [1]) == {1: 10}", vec![1]),
        ("[*(fetch(x) for x in [1, 2])] == [10, 20]", vec![1, 2]),
        ("a, b = (fetch(x) for x in [1, 2])\na + b == 30", vec![1, 2]),
        ("out = [0]\nout.extend(fetch(x) for x in [1])\nout == [0, 10]", vec![1]),
    ] {
        assert_eq!(answer(start(code), false), (t(true), asked), "{code}");
    }
}

/// `any()` and `all()` read a generator one value at a time on the VM's own
/// loop, and stop at the value that decides, as CPython does, so the host is
/// asked no further.
#[test]
fn any_and_all_stop_at_the_value_that_decides() {
    for (code, value, asked) in [
        ("any(fetch(x) > 10 for x in [1, 2, 3])", true, vec![1, 2]),
        ("all(fetch(x) < 20 for x in [1, 2, 3])", false, vec![1, 2]),
        ("any(fetch(x) > 90 for x in [1, 2])", false, vec![1, 2]),
    ] {
        assert_eq!(answer(start(code), false), (t(value), asked), "{code}");
    }
}

/// The `next()` builtin reads a generator as its `__next__()` does, and gives
/// its default once the generator is done.
#[test]
fn next_reads_a_generator_across_a_host_call() {
    let code = "\
g = (fetch(x) for x in [1])
next(g) == 10 and next(g, 'end') == 'end'
";
    assert_eq!(answer(start(code), false), (t(true), vec![1]));
}

/// What a generator raises while a reader steps it on the loop reaches the
/// handlers around the reader, as in CPython, and no handler in the body of
/// the loop.
#[test]
fn a_raise_in_a_read_generator_reaches_the_handler_around_the_reader() {
    let code = "\
def g():
    yield fetch(1)
    raise ValueError('no')

seen = []
try:
    for v in g():
        try:
            seen.append(v)
        except ValueError:
            seen.append('body')
except ValueError:
    seen.append('loop')
try:
    sum(g())
except ValueError:
    seen.append('sum')
seen == [10, 'loop', 'sum']
";
    assert_eq!(answer(start(code), false), (t(true), vec![1, 1]));
}

/// A read that waits on the host is a frame and the values on the stack of
/// its reader, so it survives a dump and goes on where it stood.
#[test]
fn a_read_survives_dump_and_load() {
    let code = "\
total = 0
for v in (fetch(x) for x in [1]):
    total += v
(total, sum(fetch(x) for x in [2]), any(fetch(x) > 0 for x in [3]), next(fetch(x) for x in [4])) == (10, 20, True, 40)
";
    assert_eq!(answer(start(code), true), (t(true), vec![1, 2, 3, 4]));
}
