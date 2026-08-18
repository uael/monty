//! Copying a namespace deeply, so the copy shares nothing with it.
//!
//! Copying a namespace shares the objects; copying it deeply copies them too.
//! These tests prove the copy
//! by mutating one side and reading the other, and prove the sharing that
//! survives by identity (`is`), never by equality, which would pass for a copy
//! too.

use monty::MontyRepl;
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

fn t(value: bool) -> MontyObject {
    MontyObject::Bool(value)
}

/// Every test deep-copies the namespace its own code was defined in, which is the
/// only way to have a donor that owns functions and classes.
const HERE: &str = "here = namespace.current()\n";

#[test]
fn a_deep_copy_copies_what_a_plain_copy_would_share() {
    let mut repl = session();
    feed(&mut repl, "board = [1]");
    feed(&mut repl, &format!("{HERE}mine = here.deepcopy()"));
    feed(&mut repl, "mine['board'].append(2)");
    assert_eq!(feed(&mut repl, "board == [1]"), t(true));
    assert_eq!(feed(&mut repl, "mine['board'] == [1, 2]"), t(true));
}

#[test]
fn a_function_defined_in_the_donor_reads_and_writes_the_copy() {
    let mut repl = session();
    feed(&mut repl, "seen = []\ndef note(x):\n    seen.append(x)\n");
    feed(&mut repl, &format!("{HERE}mine = here.deepcopy()"));
    feed(&mut repl, "mine['note'](1)");
    // The copy's function wrote the copy's `seen`, and nothing here moved.
    assert_eq!(feed(&mut repl, "seen == []"), t(true));
    assert_eq!(feed(&mut repl, "mine['seen'] == [1]"), t(true));
    // And this side's own function still writes this side's.
    feed(&mut repl, "note(9)");
    assert_eq!(feed(&mut repl, "seen == [9] and mine['seen'] == [1]"), t(true));
}

#[test]
fn a_class_defined_in_the_donor_is_rebuilt_and_its_methods_follow() {
    let mut repl = session();
    feed(
        &mut repl,
        "tally = []\nclass Counter:\n    kept = []\n    def count(self):\n        tally.append(1)\n",
    );
    feed(&mut repl, &format!("{HERE}mine = here.deepcopy()"));
    feed(&mut repl, "mine['Counter']().count()");
    // The copied class's method resolved `tally` in the copy.
    assert_eq!(feed(&mut repl, "tally == []"), t(true));
    assert_eq!(feed(&mut repl, "mine['tally'] == [1]"), t(true));
    // Class attributes are state, so they are copied too.
    feed(&mut repl, "mine['Counter'].kept.append('x')");
    assert_eq!(feed(&mut repl, "Counter.kept == []"), t(true));
}

#[test]
fn what_was_defined_elsewhere_stays_the_same_object() {
    let mut repl = session();
    feed(&mut repl, "import json");
    feed(&mut repl, "text = 'a name'\nnumber = 10 ** 30");
    feed(&mut repl, &format!("{HERE}mine = here.deepcopy()"));
    // A module, and immutables, have no state to diverge.
    assert_eq!(feed(&mut repl, "mine['json'] is json"), t(true));
    assert_eq!(feed(&mut repl, "mine['text'] is text"), t(true));
    assert_eq!(feed(&mut repl, "mine['number'] is number"), t(true));
}

#[test]
fn deepcopy_decides_what_keeps_its_identity() {
    let mut repl = session();
    feed(
        &mut repl,
        "class Seat:\n    def __deepcopy__(self, memo):\n        return self\n\nclass Plain:\n    pass\n",
    );
    feed(&mut repl, "seat = Seat()\nplain = Plain()");
    feed(&mut repl, &format!("{HERE}mine = here.deepcopy()"));
    // The class said so, so the copy holds the very same object...
    assert_eq!(feed(&mut repl, "mine['seat'] is seat"), t(true));
    // ...while one that said nothing is copied.
    assert_eq!(feed(&mut repl, "mine['plain'] is plain"), t(false));
}

#[test]
fn what_the_donor_shared_with_itself_the_copy_shares_with_itself() {
    let mut repl = session();
    feed(&mut repl, "one = []\ntwo = one\nboth = [one, two]");
    feed(&mut repl, &format!("{HERE}mine = here.deepcopy()"));
    // Aliasing is part of the shape, so it survives the copy...
    assert_eq!(feed(&mut repl, "mine['one'] is mine['two']"), t(true));
    assert_eq!(feed(&mut repl, "mine['both'][0] is mine['one']"), t(true));
    // ...and it is the copy's own aliasing, not the donor's.
    assert_eq!(feed(&mut repl, "mine['one'] is one"), t(false));
}

#[test]
fn a_value_that_reaches_itself_is_copied_rather_than_chased() {
    let mut repl = session();
    feed(&mut repl, "loop = []\nloop.append(loop)");
    feed(&mut repl, &format!("{HERE}mine = here.deepcopy()"));
    assert_eq!(feed(&mut repl, "mine['loop'][0] is mine['loop']"), t(true));
    assert_eq!(feed(&mut repl, "mine['loop'] is loop"), t(false));
}

#[test]
fn a_dict_keyed_by_an_object_can_still_be_read_after_the_copy() {
    let mut repl = session();
    // A key hashed by identity lands in a different bucket once copied, so a
    // copy that moved the entries without re-hashing them would lose this.
    feed(
        &mut repl,
        "class Key:\n    pass\n\nk = Key()\nby_object = {k: 'found'}\nnested = {'k': k}",
    );
    feed(&mut repl, &format!("{HERE}mine = here.deepcopy()"));
    assert_eq!(
        feed(&mut repl, "mine['by_object'][mine['nested']['k']] == 'found'"),
        t(true)
    );
    assert_eq!(feed(&mut repl, "len(mine['by_object']) == 1"), t(true));
}

#[test]
fn a_captured_cell_is_copied_with_the_function_that_captured_it() {
    let mut repl = session();
    feed(
        &mut repl,
        "def make():\n    held = []\n    def add(x):\n        held.append(x)\n    def read():\n        return list(held)\n    return add, read\n\nadd, read = make()\n",
    );
    feed(&mut repl, &format!("{HERE}mine = here.deepcopy()"));
    feed(&mut repl, "mine['add'](1)");
    // The copy's pair still share their cell with each other...
    assert_eq!(feed(&mut repl, "mine['read']() == [1]"), t(true));
    // ...and not with the donor's pair.
    assert_eq!(feed(&mut repl, "read() == []"), t(true));
}

#[test]
fn what_no_copy_could_carry_is_refused_by_name() {
    let mut repl = session();
    feed(&mut repl, "def counter():\n    yield 1\n\nlive = counter()\n");
    let error = feed_err(&mut repl, &format!("{HERE}here.deepcopy()"));
    assert!(error.contains("a deep copy cannot carry"), "{error}");
    assert!(error.contains("generator"), "{error}");
    // The refusal left the session standing.
    assert_eq!(feed(&mut repl, "1 + 1"), MontyObject::Int(2));
}

#[test]
fn each_namespace_runs_its_own_code() {
    let mut repl = session();
    feed(&mut repl, "log = []\ndef put(x):\n    log.append(x)\n");
    feed(&mut repl, &format!("{HERE}mine = here.deepcopy()"));
    feed(&mut repl, "mine['put']('to the copy')");
    feed(&mut repl, "put('to here')");
    assert_eq!(feed(&mut repl, "log == ['to here']"), t(true));
    assert_eq!(feed(&mut repl, "mine['log'] == ['to the copy']"), t(true));
}

#[test]
fn a_handle_onto_the_running_namespace_owns_nothing() {
    let mut repl = session();
    feed(&mut repl, "x = 1");
    // Taken and dropped: were it an owning handle, the namespace this session
    // is running in would have been condemned by the drop.
    feed(&mut repl, "namespace.current()\nnamespace.current()\n");
    feed(&mut repl, "y = 2");
    assert_eq!(feed(&mut repl, "x + y == 3"), t(true));
}

#[test]
fn the_current_handle_is_gated_on_the_session_option() {
    let mut repl = MontyRepl::new("ns.py", ResourceTracker::default(), CompileOptions::default());
    let error = feed_err(&mut repl, "namespace.current()");
    assert!(error.contains("namespaces are not enabled"), "{error}");
}
