//! Generator behaviour that a `test_cases` fixture cannot express: suspending
//! a generator body to the host, and dumping one mid-flight.
//!
//! Both hinge on the same design point. A generator step runs on the VM's own
//! frame stack, so a `for` loop can suspend inside a generator body and the
//! ordinary session dump captures it; a Rust-side consumer (`list`, `sum`,
//! `next`, ...) holds a Rust frame across the step instead and cannot.

use monty::{Dump, MontyRun, RunProgress, Session, SessionRef, dump};
use monty_types::{
    CompileOptions, ExcType, MontyException, MontyObject, NameLookupResult, PrintWriter, ResourceTracker,
};

/// Compiles `source` and runs it to its first suspension.
fn start(source: &str) -> RunProgress {
    let runner = MontyRun::new(source.to_owned(), "test.py", vec![], CompileOptions::default()).unwrap();
    let progress = runner
        .start(vec![], ResourceTracker::default(), PrintWriter::Stdout)
        .unwrap();
    resolve_name_lookups(progress).unwrap()
}

/// Answers the `NameLookup` yields that an undeclared `ext_fn` produces, so the
/// run reaches the external call itself.
fn resolve_name_lookups(mut progress: RunProgress) -> Result<RunProgress, MontyException> {
    while let RunProgress::NameLookup(lookup) = progress {
        let name = lookup.name.clone();
        progress = lookup.resume(
            NameLookupResult::Value(MontyObject::Function { name, docstring: None }),
            PrintWriter::Stdout,
        )?;
    }
    Ok(progress)
}

/// Round-trips a paused run through the real dump format and resumes *both*
/// copies with `answer`, asserting they finish alike.
///
/// Both are resumed because an unresumed `RunProgress` leaves its globals' refs
/// unreleased and aborts under `memory-model-checks`; that is pre-existing and
/// not generator-specific (see `binary_serde.rs`).
fn dump_and_resume_both(progress: RunProgress, answer: MontyObject) -> MontyObject {
    let bytes = dump("test.py", None, SessionRef::Running(&progress)).unwrap();
    let loaded = match Dump::load(&bytes).unwrap().state {
        Session::Running(loaded) => *loaded,
        _ => panic!("dumped a running session, loaded something else"),
    };

    let original = progress.into_function_call().expect("should be at the external call");
    let from_original = original
        .resume(answer.clone(), PrintWriter::Stdout)
        .unwrap()
        .into_complete()
        .unwrap();

    let call = loaded
        .into_function_call()
        .expect("the loaded run should still be at the call");
    let from_loaded = call
        .resume(answer, PrintWriter::Stdout)
        .unwrap()
        .into_complete()
        .unwrap();

    assert_eq!(
        from_original, from_loaded,
        "a loaded run must continue like the original"
    );
    from_loaded
}

#[test]
fn for_loop_suspends_inside_a_generator_body() {
    // The `for` loop steps the generator on the VM's own frame stack, so the
    // external call inside the body reaches the host like any other.
    let progress =
        start("def gen():\n    yield ext_fn(1)\n    yield 2\n\ntotal = 0\nfor v in gen():\n    total += v\ntotal");

    let call = progress.into_function_call().expect("should reach the external call");
    assert_eq!(call.function_name, "ext_fn");
    assert_eq!(call.args, vec![MontyObject::Int(1)]);

    let result = call.resume(MontyObject::Int(10), PrintWriter::Stdout).unwrap();
    assert_eq!(result.into_complete().unwrap(), MontyObject::Int(12));
}

#[test]
fn yield_from_suspends_inside_a_delegate() {
    // `yield from` steps its delegate the same way, so a suspension two
    // generators deep still reaches the host.
    let progress = start(
        "def inner():\n    yield ext_fn(5)\n\ndef outer():\n    yield from inner()\n\ntotal = 0\nfor v in outer():\n    total += v\ntotal",
    );

    let call = progress.into_function_call().expect("should reach the external call");
    assert_eq!(call.args, vec![MontyObject::Int(5)]);

    let result = call.resume(MontyObject::Int(7), PrintWriter::Stdout).unwrap();
    assert_eq!(result.into_complete().unwrap(), MontyObject::Int(7));
}

#[test]
fn a_rust_side_consumer_cannot_suspend_a_generator() {
    // `list()` holds a Rust frame across the step, so the same body that works
    // under a `for` loop reports the gap loudly rather than losing the call.
    let runner = MontyRun::new(
        "def gen():\n    yield ext_fn(1)\n\nlist(gen())".to_owned(),
        "test.py",
        vec![],
        CompileOptions::default(),
    )
    .unwrap();
    // The call never reaches the host at all: the nested run cannot preserve
    // the suspension, so it fails the whole start.
    let error = runner
        .start(vec![], ResourceTracker::default(), PrintWriter::Stdout)
        .expect_err("expected the suspension to be rejected");
    assert_eq!(error.exc_type(), ExcType::NotImplementedError);
    assert_eq!(
        error.message().unwrap(),
        "generator: external function 'ext_fn' is not yet supported in this context"
    );
}

#[test]
fn a_generator_suspended_mid_step_survives_a_dump() {
    // The generator's frame and the activation that records who is driving it
    // are both live on the VM when the dump happens.
    let progress = start(
        "def gen():\n    a = ext_fn(1)\n    yield a\n    yield a * 2\n\ntotal = 0\nfor v in gen():\n    total += v\ntotal",
    );

    let result = dump_and_resume_both(progress, MontyObject::Int(3));
    assert_eq!(result, MontyObject::Int(9));
}

#[test]
fn a_generator_paused_at_a_yield_survives_a_dump() {
    // Here the generator is not running: its frame lives in the heap object,
    // and the dump has to carry that rather than any activation.
    let progress = start(
        "def gen():\n    yield 1\n    yield 2\n\ng = gen()\nfirst = next(g)\nextra = ext_fn(0)\nfirst + next(g) + extra",
    );

    let result = dump_and_resume_both(progress, MontyObject::Int(40));
    assert_eq!(result, MontyObject::Int(43));
}

#[test]
fn a_generator_expression_survives_a_dump() {
    // A generator expression is an ordinary generator, so it dumps like one.
    let progress = start("values = [1, 2, 3]\ng = (v * 2 for v in values)\nextra = ext_fn(0)\nsum(g) + extra");

    let result = dump_and_resume_both(progress, MontyObject::Int(1));
    assert_eq!(result, MontyObject::Int(13));
}

#[test]
fn an_async_generator_suspends_to_the_host() {
    // `async for` drives the generator through `Await`, which is also a
    // yield-capable path, so the body can reach the host.
    let progress = start(
        "import asyncio\n\nasync def ticks():\n    yield ext_fn(1)\n    yield 2\n\nasync def main():\n    total = 0\n    async for v in ticks():\n        total += v\n    return total\n\nasyncio.run(main())",
    );

    let call = progress.into_function_call().expect("should reach the external call");
    assert_eq!(call.args, vec![MontyObject::Int(1)]);

    let result = call.resume(MontyObject::Int(5), PrintWriter::Stdout).unwrap();
    assert_eq!(result.into_complete().unwrap(), MontyObject::Int(7));
}

#[test]
fn a_starred_generator_expression_suspends() {
    // `f(*(...))` drains the generator expression before the call, and drains it
    // on the VM's own frame stack, so a host call inside the body reaches the
    // host instead of reporting the Rust-side-consumer gap. This is the shape
    // `asyncio.gather(*(coro(x) for x in xs))` takes.
    let progress = start("def f(*args):\n    return args\n\nf(*(ext_fn(i) for i in [1]))");

    let call = progress.into_function_call().expect("should reach the external call");
    assert_eq!(call.function_name, "ext_fn");
    assert_eq!(call.args, vec![MontyObject::Int(1)]);

    let result = call.resume(MontyObject::Int(4), PrintWriter::Stdout).unwrap();
    assert_eq!(
        result.into_complete().unwrap(),
        MontyObject::Tuple(vec![MontyObject::Int(4)])
    );
}

#[test]
fn a_starred_generator_expression_in_a_list_suspends() {
    // The same lowering serves a list or tuple display, where the star operand
    // is drained into the collection under construction.
    let progress = start("[0, *(ext_fn(i) for i in [2])]");

    let call = progress.into_function_call().expect("should reach the external call");
    assert_eq!(call.args, vec![MontyObject::Int(2)]);

    let result = call.resume(MontyObject::Int(5), PrintWriter::Stdout).unwrap();
    assert_eq!(
        result.into_complete().unwrap(),
        MontyObject::List(vec![MontyObject::Int(0), MontyObject::Int(5)])
    );
}
