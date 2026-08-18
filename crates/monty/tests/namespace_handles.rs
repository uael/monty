//! Namespaces held as values by the code inside.
//!
//! `namespace()` mints one, `namespace(source)` copies one, and the handle
//! reads and writes the same slot vector the namespace's own code resolves
//! globals against. These tests prove the binds are real by mutating through
//! the handle and reading through the object, never by comparing values that
//! merely look alike; and they prove release by watching a released slot be
//! reused, never by peeking at internals.

use monty::{Dump, MontyRepl, Session, SessionRef, dump};
use monty_types::{CompileOptions, MontyObject, PrintWriter, ResourceTracker};

fn session() -> MontyRepl {
    let mut repl = MontyRepl::new("ns.py", ResourceTracker::default(), CompileOptions::default());
    repl.set_namespaces(true);
    repl
}

fn feed(repl: &mut MontyRepl, code: &str) -> MontyObject {
    repl.feed_run(code, vec![], PrintWriter::Stdout).unwrap().value
}

fn feed_err(repl: &mut MontyRepl, code: &str) -> String {
    repl.feed_run(code, vec![], PrintWriter::Stdout)
        .unwrap_err()
        .to_string()
}

fn round_trip(repl: &MontyRepl) -> MontyRepl {
    let bytes = dump("ns.py", None, SessionRef::Idle(repl)).unwrap();
    match Dump::load(&bytes).unwrap().state {
        Session::Idle(repl) => *repl,
        _ => panic!("dumped an idle session, loaded something else"),
    }
}

fn t(value: bool) -> MontyObject {
    MontyObject::Bool(value)
}

#[test]
fn minting_is_gated_on_the_session_option() {
    let mut repl = MontyRepl::new("ns.py", ResourceTracker::default(), CompileOptions::default());
    let error = feed_err(&mut repl, "namespace()");
    assert!(error.contains("namespaces are not enabled"), "{error}");
}

#[test]
fn a_bind_through_the_handle_shares_the_object_and_not_the_binding() {
    let mut repl = session();
    feed(&mut repl, "board = []");
    feed(&mut repl, "ns = namespace({'board': board})");
    // Mutation through the handle's read is mutation of the one object.
    feed(&mut repl, "ns['board'].append(1)");
    assert_eq!(feed(&mut repl, "board == [1]"), t(true));
    // Rebinding through the handle crosses in neither direction.
    feed(&mut repl, "ns['board'] = [9]");
    assert_eq!(feed(&mut repl, "board == [1]"), t(true));
    assert_eq!(feed(&mut repl, "ns['board'] == [9]"), t(true));
}

#[test]
fn a_namespace_source_is_copied() {
    let mut repl = session();
    feed(&mut repl, "board = []\nns = namespace({'board': board})");
    feed(&mut repl, "child = namespace(ns)");
    feed(&mut repl, "child['board'].append(2)");
    // The object is shared all the way down...
    assert_eq!(feed(&mut repl, "board == [2]"), t(true));
    // ...and the name maps are independent.
    feed(&mut repl, "child['board'] = [7]");
    assert_eq!(feed(&mut repl, "ns['board'] == [2]"), t(true));
}

#[test]
fn copy_is_the_shallow_one_the_constructor_also_gives() {
    let mut repl = session();
    feed(&mut repl, "board = []\nns = namespace({'board': board})");
    feed(&mut repl, "twin = ns.copy()");
    // Same objects...
    assert_eq!(feed(&mut repl, "twin['board'] is board"), t(true));
    // ...own names.
    feed(&mut repl, "twin['board'] = [1]");
    assert_eq!(feed(&mut repl, "ns['board'] is board"), t(true));
}

#[test]
fn the_handle_wears_a_mapping_surface() {
    let mut repl = session();
    feed(&mut repl, "x = 1\ny = [2]\nns = namespace({'x': x, 'y': y})");
    assert_eq!(feed(&mut repl, "len(ns) == 2"), t(true));
    assert_eq!(feed(&mut repl, "'x' in ns and 'z' not in ns"), t(true));
    assert_eq!(feed(&mut repl, "sorted(ns.keys()) == ['x', 'y']"), t(true));
    assert_eq!(feed(&mut repl, "sorted(ns) == ['x', 'y']"), t(true));
    assert_eq!(
        feed(
            &mut repl,
            "ns.get('x') == 1 and ns.get('z') is None and ns.get('z', 9) == 9"
        ),
        t(true)
    );
    assert_eq!(feed(&mut repl, "dict(ns) == {'x': 1, 'y': [2]}"), t(true));
    assert_eq!(feed(&mut repl, "sorted(ns.items()) == [('x', 1), ('y', [2])]"), t(true));
    assert_eq!(feed(&mut repl, "ns['x'] == 1"), t(true));
    let missing = feed_err(&mut repl, "ns['z']");
    assert!(missing.contains("KeyError"), "{missing}");
    // The snapshot is a dict of its own: rebinding in it changes nothing here.
    feed(&mut repl, "dict(ns)['x'] = 5");
    assert_eq!(feed(&mut repl, "ns['x'] == 1"), t(true));
}

/// A handle the host holds keeps its whole mapping surface when it is handed
/// back in: it is a reference to the namespace, not a copy of what it held.
#[test]
fn a_handle_the_host_holds_reads_and_binds_when_it_comes_back() {
    let mut repl = session();
    repl.set_cross_by_reference(true);
    feed(&mut repl, "x = 1\nspare = None\nns = namespace({'x': x})");
    let held = repl.export_global("ns", PrintWriter::Stdout).expect("`ns` is bound");
    let MontyObject::SessionRef { id, .. } = held else {
        panic!("a namespace crosses as a reference, got {held:?}");
    };
    let handed = |repr: &str| {
        vec![(
            "held".to_owned(),
            MontyObject::SessionRef {
                id,
                repr: repr.to_owned(),
            },
        )]
    };

    // The whole surface answers through the reference, reads and binds alike.
    assert_eq!(
        repl.probe_scoped("(len(held), held['x'], 'x' in held, sorted(held))", handed(""), PrintWriter::Stdout)
            .unwrap(),
        MontyObject::Tuple(vec![
            MontyObject::Int(1),
            MontyObject::Int(1),
            t(true),
            MontyObject::List(vec![MontyObject::String("x".to_owned())]),
        ])
    );
    repl.run_scoped_in(None, "held['spare'] = 7", handed(""), PrintWriter::Stdout)
        .unwrap();
    assert!(repl.release(id));

    // And the bind landed in the namespace itself, which the session's own
    // code reads through the handle it still has.
    assert_eq!(feed(&mut repl, "ns['spare'] == 7 and len(ns) == 2"), t(true));
}

#[test]
fn a_name_no_code_ever_mentioned_cannot_be_bound() {
    let mut repl = session();
    feed(&mut repl, "ns = namespace()");
    let error = feed_err(&mut repl, "ns['zzz_never_compiled'] = 1");
    assert!(error.contains("no program slot"), "{error}");
    // One compiled mention anywhere mints the slot; then the bind lands.
    feed(&mut repl, "zzz_never_compiled = None");
    feed(&mut repl, "ns['zzz_never_compiled'] = 1");
    assert_eq!(feed(&mut repl, "ns['zzz_never_compiled'] == 1"), t(true));
}

#[test]
fn del_unbinds_and_missing_del_refuses() {
    let mut repl = session();
    feed(&mut repl, "x = 1\nns = namespace({'x': x})");
    feed(&mut repl, "del ns['x']");
    assert_eq!(feed(&mut repl, "'x' not in ns and len(ns) == 0"), t(true));
    let error = feed_err(&mut repl, "del ns['x']");
    assert!(error.contains("KeyError"), "{error}");
}

#[test]
fn handles_are_references_and_equality_is_the_namespace() {
    let mut repl = session();
    feed(&mut repl, "ns = namespace()\nsame = ns\nother = namespace()");
    assert_eq!(feed(&mut repl, "ns == same"), t(true));
    assert_eq!(feed(&mut repl, "ns == other"), t(false));
    assert_eq!(feed(&mut repl, "ns == 3"), t(false));
}

#[test]
fn dropping_the_last_handle_releases_the_namespace() {
    let mut repl = session();
    feed(&mut repl, "a = namespace()\nfirst = repr(a)");
    // The drop happens here; the release is swept at the feed boundary.
    feed(&mut repl, "a = None");
    // A released slot is reused, which is only possible if it was released.
    feed(&mut repl, "b = namespace()");
    assert_eq!(feed(&mut repl, "repr(b) == first"), t(true));
}

#[test]
fn the_mode_the_handles_and_their_contents_survive_dump_and_load() {
    let mut repl = session();
    feed(&mut repl, "board = []\nns = namespace({'board': board})");
    let mut repl = round_trip(&repl);
    // The handle still names the same namespace over the same heap.
    feed(&mut repl, "ns['board'].append(3)");
    assert_eq!(feed(&mut repl, "board == [3]"), t(true));
    // The mode traveled: minting still works without re-enabling.
    feed(&mut repl, "extra = namespace()");
    assert_eq!(feed(&mut repl, "extra == ns"), t(false));
}

#[test]
fn a_non_mapping_source_is_refused() {
    let mut repl = session();
    let error = feed_err(&mut repl, "namespace(3)");
    assert!(error.contains("takes a namespace or a dict"), "{error}");
    let keys = feed_err(&mut repl, "namespace({1: 2})");
    assert!(keys.contains("namespace keys are names"), "{keys}");
}
