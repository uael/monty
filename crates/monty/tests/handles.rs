//! Tests for handles: a value of the sandbox with no data form, or a class,
//! that a session holds for its host and that comes back as the same object.

use monty::{Dump, MontyRepl, ReplProgress, Session, SessionRef, dump};
use monty_types::{
    CompileOptions, MontyObject, NamedValues, PrintWriter, ResourceTracker,
    unstable::{self, MontyNode},
};

const CODE: &str = "\
def echo(x):
    return x

def adder(n):
    def add(x):
        return x + n
    return add

class Point:
    def __init__(self, x):
        self.x = x

    def moved(self, by):
        return self.x + by
";

fn session() -> MontyRepl {
    let mut repl = MontyRepl::new("handles.py", ResourceTracker::default(), CompileOptions::default()).with_handles();
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

fn handle(object: &MontyObject) -> (monty_types::MontyUuid, String) {
    let MontyNode::Handle { id, type_name } = unstable::node(object.as_ref()).clone() else {
        panic!("expected a handle, got {object:?}")
    };
    (id, type_name)
}

fn no_longer_held(repl: &mut MontyRepl, object: MontyObject) {
    let error = repl
        .feed_run("f", named("f", object), PrintWriter::Disabled)
        .unwrap_err();
    assert!(
        error
            .message()
            .is_some_and(|message| message.contains("is no longer held")),
        "{error:?}"
    );
}

#[test]
fn a_function_crosses_as_the_handle_its_session_holds_it_under() {
    let mut repl = session();
    let echo = value(&mut repl, "echo");
    let (id, type_name) = handle(&echo);
    assert_eq!(type_name, "function");
    assert_eq!(echo.as_ref().handle(), Some(id));
    assert_eq!(value(&mut repl, "echo"), echo, "the same function keeps its handle");
    let back = repl
        .feed_run("(f is echo, f(3))", named("f", echo), PrintWriter::Disabled)
        .unwrap();
    assert_eq!(
        back,
        MontyObject::tuple([MontyObject::bool(true), MontyObject::int(3)]),
        "a handle comes back as the same object"
    );
    assert!(repl.release(&id));
    assert!(!repl.release(&id), "a handle is released once");
}

#[test]
fn a_session_without_handles_gives_a_value_with_no_data_form_as_its_repr() {
    let mut repl = MontyRepl::new("handles.py", ResourceTracker::default(), CompileOptions::default());
    repl.feed_run(CODE, vec![], PrintWriter::Disabled).unwrap();
    let echo = value(&mut repl, "echo");
    assert!(
        matches!(unstable::node(echo.as_ref()), MontyNode::Repr(_)),
        "got {echo:?}"
    );
}

#[test]
fn a_held_value_outlives_the_code_that_made_it_until_it_is_released() {
    let mut repl = session();
    let add = value(&mut repl, "adder(10)");
    let (id, _) = handle(&add);
    let got = repl
        .feed_run("f(5)", named("f", add.clone()), PrintWriter::Disabled)
        .unwrap();
    assert_eq!(got, MontyObject::int(15));
    assert!(repl.release(&id));
    no_longer_held(&mut repl, add);
}

#[test]
fn an_iterator_a_bound_method_and_a_coroutine_cross_as_handles_and_back_as_themselves() {
    let mut repl = session();
    repl.feed_run(
        "it = iter([1, 2])\npt = Point(4)\nasync def later():\n    return 3\nco = later()",
        vec![],
        PrintWriter::Disabled,
    )
    .unwrap();
    let iterator = value(&mut repl, "it");
    assert_eq!(handle(&iterator).1, "list_iterator");
    let moved = value(&mut repl, "pt.moved");
    let named_type = value(&mut repl, "type(pt.moved).__name__");
    assert_eq!(
        Some(handle(&moved).1.as_str()),
        named_type.as_ref().as_str(),
        "a handle names the type that the sandbox names"
    );
    let coroutine = value(&mut repl, "co");
    assert_eq!(handle(&coroutine).1, "coroutine");
    let mut given = named("h", iterator);
    given.push("m", moved);
    given.push("c", coroutine);
    let got = repl
        .feed_run(
            "(next(h), h is it, m(1), c is co, await c)",
            given,
            PrintWriter::Disabled,
        )
        .unwrap();
    assert_eq!(
        got,
        MontyObject::tuple([
            MontyObject::int(1),
            MontyObject::bool(true),
            MontyObject::int(5),
            MontyObject::bool(true),
            MontyObject::int(3)
        ])
    );
}

#[test]
fn a_class_crosses_as_its_type_and_is_held() {
    let mut repl = session();
    let point = value(&mut repl, "Point");
    let MontyNode::ClassType(class) = unstable::node(point.as_ref()) else {
        panic!("expected a class, got {point:?}")
    };
    assert_eq!(class.name, "Point");
    let id = point.as_ref().handle().expect("a class of the sandbox has a handle");
    repl.feed_run("Point = None", vec![], PrintWriter::Disabled).unwrap();
    let back = repl
        .feed_run("c(2).x", named("c", point.clone()), PrintWriter::Disabled)
        .unwrap();
    assert_eq!(
        back,
        MontyObject::int(2),
        "the session holds the class its name let go of"
    );
    assert!(repl.release(&id));
}

#[test]
fn a_held_value_survives_a_dump_and_a_load() {
    let mut repl = session();
    let add = value(&mut repl, "adder(1)");
    repl.feed_run("adder = None", vec![], PrintWriter::Disabled).unwrap();
    let bytes = dump("handles.py", None, &[], SessionRef::Idle(&repl)).unwrap();
    drop(repl);
    let Session::Idle(mut repl) = Dump::load(&bytes).unwrap().state else {
        panic!("dumped an idle session, loaded something else");
    };
    let got = repl.feed_run("f(1)", named("f", add), PrintWriter::Disabled).unwrap();
    assert_eq!(got, MontyObject::int(2));
}

#[test]
fn a_waiting_call_releases_a_handle() {
    let mut repl = session();
    let echo = value(&mut repl, "echo");
    let (id, _) = handle(&echo);
    let progress = repl.feed_start("outer() + 1", vec![], PrintWriter::Disabled).unwrap();
    let ReplProgress::FunctionCall(mut call) = progress else {
        panic!("expected a call of the host, got {progress:?}")
    };
    assert!(call.release(&id));
    let (mut repl, got) = call
        .resume(MontyObject::int(1), PrintWriter::Disabled)
        .unwrap()
        .into_complete()
        .expect("the snippet completes");
    assert_eq!(got, MontyObject::int(2));
    no_longer_held(&mut repl, echo);
}
