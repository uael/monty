//! Tests for handles and `ReplFunctionCall::call_first`: a callable of the
//! sandbox that crosses to the host, and a call the host makes with it before
//! it answers a call the sandbox made to it.

use monty::{Dump, MontyRepl, ReplFunctionCall, ReplProgress, Session, SessionRef, dump};
use monty_types::{
    CallArgs, CompileOptions, ExcType, MontyException, MontyObject, MontyUuid, NamedValues, PrintWriter,
    ResourceLimits, ResourceTracker,
    unstable::{self, MontyNode},
};

const CODE: &str = "\
def echo(x):
    return ask(x)

def boom():
    raise ValueError('boom')

def deep(n):
    return n if n == 0 else deep(n - 1)

def waits():
    import asyncio
    async def body():
        return await fetch()
    return asyncio.run(body())

def adder(n):
    def add(x):
        return x + n
    return add

class Point:
    def __init__(self, x):
        self.x = ask(x)
";

fn session(limits: ResourceLimits) -> MontyRepl {
    let mut repl =
        MontyRepl::new("call_first.py", ResourceTracker::new(limits), CompileOptions::default()).with_handles();
    repl.feed_run(CODE, vec![], PrintWriter::Disabled).unwrap();
    repl
}

fn named(name: &str, value: MontyObject) -> NamedValues {
    let mut named = NamedValues::new();
    named.push(name, value);
    named
}

fn value(repl: &mut MontyRepl, code: &str) -> MontyObject {
    repl.feed_run(code, vec![], PrintWriter::Disabled).unwrap()
}

/// The call the sandbox made that the host is answering.
fn outer(repl: MontyRepl) -> ReplFunctionCall {
    let progress = repl.feed_start("outer() + 1", vec![], PrintWriter::Disabled).unwrap();
    called(progress, "outer")
}

fn called(progress: ReplProgress, name: &str) -> ReplFunctionCall {
    let ReplProgress::FunctionCall(call) = progress else {
        panic!("expected a call of the host, got {progress:?}")
    };
    assert_eq!(call.function_name, name);
    call
}

fn returned(progress: ReplProgress) -> (Result<MontyObject, MontyException>, ReplFunctionCall) {
    match progress {
        ReplProgress::Returned { result, call } => (result, call),
        other => panic!("expected the call to return, got {other:?}"),
    }
}

fn args(values: impl IntoIterator<Item = i64>) -> CallArgs {
    CallArgs::from(values.into_iter().map(MontyObject::int).collect::<Vec<_>>())
}

/// The snippet completes with what the host answered `outer` with, plus one.
fn completes(call: ReplFunctionCall, answer: i64) {
    let progress = call.resume(MontyObject::int(answer), PrintWriter::Disabled).unwrap();
    let (_, value) = progress.into_complete().expect("the snippet completes");
    assert_eq!(value, MontyObject::int(answer + 1));
}

#[test]
fn a_callable_crosses_as_the_handle_its_session_holds_it_under() {
    let mut repl = session(ResourceLimits::default());
    let echo = value(&mut repl, "echo");
    let MontyNode::Handle { id, type_name } = unstable::node(echo.as_ref()).clone() else {
        panic!("expected a handle, got {echo:?}")
    };
    assert_eq!(type_name, "function");
    assert_eq!(value(&mut repl, "echo"), echo, "the same callable keeps its handle");
    let back = repl
        .feed_run("f is echo", named("f", echo), PrintWriter::Disabled)
        .unwrap();
    assert_eq!(back, MontyObject::bool(true), "a handle comes back as the same object");
    assert!(repl.release(&id));
    assert!(!repl.release(&id), "a handle is released once");
}

#[test]
fn a_session_without_handles_gives_a_value_with_no_data_form_as_its_repr() {
    let mut repl = MontyRepl::new("call_first.py", ResourceTracker::default(), CompileOptions::default());
    repl.feed_run(CODE, vec![], PrintWriter::Disabled).unwrap();
    let echo = value(&mut repl, "echo");
    assert!(
        matches!(unstable::node(echo.as_ref()), MontyNode::Repr(_)),
        "got {echo:?}"
    );
}

#[test]
fn a_held_callable_outlives_the_code_that_made_it_until_it_is_released() {
    let mut repl = session(ResourceLimits::default());
    let add = value(&mut repl, "adder(10)");
    let MontyNode::Handle { id, .. } = unstable::node(add.as_ref()).clone() else {
        panic!("expected a handle, got {add:?}")
    };
    let got = repl
        .feed_run("f(5)", named("f", add.clone()), PrintWriter::Disabled)
        .unwrap();
    assert_eq!(got, MontyObject::int(15));
    repl.feed_run("f = None", vec![], PrintWriter::Disabled).unwrap();
    assert!(repl.release(&id));
    let error = repl.feed_run("f", named("f", add), PrintWriter::Disabled).unwrap_err();
    assert!(
        error
            .message()
            .is_some_and(|message| message.contains("is no longer held")),
        "{error:?}"
    );
}

#[test]
fn a_class_crosses_as_its_type_and_is_held() {
    let mut repl = session(ResourceLimits::default());
    let point = value(&mut repl, "Point");
    let MontyNode::ClassType(class) = unstable::node(point.as_ref()) else {
        panic!("expected a class, got {point:?}")
    };
    assert_eq!(class.name, "Point");
    let back = repl
        .feed_run("c is Point", named("c", point), PrintWriter::Disabled)
        .unwrap();
    assert_eq!(back, MontyObject::bool(true));
}

#[test]
fn a_call_of_the_host_returns_and_leaves_its_call_pending() {
    let mut repl = session(ResourceLimits::default());
    let echo = value(&mut repl, "echo");
    let call = outer(repl);
    let ask = called(call.call_first(echo, args([5]), PrintWriter::Disabled).unwrap(), "ask");
    assert_eq!(ask.args.args().collect::<Vec<_>>(), vec![MontyObject::int(5)]);
    let (result, call) = returned(ask.resume(MontyObject::int(50), PrintWriter::Disabled).unwrap());
    assert_eq!(result.unwrap(), MontyObject::int(50));
    assert_eq!(call.function_name, "outer");
    completes(call, 41);
}

#[test]
fn a_call_of_the_host_nests_in_the_answer_of_another() {
    let mut repl = session(ResourceLimits::default());
    let echo = value(&mut repl, "echo");
    let call = outer(repl);
    let ask1 = called(
        call.call_first(echo.clone(), args([1]), PrintWriter::Disabled).unwrap(),
        "ask",
    );
    let ask2 = called(ask1.call_first(echo, args([2]), PrintWriter::Disabled).unwrap(), "ask");
    assert_eq!(ask2.args.args().collect::<Vec<_>>(), vec![MontyObject::int(2)]);

    let (inner, ask1) = returned(ask2.resume(MontyObject::int(20), PrintWriter::Disabled).unwrap());
    assert_eq!(inner.unwrap(), MontyObject::int(20));
    assert_eq!(ask1.function_name, "ask");
    assert_eq!(ask1.args.args().collect::<Vec<_>>(), vec![MontyObject::int(1)]);

    let (first, call) = returned(ask1.resume(MontyObject::int(10), PrintWriter::Disabled).unwrap());
    assert_eq!(first.unwrap(), MontyObject::int(10));
    completes(call, 1);
}

#[test]
fn a_call_of_the_host_that_returns_at_once_gives_its_value() {
    let repl = session(ResourceLimits::default());
    let call = outer(repl);
    let len = MontyObject::builtin_function_from_name("len").unwrap();
    let list = CallArgs::from(vec![MontyObject::list([MontyObject::int(1), MontyObject::int(2)])]);
    let (result, call) = returned(call.call_first(len, list, PrintWriter::Disabled).unwrap());
    assert_eq!(result.unwrap(), MontyObject::int(2));
    completes(call, 3);
}

#[test]
fn a_call_of_the_host_takes_keywords() {
    let mut repl = session(ResourceLimits::default());
    let echo = value(&mut repl, "echo");
    let call = outer(repl);
    let keywords = CallArgs::from((vec![], vec![(MontyObject::string("x"), MontyObject::int(7))]));
    let ask = called(call.call_first(echo, keywords, PrintWriter::Disabled).unwrap(), "ask");
    assert_eq!(ask.args.args().collect::<Vec<_>>(), vec![MontyObject::int(7)]);
    let (result, call) = returned(ask.resume(MontyObject::int(70), PrintWriter::Disabled).unwrap());
    assert_eq!(result.unwrap(), MontyObject::int(70));
    completes(call, 0);
}

#[test]
fn a_call_of_the_host_makes_an_instance_of_a_class() {
    let mut repl = session(ResourceLimits::default());
    let point = value(&mut repl, "Point");
    let call = outer(repl);
    let ask = called(call.call_first(point, args([3]), PrintWriter::Disabled).unwrap(), "ask");
    let (result, call) = returned(ask.resume(MontyObject::int(30), PrintWriter::Disabled).unwrap());
    let made = result.unwrap();
    let pairs = made.as_ref().pairs().expect("an instance with attrs");
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].0.as_str(), Some("x"));
    assert_eq!(pairs[0].1.as_int(), Some(30));
    completes(call, 9);
}

#[test]
fn a_raise_ends_only_the_call_of_the_host() {
    let mut repl = session(ResourceLimits::default());
    let boom = value(&mut repl, "boom");
    let call = outer(repl);
    let (result, call) = returned(call.call_first(boom, CallArgs::new(), PrintWriter::Disabled).unwrap());
    let raised = result.unwrap_err();
    assert_eq!(raised.exc_type(), ExcType::ValueError);
    assert_eq!(raised.message(), Some("boom"));
    completes(call, 2);
}

#[test]
fn what_the_host_raises_in_a_call_of_the_host_ends_that_call() {
    let mut repl = session(ResourceLimits::default());
    let echo = value(&mut repl, "echo");
    let call = outer(repl);
    let ask = called(call.call_first(echo, args([1]), PrintWriter::Disabled).unwrap(), "ask");
    let refused = MontyException::new(ExcType::KeyError, Some("nothing".to_owned()));
    let (result, call) = returned(ask.resume(refused, PrintWriter::Disabled).unwrap());
    assert_eq!(result.unwrap_err().exc_type(), ExcType::KeyError);
    completes(call, 4);
}

#[test]
fn a_callable_of_the_host_is_the_hosts_to_call() {
    let repl = session(ResourceLimits::default());
    let call = outer(repl);
    let host = MontyObject::function("ext", None);
    let (result, call) = returned(call.call_first(host, CallArgs::new(), PrintWriter::Disabled).unwrap());
    assert_eq!(result.unwrap_err().exc_type(), ExcType::NotImplementedError);
    completes(call, 5);
}

#[test]
fn a_call_of_the_host_cannot_await_a_future() {
    let mut repl = session(ResourceLimits::default());
    let waits = value(&mut repl, "waits");
    let call = outer(repl);
    let fetch = called(
        call.call_first(waits, CallArgs::new(), PrintWriter::Disabled).unwrap(),
        "fetch",
    );
    let (result, call) = returned(fetch.resume_pending(PrintWriter::Disabled).unwrap());
    let raised = result.unwrap_err();
    assert_eq!(raised.exc_type(), ExcType::RuntimeError);
    assert_eq!(
        raised.message(),
        Some("a call that the host makes before it answers cannot await a future")
    );
    completes(call, 6);
}

#[test]
fn a_call_of_the_host_counts_against_the_recursion_limit() {
    let limits = ResourceLimits {
        max_recursion_depth: 40,
        ..ResourceLimits::default()
    };
    let mut repl = session(limits);
    let deep = value(&mut repl, "deep");
    let call = outer(repl);
    let (result, call) = returned(
        call.call_first(deep.clone(), args([100]), PrintWriter::Disabled)
            .unwrap(),
    );
    assert_eq!(result.unwrap_err().exc_type(), ExcType::RecursionError);
    let (result, call) = returned(call.call_first(deep, args([5]), PrintWriter::Disabled).unwrap());
    assert_eq!(result.unwrap(), MontyObject::int(0));
    completes(call, 7);
}

#[test]
fn an_uncatchable_error_in_a_call_of_the_host_ends_the_snippet() {
    let limits = ResourceLimits {
        max_memory: Some(1024 * 1024),
        ..ResourceLimits::default()
    };
    let mut repl = session(limits);
    repl.feed_run("def hog():\n    return [0] * 10_000_000", vec![], PrintWriter::Disabled)
        .unwrap();
    let hog = value(&mut repl, "hog");
    let call = outer(repl);
    let error = call
        .call_first(hog, CallArgs::new(), PrintWriter::Disabled)
        .expect_err("the snippet ends");
    assert_eq!(error.error.exc_type(), ExcType::MemoryError);
    let mut repl = error.repl;
    assert_eq!(value(&mut repl, "1 + 1"), MontyObject::int(2));
}

#[test]
fn a_session_with_a_call_of_the_host_running_dumps_and_loads() {
    let mut repl = session(ResourceLimits::default());
    let echo = value(&mut repl, "echo");
    let call = outer(repl);
    let progress = call.call_first(echo.clone(), args([3]), PrintWriter::Disabled).unwrap();
    let bytes = dump("call_first.py", None, SessionRef::Suspended(&progress)).unwrap();
    drop(progress);
    let Session::Suspended(progress) = Dump::load(&bytes).unwrap().state else {
        panic!("dumped a suspended session, loaded something else");
    };
    let ask = called(*progress, "ask");
    let (result, call) = returned(ask.resume(MontyObject::int(30), PrintWriter::Disabled).unwrap());
    assert_eq!(result.unwrap(), MontyObject::int(30));
    let (again, call) = returned(
        call.call_first(echo, args([4]), PrintWriter::Disabled)
            .map(|progress| called(progress, "ask"))
            .unwrap()
            .resume(MontyObject::int(40), PrintWriter::Disabled)
            .unwrap(),
    );
    assert_eq!(again.unwrap(), MontyObject::int(40), "the handle survives the dump");
    completes(call, 8);
}

#[test]
fn a_waiting_call_releases_a_handle() {
    let mut repl = session(ResourceLimits::default());
    let echo = value(&mut repl, "echo");
    let MontyNode::Handle { id, .. } = unstable::node(echo.as_ref()).clone() else {
        panic!("expected a handle, got {echo:?}")
    };
    let mut call = outer(repl);
    assert!(call.release(&id));
    let (result, call) = returned(call.call_first(echo, args([1]), PrintWriter::Disabled).unwrap());
    assert!(
        result
            .unwrap_err()
            .message()
            .is_some_and(|message| message.contains("is no longer held")),
        "a released handle calls nothing"
    );
    completes(call, 10);
}

const RAISES: &str = "\
class Refused(Exception):
    pass

class Never:
    def __init__(self):
        raise RuntimeError('never')
";

fn raising() -> MontyRepl {
    let mut repl = session(ResourceLimits::default());
    repl.feed_run(RAISES, vec![], PrintWriter::Disabled).unwrap();
    repl
}

fn id(of: u8) -> MontyUuid {
    MontyUuid::try_from_slice(&[of; 16]).unwrap()
}

#[test]
fn an_instance_whose_object_is_gone_is_made_again_from_its_class_and_attrs() {
    let mut repl = raising();
    let never = value(&mut repl, "Never");
    let made = MontyObject::class_instance(never, id(7), [(MontyObject::string("x"), MontyObject::int(1))]);
    let mut both = NamedValues::new();
    both.push("i", made.clone());
    both.push("j", made);
    let got = repl
        .feed_run("(i.x, type(i) is Never, i is j)", both, PrintWriter::Disabled)
        .unwrap();
    assert_eq!(
        got,
        MontyObject::tuple([MontyObject::int(1), MontyObject::bool(true), MontyObject::bool(true)]),
        "made with no __init__ run, and once under its id"
    );
}

#[test]
fn a_host_raises_an_exception_of_the_sandbox_at_the_call() {
    let mut repl = raising();
    let refused = value(&mut repl, "Refused");
    let progress = repl
        .feed_start(
            "try:\n    got = outer()\nexcept Refused as no:\n    got = ('caught', no.args)\ngot",
            vec![],
            PrintWriter::Disabled,
        )
        .unwrap();
    let call = called(progress, "outer");
    let no = MontyObject::class_instance(
        refused,
        id(8),
        [(
            MontyObject::string("args"),
            MontyObject::tuple([MontyObject::string("no")]),
        )],
    );
    let (_, got) = call.raise(no, PrintWriter::Disabled).unwrap().into_complete().unwrap();
    assert_eq!(
        got,
        MontyObject::tuple([
            MontyObject::string("caught"),
            MontyObject::tuple([MontyObject::string("no")])
        ])
    );
}

#[test]
fn an_exception_the_host_raises_that_nothing_catches_ends_the_snippet_under_its_class() {
    let mut repl = raising();
    let refused = value(&mut repl, "Refused");
    let call = outer(repl);
    let no = MontyObject::class_instance(
        refused,
        id(9),
        [(
            MontyObject::string("args"),
            MontyObject::tuple([MontyObject::string("no")]),
        )],
    );
    let error = call.raise(no, PrintWriter::Disabled).expect_err("nothing catches it");
    assert_eq!(error.error.type_name(), "Refused");
    assert_eq!(error.error.message(), Some("no"));
}

#[test]
fn a_host_that_raises_no_exception_raises_a_type_error() {
    let repl = raising();
    let progress = repl
        .feed_start(
            "try:\n    got = outer()\nexcept TypeError as no:\n    got = str(no)\ngot",
            vec![],
            PrintWriter::Disabled,
        )
        .unwrap();
    let call = called(progress, "outer");
    let (_, got) = call
        .raise(MontyObject::int(3), PrintWriter::Disabled)
        .unwrap()
        .into_complete()
        .unwrap();
    assert_eq!(got, MontyObject::string("exceptions must derive from BaseException"));
}

#[test]
fn a_value_with_no_data_form_crosses_as_a_handle_and_back_as_itself() {
    let mut repl = session(ResourceLimits::default());
    repl.feed_run(
        "def counts():\n    yield 1\n    yield 2\ng = counts()",
        vec![],
        PrintWriter::Disabled,
    )
    .unwrap();
    let generator = value(&mut repl, "g");
    assert_eq!(generator.as_ref().type_name(), "generator");
    assert!(generator.as_ref().handle().is_some());
    let got = repl
        .feed_run("(next(h), h is g)", named("h", generator), PrintWriter::Disabled)
        .unwrap();
    assert_eq!(got, MontyObject::tuple([MontyObject::int(1), MontyObject::bool(true)]));
}

#[test]
fn a_host_builds_a_template_from_its_parts_by_handles() {
    let mut repl = session(ResourceLimits::default());
    repl.feed_run(
        "from string.templatelib import Interpolation, Template\ndef shown(t):\n    return [(i.value, i.expression) for i in t.interpolations]",
        vec![],
        PrintWriter::Disabled,
    )
    .unwrap();
    let (interpolation, template, shown) = (
        value(&mut repl, "Interpolation"),
        value(&mut repl, "Template"),
        value(&mut repl, "shown"),
    );
    let part = repl
        .feed_run("i(42, 'x')", named("i", interpolation), PrintWriter::Disabled)
        .unwrap();
    assert_eq!(part.as_ref().type_name(), "string.templatelib.Interpolation");
    let mut parts = named("t", template);
    parts.push("p", part);
    let built = repl.feed_run("t('a ', p)", parts, PrintWriter::Disabled).unwrap();
    let mut given = named("s", shown);
    given.push("b", built);
    let got = repl.feed_run("s(b)", given, PrintWriter::Disabled).unwrap();
    assert_eq!(
        got,
        MontyObject::list([MontyObject::tuple([MontyObject::int(42), MontyObject::string("x")])])
    );
}
