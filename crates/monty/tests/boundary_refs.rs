//! The boundary carrying references rather than copies, in both directions:
//! a host object the sandbox holds ([`MontyObject::HostRef`]), a session value
//! the host holds ([`MontyObject::SessionRef`]), and reading a session while a
//! snippet is suspended inside a call it made.

use monty::{Dump, MontyRepl, Session, SessionRef, dump};
use monty_types::{CompileOptions, ExtFunctionResult, MontyObject, PrintWriter, ResourceTracker};

fn new_repl() -> MontyRepl {
    MontyRepl::new("refs.py", ResourceTracker::default(), CompileOptions::default())
}

fn feed(repl: &mut MontyRepl, code: &str) -> MontyObject {
    repl.feed_run(code, vec![], PrintWriter::Stdout).unwrap().value
}

fn feed_with(repl: &mut MontyRepl, code: &str, inputs: Vec<(String, MontyObject)>) -> MontyObject {
    repl.feed_run(code, inputs, PrintWriter::Stdout).unwrap().value
}

fn host_ref(id: u64, type_name: &str) -> MontyObject {
    MontyObject::HostRef {
        id,
        type_name: type_name.to_owned(),
    }
}

/// Runs `code` and hands back the one suspension it takes, so a test can look
/// at what the sandbox asked for.
fn suspend_on(repl: MontyRepl, code: &str, inputs: Vec<(String, MontyObject)>) -> monty::ReplFunctionCall {
    repl.feed_start(code, inputs, PrintWriter::Stdout)
        .expect("the snippet suspends rather than failing")
        .into_function_call()
        .expect("an operation on a host reference suspends as a call")
}

/// Answers a suspension with `None` and runs the snippet out.
///
/// Every test that suspends ends through here: abandoning a suspension whose
/// globals hold a heap value trips the reference-counting assertion under
/// `memory-model-checks`, and that is true of any suspension, not only these.
fn finish(call: monty::ReplFunctionCall) {
    let mut progress = call
        .resume(ExtFunctionResult::Return(MontyObject::None), PrintWriter::Stdout)
        .expect("the snippet runs out after its call is answered");
    while let Some(next) = progress.into_function_call() {
        progress = next
            .resume(ExtFunctionResult::Return(MontyObject::None), PrintWriter::Stdout)
            .expect("the snippet runs out after its call is answered");
    }
}

// ---------------------------------------------------------------------------
// A host object the sandbox holds
// ---------------------------------------------------------------------------

/// Reading an attribute of a host object is a call to the host, carrying the
/// reference and the name, and nothing else has to be declared for it.
#[test]
fn host_reference_reads_an_attribute_on_the_host() {
    let call = suspend_on(new_repl(), "c.cwd", vec![("c".to_owned(), host_ref(7, "Cursor"))]);

    assert_eq!(call.function_name, "__getattr__");
    assert!(call.method_call);
    assert_eq!(
        call.args,
        vec![host_ref(7, "Cursor"), MontyObject::String("cwd".to_owned())]
    );

    let progress = call
        .resume(
            ExtFunctionResult::Return(MontyObject::String("/tmp".to_owned())),
            PrintWriter::Stdout,
        )
        .unwrap();
    let (_, outcome) = progress.into_complete().unwrap();
    assert_eq!(outcome.value, MontyObject::String("/tmp".to_owned()));
}

/// A method call on a host object goes out under its own name with the
/// reference first, so one round trip serves `obj.method(args)` rather than
/// two.
#[test]
fn host_reference_calls_a_method_in_one_round_trip() {
    let call = suspend_on(
        new_repl(),
        "c.bash('ls', timeout=5)",
        vec![("c".to_owned(), host_ref(7, "Cursor"))],
    );

    assert_eq!(call.function_name, "bash");
    assert_eq!(
        call.args,
        vec![host_ref(7, "Cursor"), MontyObject::String("ls".to_owned())]
    );
    assert_eq!(
        call.kwargs,
        vec![(MontyObject::String("timeout".to_owned()), MontyObject::Int(5))]
    );
    finish(call);
}

/// Calling the reference itself is the host's own `__call__`.
#[test]
fn host_reference_is_callable() {
    let call = suspend_on(new_repl(), "f(1)", vec![("f".to_owned(), host_ref(3, "Adder"))]);
    assert_eq!(call.function_name, "__call__");
    assert_eq!(call.args, vec![host_ref(3, "Adder"), MontyObject::Int(1)]);
    finish(call);
}

/// A host object is a context manager, since only the host knows whether it is
/// one; `with` sends both halves out.
#[test]
fn host_reference_enters_and_leaves_as_a_context_manager() {
    let call = suspend_on(
        new_repl(),
        "with c as inner:\n    pass",
        vec![("c".to_owned(), host_ref(11, "Back"))],
    );
    assert_eq!(call.function_name, "__enter__");
    assert_eq!(call.args, vec![host_ref(11, "Back")]);

    let call = call
        .resume(ExtFunctionResult::Return(MontyObject::Int(1)), PrintWriter::Stdout)
        .unwrap()
        .into_function_call()
        .expect("leaving the block asks the host too");
    assert_eq!(call.function_name, "__exit__");
    assert_eq!(call.args, vec![host_ref(11, "Back"), MontyObject::None]);
    finish(call);
}

/// The sandbox cannot name a dunder on a host object, so it cannot reach
/// `__class__` and walk out through the type graph. The VM's own operations
/// still send theirs.
#[test]
fn host_reference_refuses_a_dunder_the_sandbox_names() {
    let mut repl = new_repl();
    let err = repl
        .feed_run(
            "c.__class__",
            vec![("c".to_owned(), host_ref(7, "Cursor"))],
            PrintWriter::Stdout,
        )
        .unwrap_err();
    assert_eq!(
        err.summary(),
        "AttributeError: 'Cursor' object has no attribute '__class__'"
    );

    let err = repl
        .feed_run(
            "c.__getattr__('x')",
            vec![("c".to_owned(), host_ref(7, "Cursor"))],
            PrintWriter::Stdout,
        )
        .unwrap_err();
    assert_eq!(
        err.summary(),
        "AttributeError: 'Cursor' object has no attribute '__getattr__'"
    );
}

/// A reference read back out is the same reference, so a host that hands one
/// in recognises it by identity rather than by an attribute it planted.
#[test]
fn host_reference_crosses_back_as_itself() {
    let mut repl = new_repl();
    feed_with(&mut repl, "held = c", vec![("c".to_owned(), host_ref(7, "Cursor"))]);
    assert_eq!(feed(&mut repl, "held"), host_ref(7, "Cursor"));
    assert_eq!(
        feed(&mut repl, "repr(held)"),
        MontyObject::String("<Cursor host object>".to_owned())
    );
    assert_eq!(
        feed(&mut repl, "type(held).__name__"),
        MontyObject::String("hostref".to_owned())
    );
}

/// Two references to one host object are one object inside the sandbox, and
/// two references to different objects are not, so a set of them behaves.
#[test]
fn host_references_are_equal_exactly_when_they_name_one_object() {
    let mut repl = new_repl();
    let inputs = vec![
        ("a".to_owned(), host_ref(7, "Cursor")),
        ("b".to_owned(), host_ref(7, "Cursor")),
        ("d".to_owned(), host_ref(8, "Cursor")),
    ];
    assert_eq!(
        feed_with(&mut repl, "(a == b, a == d, len({a, b, d}))", inputs),
        MontyObject::Tuple(vec![
            MontyObject::Bool(true),
            MontyObject::Bool(false),
            MontyObject::Int(2)
        ])
    );
}

/// A reference survives the session being dumped and woken: the proxy is heap
/// state like any other, and the host's id is what it carries.
#[test]
fn host_reference_survives_dump_and_load() {
    let mut repl = new_repl();
    feed_with(&mut repl, "held = c", vec![("c".to_owned(), host_ref(7, "Cursor"))]);
    let bytes = dump("refs.py", None, SessionRef::Idle(&repl)).unwrap();
    drop(repl);
    let Session::Idle(mut woken) = Dump::load(&bytes).unwrap().state else {
        panic!("dumped an idle session, loaded something else");
    };
    assert_eq!(feed(&mut woken, "held"), host_ref(7, "Cursor"));
}

// ---------------------------------------------------------------------------
// A session value the host holds
// ---------------------------------------------------------------------------

/// A value with no copy representation crosses as its `repr` by default, and
/// as a reference once the session is asked for one.
#[test]
fn a_type_crosses_as_text_until_the_session_is_asked_for_a_reference() {
    let mut repl = new_repl();
    feed(&mut repl, "class Chunk:\n    pass");
    assert_eq!(
        feed(&mut repl, "list[Chunk]"),
        MontyObject::Repr("list[Chunk]".to_owned())
    );

    repl.set_cross_by_reference(true);
    let MontyObject::SessionRef { id, repr } = feed(&mut repl, "list[Chunk]") else {
        panic!("a type crosses as a reference in this mode");
    };
    assert_eq!(repr, "list[Chunk]");

    // and the host can ask it what it is made of, in the session's own terms
    let held = MontyObject::SessionRef { id, repr };
    assert_eq!(
        repl.probe_scoped(
            "_t.__origin__.__name__",
            vec![("_t".to_owned(), held.clone())],
            PrintWriter::Stdout
        )
        .unwrap(),
        MontyObject::String("list".to_owned())
    );
    assert_eq!(
        repl.probe_scoped(
            "_t.__args__[0].__name__",
            vec![("_t".to_owned(), held)],
            PrintWriter::Stdout
        )
        .unwrap(),
        MontyObject::String("Chunk".to_owned())
    );
    assert!(repl.release(id));
}

/// A class object crosses as a reference too, and handing it back into the
/// session yields the class itself rather than a copy of it.
#[test]
fn a_class_crosses_by_reference_and_returns_as_itself() {
    let mut repl = new_repl();
    repl.set_cross_by_reference(true);
    feed(&mut repl, "class Chunk:\n    kind = 'chunk'");
    let held = feed(&mut repl, "Chunk");
    let MontyObject::SessionRef { id, .. } = held else {
        panic!("a class crosses as a reference in this mode");
    };

    assert_eq!(
        repl.probe_scoped(
            "_c is Chunk",
            vec![("_c".to_owned(), held.clone())],
            PrintWriter::Stdout
        )
        .unwrap(),
        MontyObject::Bool(true)
    );
    assert_eq!(
        repl.probe_scoped("_c().kind", vec![("_c".to_owned(), held)], PrintWriter::Stdout)
            .unwrap(),
        MontyObject::String("chunk".to_owned())
    );
    assert!(repl.release(id));
}

/// The name a value was exported under is not what keeps it: rebinding the
/// name leaves the reference pointing at what it named.
#[test]
fn an_exported_value_outlives_the_name_it_was_exported_under() {
    let mut repl = new_repl();
    feed(&mut repl, "held = [1, 2, 3]");
    let held = repl.export_global("held", PrintWriter::Stdout).unwrap();
    let MontyObject::SessionRef { id, ref repr } = held else {
        panic!("a list bound to a name exports as a reference");
    };
    assert_eq!(repr, "[1, 2, 3]");

    feed(&mut repl, "held = None");
    assert_eq!(
        repl.probe_scoped("sum(_h)", vec![("_h".to_owned(), held)], PrintWriter::Stdout)
            .unwrap(),
        MontyObject::Int(6)
    );
    assert!(repl.release(id));
    // released once, so releasing again is reported rather than taking a
    // reference that is no longer the host's
    assert!(!repl.release(id));
}

/// A reference names a place in the heap, and a dump carries the heap, so a
/// contract exported before a session is put away is still readable after it
/// wakes.
#[test]
fn a_session_reference_survives_dump_and_load() {
    let mut repl = new_repl();
    repl.set_cross_by_reference(true);
    feed(&mut repl, "class Chunk:\n    pass\ncontract = list[Chunk]");
    let held = repl.export_global("contract", PrintWriter::Stdout).unwrap();
    let MontyObject::SessionRef { id, .. } = held else {
        panic!("a type exports as a reference");
    };

    let bytes = dump("refs.py", None, SessionRef::Idle(&repl)).unwrap();
    drop(repl);
    let Session::Idle(mut woken) = Dump::load(&bytes).unwrap().state else {
        panic!("dumped an idle session, loaded something else");
    };
    assert_eq!(
        woken
            .probe_scoped(
                "_t.__args__[0].__name__",
                vec![("_t".to_owned(), held)],
                PrintWriter::Stdout
            )
            .unwrap(),
        MontyObject::String("Chunk".to_owned())
    );
    // the mode travelled with the dump too
    assert!(woken.cross_by_reference());
    assert!(woken.release(id));
}

/// A token this session never minted names nothing, whatever integer it is:
/// the export table is the only thing that resolves one.
#[test]
fn an_invented_token_resolves_to_nothing() {
    let mut repl = new_repl();
    feed(&mut repl, "decoy = [1, 2, 3]");
    let invented = MontyObject::SessionRef {
        id: 3,
        repr: "whatever".to_owned(),
    };
    let err = repl
        .feed_run("x = _t", vec![("_t".to_owned(), invented)], PrintWriter::Stdout)
        .unwrap_err();
    assert!(
        err.summary()
            .contains("names an export this session has released or never made"),
        "got: {}",
        err.summary()
    );
}

// ---------------------------------------------------------------------------
// Reading a session while it is suspended
// ---------------------------------------------------------------------------

/// The host can look at the frame that is asking, and answer by what it finds
/// there. The rung stays resumable.
#[test]
fn a_suspended_session_answers_a_probe() {
    let repl = new_repl();
    let mut call = repl
        .feed_start(
            "class Chunk:\n    pass\nwanted = list[Chunk]\nresult = decide(1)\nresult",
            vec![],
            PrintWriter::Stdout,
        )
        .unwrap()
        .into_function_call()
        .expect("`decide` is nobody's name here, so it goes out as a call");
    assert_eq!(call.function_name, "decide");

    // the names the rung bound before it suspended are readable
    assert_eq!(
        call.probe("wanted.__args__[0].__name__", vec![], PrintWriter::Stdout)
            .unwrap(),
        MontyObject::String("Chunk".to_owned())
    );

    // and the rung is untouched: it resumes with what the host decided
    let (_, outcome) = call
        .resume(ExtFunctionResult::Return(MontyObject::Int(42)), PrintWriter::Stdout)
        .unwrap()
        .into_complete()
        .unwrap();
    assert_eq!(outcome.value, MontyObject::Int(42));
}

/// A probe's bindings are its own: the session does not come away knowing a
/// name the host supplied for one expression.
#[test]
fn a_probe_binding_does_not_become_a_name_the_session_has() {
    let mut repl = new_repl();
    assert_eq!(
        repl.probe_scoped(
            "supplied * 2",
            vec![("supplied".to_owned(), MontyObject::Int(21))],
            PrintWriter::Stdout
        )
        .unwrap(),
        MontyObject::Int(42)
    );
    let err = repl.feed_run("supplied", vec![], PrintWriter::Stdout).unwrap_err();
    assert_eq!(err.summary(), "NameError: name 'supplied' is not defined");
}

/// A probe from inside a suspension runs to completion: it cannot suspend
/// again, so a name nothing supplies is a `NameError` rather than a second
/// call the host has nowhere to answer.
#[test]
fn a_suspended_probe_raises_rather_than_suspending_again() {
    let repl = new_repl();
    let mut call = repl
        .feed_start("result = decide(1)\nresult", vec![], PrintWriter::Stdout)
        .unwrap()
        .into_function_call()
        .unwrap();

    let err = call
        .probe("nobody_supplies_this", vec![], PrintWriter::Stdout)
        .unwrap_err();
    assert_eq!(err.summary(), "NameError: name 'nobody_supplies_this' is not defined");

    // the suspension survived the failure
    let (_, outcome) = call
        .resume(ExtFunctionResult::Return(MontyObject::Int(1)), PrintWriter::Stdout)
        .unwrap()
        .into_complete()
        .unwrap();
    assert_eq!(outcome.value, MontyObject::Int(1));
}

/// The whole shape the driver needs, end to end: sandbox code calls a host
/// object with a contract the session defined, and the host reads that
/// contract while the call is still open.
#[test]
fn a_host_call_carries_a_contract_the_host_reads_before_answering() {
    let mut repl = new_repl();
    repl.set_cross_by_reference(true);
    let mut call = repl
        .feed_start(
            "class Chunk:\n    pass\nc.fill(list[Chunk], 'go')",
            vec![("c".to_owned(), host_ref(7, "Cursor"))],
            PrintWriter::Stdout,
        )
        .unwrap()
        .into_function_call()
        .expect("a method call on a host object suspends");

    assert_eq!(call.function_name, "fill");
    let [receiver, contract, prompt] = call.args.as_slice() else {
        panic!("fill was called with the reference, the contract and the prompt");
    };
    assert_eq!(*receiver, host_ref(7, "Cursor"));
    assert_eq!(*prompt, MontyObject::String("go".to_owned()));
    let MontyObject::SessionRef { id, ref repr } = *contract else {
        panic!("the contract crossed as a reference, not as text: {contract:?}");
    };
    assert_eq!(repr, "list[Chunk]");

    // the host reads the contract's structure before deciding what to answer
    let held = MontyObject::SessionRef { id, repr: repr.clone() };
    assert_eq!(
        call.probe(
            "_t.__args__[0].__name__",
            vec![("_t".to_owned(), held)],
            PrintWriter::Stdout
        )
        .unwrap(),
        MontyObject::String("Chunk".to_owned())
    );

    let (mut repl, outcome) = call
        .resume(
            ExtFunctionResult::Return(MontyObject::String("done".to_owned())),
            PrintWriter::Stdout,
        )
        .unwrap()
        .into_complete()
        .unwrap();
    assert_eq!(outcome.value, MontyObject::String("done".to_owned()));
    assert!(repl.release(id));
}
