//! Several global namespaces over one heap.
//!
//! A namespace is a name map, not a heap. Dressing one from another copies the
//! names and shares the objects, so a rebinding crosses in neither direction
//! and a mutation crosses in both; forking a session copies the heap and
//! shares nothing. These tests state both laws, and prove sharing by mutating
//! through one namespace and reading through the other rather than by
//! comparing values that merely look alike.

use std::time::Instant;

use monty::{Dump, MontyRepl, ReplProgress, ScopeId, Session, SessionRef, dump};
use monty_types::{CompileOptions, MontyObject, PrintWriter, ResourceTracker};

fn session() -> MontyRepl {
    MontyRepl::new("ns.py", ResourceTracker::default(), CompileOptions::default())
}

/// Feeds `code` to `scope`, leaving the session selecting whatever it did.
fn feed_in(repl: &mut MontyRepl, scope: ScopeId, code: &str) -> MontyObject {
    let was = repl.namespace();
    repl.select_namespace(scope).unwrap();
    let out = repl.feed_run(code, vec![], PrintWriter::Stdout);
    repl.select_namespace(was).unwrap();
    out.unwrap().value
}

fn feed_err(repl: &mut MontyRepl, scope: ScopeId, code: &str) -> String {
    let was = repl.namespace();
    repl.select_namespace(scope).unwrap();
    let out = repl.feed_run(code, vec![], PrintWriter::Stdout);
    repl.select_namespace(was).unwrap();
    out.unwrap_err().to_string()
}

fn feed(repl: &mut MontyRepl, code: &str) -> MontyObject {
    repl.feed_run(code, vec![], PrintWriter::Stdout).unwrap().value
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

// ---------------------------------------------------------------------------
// The shallow law
// ---------------------------------------------------------------------------

#[test]
fn a_dressed_child_shares_objects_and_not_bindings() {
    let mut repl = session();
    feed(&mut repl, "board = []");
    feed(&mut repl, "x = 1");
    let parent = repl.namespace();
    let child = repl.dress_namespace(parent).unwrap();

    // A mutation crosses, child to parent.
    feed_in(&mut repl, child, "board.append('c')");
    assert_eq!(feed(&mut repl, "board == ['c']"), t(true));

    // And parent to child.
    feed(&mut repl, "board.append('p')");
    assert_eq!(feed_in(&mut repl, child, "board == ['c', 'p']"), t(true));

    // A rebinding crosses neither way.
    feed_in(&mut repl, child, "x = 2");
    assert_eq!(feed(&mut repl, "x"), MontyObject::Int(1));
    assert_eq!(feed_in(&mut repl, child, "x"), MontyObject::Int(2));

    // Rebinding a shared name in the parent leaves the child on the old object.
    feed(&mut repl, "board = ['replaced']");
    assert_eq!(feed_in(&mut repl, child, "board == ['c', 'p']"), t(true));
}

#[test]
fn a_name_only_the_child_binds_is_not_a_name_the_parent_has() {
    let mut repl = session();
    feed(&mut repl, "shared = 1");
    let parent = repl.namespace();
    let child = repl.dress_namespace(parent).unwrap();

    feed_in(&mut repl, child, "only_child = 7");
    assert_eq!(feed_in(&mut repl, child, "only_child"), MontyObject::Int(7));

    let err = feed_err(&mut repl, parent, "only_child");
    assert!(err.contains("NameError"), "expected NameError, got {err}");
}

#[test]
fn an_empty_namespace_starts_with_nothing_bound() {
    let mut repl = session();
    feed(&mut repl, "x = 1");
    let fresh = repl.create_namespace().unwrap();

    let err = feed_err(&mut repl, fresh, "x");
    assert!(err.contains("NameError"), "expected NameError, got {err}");

    // And it is a working namespace, not merely an empty one.
    feed_in(&mut repl, fresh, "x = 'mine'");
    assert_eq!(feed_in(&mut repl, fresh, "x"), MontyObject::String("mine".to_owned()));
    assert_eq!(feed(&mut repl, "x"), MontyObject::Int(1));
}

#[test]
fn siblings_share_the_parent_and_not_each_other() {
    let mut repl = session();
    feed(&mut repl, "board = []");
    feed(&mut repl, "tag = 'parent'");
    let parent = repl.namespace();

    let kids: Vec<ScopeId> = (0..3).map(|_| repl.dress_namespace(parent).unwrap()).collect();
    for (i, &kid) in kids.iter().enumerate() {
        feed_in(&mut repl, kid, &format!("tag = 'kid{i}'"));
        feed_in(&mut repl, kid, &format!("board.append({i})"));
    }

    assert_eq!(feed(&mut repl, "tag"), MontyObject::String("parent".to_owned()));
    assert_eq!(feed(&mut repl, "board == [0, 1, 2]"), t(true));
    for (i, &kid) in kids.iter().enumerate() {
        assert_eq!(feed_in(&mut repl, kid, "tag"), MontyObject::String(format!("kid{i}")));
    }
}

// ---------------------------------------------------------------------------
// Identity, and where a function reads its globals
// ---------------------------------------------------------------------------

#[test]
fn a_class_crosses_as_itself_not_as_a_twin() {
    let mut repl = session();
    feed(&mut repl, "class C:\n    def __init__(self, n):\n        self.n = n\n");
    feed(&mut repl, "inst = C(1)");
    feed(&mut repl, "board = []");
    let parent = repl.namespace();
    let child = repl.dress_namespace(parent).unwrap();

    assert_eq!(feed_in(&mut repl, child, "type(inst) is C"), t(true));

    // The child builds an instance under its own name for the class and hands
    // it over through the shared board; the parent's `C` claims it.
    feed_in(&mut repl, child, "board.append(C(2))");
    assert_eq!(feed(&mut repl, "isinstance(board[0], C)"), t(true));
    assert_eq!(feed(&mut repl, "board[0].n"), MontyObject::Int(2));
    assert_eq!(feed(&mut repl, "type(board[0]) is type(inst)"), t(true));
}

#[test]
fn a_function_reads_the_namespace_it_was_defined_in() {
    let mut repl = session();
    feed(&mut repl, "scale = 10");
    feed(&mut repl, "def scaled(v):\n    return v * scale\n");
    let parent = repl.namespace();
    let child = repl.dress_namespace(parent).unwrap();

    assert_eq!(feed_in(&mut repl, child, "scaled(2)"), MontyObject::Int(20));

    // The child rebinds the name the parent's function reads. A rebinding does
    // not cross, so the function keeps reading the parent's.
    feed_in(&mut repl, child, "scale = 100");
    assert_eq!(feed_in(&mut repl, child, "scale"), MontyObject::Int(100));
    assert_eq!(
        feed_in(&mut repl, child, "scaled(2)"),
        MontyObject::Int(20),
        "a function defined in the parent resolves globals there, not in its caller"
    );
    assert_eq!(feed(&mut repl, "scaled(2)"), MontyObject::Int(20));

    // A function the child defines for itself reads the child's.
    feed_in(&mut repl, child, "def own(v):\n    return v * scale\n");
    assert_eq!(feed_in(&mut repl, child, "own(2)"), MontyObject::Int(200));
    assert_eq!(feed(&mut repl, "scaled(2)"), MontyObject::Int(20));
}

#[test]
fn a_function_writes_the_namespace_it_was_defined_in() {
    let mut repl = session();
    feed(&mut repl, "seen = 0");
    feed(&mut repl, "def bump():\n    global seen\n    seen = seen + 1\n");
    let parent = repl.namespace();
    let child = repl.dress_namespace(parent).unwrap();

    feed_in(&mut repl, child, "bump()");
    assert_eq!(
        feed(&mut repl, "seen"),
        MontyObject::Int(1),
        "the store landed where the function was defined"
    );
    assert_eq!(
        feed_in(&mut repl, child, "seen"),
        MontyObject::Int(0),
        "and not in the namespace that called it"
    );
}

#[test]
fn a_generator_resumes_in_the_namespace_it_was_defined_in() {
    let mut repl = session();
    feed(&mut repl, "step = 1");
    feed(&mut repl, "def counter():\n    yield step\n    yield step\n");
    let parent = repl.namespace();
    let child = repl.dress_namespace(parent).unwrap();

    feed_in(&mut repl, child, "g = counter()");
    assert_eq!(feed_in(&mut repl, child, "next(g)"), MontyObject::Int(1));
    // Rebinding in the child must not reach a generator whose def ran in the parent.
    feed_in(&mut repl, child, "step = 99");
    assert_eq!(feed_in(&mut repl, child, "next(g)"), MontyObject::Int(1));
}

// ---------------------------------------------------------------------------
// The dump
// ---------------------------------------------------------------------------

#[test]
fn a_dump_carries_the_sharing() {
    let mut repl = session();
    feed(&mut repl, "board = []");
    feed(&mut repl, "x = 1");
    let parent = repl.namespace();
    let child = repl.dress_namespace(parent).unwrap();
    feed_in(&mut repl, child, "x = 2");
    feed_in(&mut repl, child, "board.append('before')");

    let mut woken = round_trip(&repl);
    drop(repl);

    // Distinct bindings survive.
    assert_eq!(feed(&mut woken, "x"), MontyObject::Int(1));
    assert_eq!(feed_in(&mut woken, child, "x"), MontyObject::Int(2));

    // And the two maps still name one object, not two copies of it.
    feed_in(&mut woken, child, "board.append('after')");
    assert_eq!(feed(&mut woken, "board == ['before', 'after']"), t(true));
}

#[test]
fn a_dump_carries_where_a_function_reads_its_globals() {
    let mut repl = session();
    feed(&mut repl, "scale = 10");
    feed(&mut repl, "def scaled(v):\n    return v * scale\n");
    let child = repl.dress_namespace(repl.namespace()).unwrap();
    feed_in(&mut repl, child, "scale = 100");

    let mut woken = round_trip(&repl);
    drop(repl);
    assert_eq!(feed_in(&mut woken, child, "scaled(2)"), MontyObject::Int(20));
    assert_eq!(feed_in(&mut woken, child, "scale"), MontyObject::Int(100));
}

#[test]
fn a_fork_shares_nothing() {
    let mut repl = session();
    feed(&mut repl, "board = []");
    let child = repl.dress_namespace(repl.namespace()).unwrap();

    let mut left = round_trip(&repl);
    let mut right = round_trip(&repl);
    drop(repl);

    feed_in(&mut left, child, "board.append('left')");
    assert_eq!(feed(&mut left, "board == ['left']"), t(true));
    assert_eq!(feed(&mut right, "board == []"), t(true));
    assert_eq!(
        feed_in(&mut right, child, "board == []"),
        t(true),
        "a branch must not be able to mutate its origin"
    );
}

// ---------------------------------------------------------------------------
// Reading one namespace while another is suspended
// ---------------------------------------------------------------------------

#[test]
fn a_second_namespace_is_readable_while_the_first_is_suspended() {
    let mut repl = session();
    feed(&mut repl, "board = []");
    feed(&mut repl, "x = 1");
    let parent = repl.namespace();
    let child = repl.dress_namespace(parent).unwrap();
    feed_in(&mut repl, child, "x = 2");
    feed_in(&mut repl, child, "who = 'child'");

    let mut progress = repl
        .feed_start("board.append(ask(1))", vec![], PrintWriter::Stdout)
        .expect("suspends at the undefined callable");
    assert!(matches!(progress, ReplProgress::FunctionCall(_)));

    // The suspended namespace reads as itself.
    assert_eq!(
        progress.probe("x", vec![], PrintWriter::Stdout).unwrap(),
        MontyObject::Int(1)
    );
    // And the other one reads as itself, at the same moment.
    assert_eq!(
        progress
            .probe_in(Some(child), "x", vec![], PrintWriter::Stdout)
            .unwrap(),
        MontyObject::Int(2)
    );
    assert_eq!(
        progress
            .probe_in(Some(child), "who", vec![], PrintWriter::Stdout)
            .unwrap(),
        MontyObject::String("child".to_owned())
    );
    // A write through the parked namespace reaches the object the suspended
    // snippet is about to append to.
    progress
        .probe_in(
            Some(child),
            "board.append('while suspended')",
            vec![],
            PrintWriter::Stdout,
        )
        .unwrap();

    let call = progress.into_function_call().expect("still a function call");
    let progress = call.resume(MontyObject::Int(2), PrintWriter::Stdout).unwrap();
    let (mut repl, _) = progress.into_complete().expect("completes");
    assert_eq!(feed(&mut repl, "board == ['while suspended', 2]"), t(true));
    assert_eq!(feed_in(&mut repl, child, "x"), MontyObject::Int(2));
}

#[test]
fn probing_a_namespace_the_session_does_not_have_is_refused() {
    let mut repl = session();
    feed(&mut repl, "x = 1");
    let gone = repl.create_namespace().unwrap();
    assert!(repl.release_namespace(gone));

    let err = repl
        .probe_scoped_in(Some(gone), "x", vec![], PrintWriter::Stdout)
        .unwrap_err()
        .to_string();
    assert!(err.contains("no such namespace"), "got {err}");
    assert!(repl.select_namespace(gone).is_err());
}

// ---------------------------------------------------------------------------
// Release
// ---------------------------------------------------------------------------

#[test]
fn releasing_a_namespace_keeps_what_another_still_names() {
    let mut repl = session();
    feed(&mut repl, "board = []");
    let parent = repl.namespace();
    let child = repl.dress_namespace(parent).unwrap();

    feed_in(&mut repl, child, "board.append('written')");
    assert!(repl.release_namespace(child));
    assert!(!repl.release_namespace(child), "releasing twice is not a release");

    // The list outlived the namespace that wrote to it, because the parent
    // still names it.
    assert_eq!(feed(&mut repl, "board == ['written']"), t(true));
}

#[test]
fn the_selected_namespace_cannot_be_released() {
    let mut repl = session();
    feed(&mut repl, "x = 1");
    assert!(
        !repl.release_namespace(repl.namespace()),
        "a session always needs somewhere to run"
    );
    assert_eq!(feed(&mut repl, "x"), MontyObject::Int(1));
}

// ---------------------------------------------------------------------------
// The event loop across namespaces
// ---------------------------------------------------------------------------

/// Tasks belonging to different namespaces interleave, and each resolves its
/// globals against its own however many others ran in between.
///
/// The coroutines are defined in two namespaces and awaited together in one,
/// so the loop switches namespace on every task switch rather than once.
#[test]
fn the_loop_runs_tasks_of_different_namespaces() {
    let mut repl = session();
    feed(&mut repl, "tag = 'parent'");
    // A board the parent owns, so both namespaces name the same list.
    feed(&mut repl, "board = []");
    feed(
        &mut repl,
        "async def who():\n    import asyncio\n    await asyncio.sleep(0)\n    return tag\n",
    );
    let parent = repl.namespace();
    let child = repl.dress_namespace(parent).unwrap();
    feed_in(&mut repl, child, "tag = 'child'");
    // A coroutine of the child's own, over the child's `tag`.
    feed_in(
        &mut repl,
        child,
        "async def mine():\n    import asyncio\n    await asyncio.sleep(0)\n    return tag\n",
    );
    // Hand the child's coroutine function to the parent through the shared list.
    feed_in(&mut repl, child, "board.append(mine)");

    // Both run in one loop, in the parent's namespace.
    assert_eq!(
        feed(
            &mut repl,
            "import asyncio\nresults = await asyncio.gather(who(), board[0]())\nresults == ['parent', 'child']"
        ),
        t(true),
        "each task read the namespace its own def ran in"
    );
}

/// A task parked on a timer resumes in its own namespace after a task from
/// another namespace has run and moved the installed one.
#[test]
fn a_parked_task_resumes_in_its_own_namespace() {
    let mut repl = session();
    feed(&mut repl, "tag = 'parent'");
    feed(&mut repl, "board = []");
    feed(
        &mut repl,
        "async def slow():\n    import asyncio\n    await asyncio.sleep(0.01)\n    return tag\n",
    );
    let child = repl.dress_namespace(repl.namespace()).unwrap();
    feed_in(&mut repl, child, "tag = 'child'");
    feed_in(
        &mut repl,
        child,
        "async def quick():\n    import asyncio\n    await asyncio.sleep(0)\n    return tag\n",
    );
    feed_in(&mut repl, child, "board.append(quick)");

    assert_eq!(
        feed(
            &mut repl,
            "import asyncio\nout = await asyncio.gather(slow(), board[0]())\nout == ['parent', 'child']"
        ),
        t(true)
    );
}

// ---------------------------------------------------------------------------
// What the shared slot map costs
// ---------------------------------------------------------------------------

/// A namespace made before a name existed still resolves that name's slot.
///
/// The slot map is shared and only grows, so every namespace is padded to it:
/// a namespace that missed the feeds which added slots reads `Undefined`
/// there, which is the `NameError` it should be, rather than indexing off the
/// end of a short vector.
#[test]
fn a_namespace_is_padded_to_names_added_after_it() {
    let mut repl = session();
    let early = repl.create_namespace().unwrap();

    for i in 0..64 {
        feed(&mut repl, &format!("later_{i} = {i}"));
    }

    let err = feed_err(&mut repl, early, "later_63");
    assert!(err.contains("NameError"), "expected NameError, got {err}");
    // And it is usable afterwards, at the grown width.
    feed_in(&mut repl, early, "later_63 = 'mine'");
    assert_eq!(
        feed_in(&mut repl, early, "later_63"),
        MontyObject::String("mine".to_owned())
    );
    assert_eq!(feed(&mut repl, "later_63"), MontyObject::Int(63));
}

/// Prints what a namespace costs and what a fork costs against dressing one.
/// Run with `--nocapture`; the assertions are the invariants, the numbers are
/// for the record.
#[test]
fn cost_of_a_namespace() {
    const NAMES: usize = 200;
    const FRAMES: usize = 500;

    let mut repl = session();
    for i in 0..NAMES {
        feed(&mut repl, &format!("v{i} = [{i}]"));
    }
    let parent = repl.namespace();

    let started = Instant::now();
    let kids: Vec<ScopeId> = (0..FRAMES).map(|_| repl.dress_namespace(parent).unwrap()).collect();
    let dress_each = started.elapsed() / u32::try_from(FRAMES).expect("frame count fits");

    let bytes = dump("ns.py", None, SessionRef::Idle(&repl)).unwrap();
    let started = Instant::now();
    for _ in 0..10 {
        drop(round_trip(&repl));
    }
    let fork_each = started.elapsed() / 10;

    // Each namespace is one Value per slot the program has, and Value is 16
    // bytes; that is the whole per-namespace cost of sharing one slot map.
    let per_namespace = 16 * NAMES;
    println!(
        "COST names={NAMES} frames={FRAMES} | namespace={per_namespace}B \
         total={}KiB | dress={dress_each:?}/ea | fork={fork_each:?}/ea dump={}B",
        per_namespace * FRAMES / 1024,
        bytes.len(),
    );

    // Sharing still holds at this width, which is what makes the arithmetic
    // above the whole cost rather than a floor.
    feed_in(&mut repl, kids[0], "v0.append('written')");
    assert_eq!(feed(&mut repl, "v0 == [0, 'written']"), t(true));
    assert_eq!(feed_in(&mut repl, kids[1], "v0 == [0, 'written']"), t(true));
}
