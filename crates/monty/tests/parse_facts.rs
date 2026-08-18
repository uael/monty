//! Reading a snippet without running it: the distinction between unfinished
//! and wrong, and the bindings a host wants to know about before it runs.

use monty::parse_facts;
use monty_types::ParseFacts;

fn facts(code: &str) -> ParseFacts {
    parse_facts(code, "said.py", &[])
}

fn binds(code: &str, names: &[&str]) -> Vec<String> {
    let asked: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
    parse_facts(code, "said.py", &asked).stores
}

/// Unfinished input is a request for more, not an error: exactly the line
/// CPython's `codeop.compile_command` draws for an interactive prompt.
#[test]
fn unfinished_input_is_not_an_error() {
    for code in [
        "x = (",
        "if a:",
        "def f():",
        "'''abc",
        "xs = [1,\n2,",
        "with open('f') as f:",
    ] {
        let got = facts(code);
        assert!(!got.complete, "{code:?} should read as unfinished");
        assert!(got.error.is_none(), "{code:?} should carry no error");
    }
}

/// A snippet that is wrong rather than unfinished carries the syntax error a
/// feed of it would raise, filename and all.
#[test]
fn a_real_syntax_error_is_reported() {
    let got = facts("x = )");
    assert!(got.complete);
    let error = got.error.expect("a malformed snippet reports its error");
    assert_eq!(error.summary(), "SyntaxError: Expected an expression");
    assert!(error.to_string().contains("said.py"), "got: {error}");
}

/// Source that parses reports no error, whether or not Monty would run it: a
/// module-level `return` is a compile-time SyntaxError in CPython and simply
/// runs here, so reading it says nothing is wrong.
#[test]
fn source_that_parses_carries_no_error() {
    for code in ["x = 1", "", "x = 1\nreturn x", "class A:\n    pass"] {
        let got = facts(code);
        assert!(got.complete, "{code:?}");
        assert!(got.error.is_none(), "{code:?}");
        assert!(!got.binds_global, "{code:?}");
    }
}

/// `global` is reported from any scope: a declaration inside a function is
/// precisely the one that reaches out of it.
#[test]
fn global_is_found_in_every_scope() {
    assert!(facts("global g").binds_global);
    assert!(facts("def f():\n    global g\n    g = 1").binds_global);
    assert!(facts("class A:\n    def m(self):\n        global g\n        g = 1").binds_global);
    assert!(facts("if a:\n    def f():\n        global g").binds_global);
    assert!(!facts("g = 1\ndef f():\n    return g").binds_global);
}

/// Module-level bindings, in every shape that makes one.
#[test]
fn module_level_bindings_are_reported() {
    for (code, name) in [
        ("x = 1", "x"),
        ("x, y = 1, 2", "y"),
        ("x += 1", "x"),
        ("x: int = 1", "x"),
        ("for x in xs:\n    pass", "x"),
        ("with open('f') as x:\n    pass", "x"),
        ("while (x := next(it)):\n    pass", "x"),
        ("def x():\n    pass", "x"),
        ("class x:\n    pass", "x"),
        ("import x", "x"),
        ("import a.b as x", "x"),
        ("import a.b", "a"),
        ("from a import x", "x"),
        ("from a import b as x", "x"),
        ("try:\n    pass\nexcept ValueError as x:\n    pass", "x"),
        ("type x = int", "x"),
        ("if a:\n    x = 1", "x"),
        ("[y for y in xs if (x := y)]", "x"),
    ] {
        assert_eq!(binds(code, &[name]), vec![name.to_owned()], "{code:?} binds {name}");
    }
}

/// A binding made inside a `def`, `class` or `lambda` belongs to that scope,
/// not to the module.
#[test]
fn bindings_in_a_nested_scope_are_not_module_level() {
    for code in [
        "def f():\n    x = 1",
        "def f(x):\n    return x",
        "class A:\n    x = 1",
        "f = lambda: (x := 1)",
        "def f():\n    for x in xs:\n        pass",
        // A comprehension is a scope of its own, and Monty's compiler already
        // gives it one: after `[x for x in xs]` the name is undefined, exactly
        // as in CPython.
        "ys = [x for x in xs]",
        "ys = {x for x in xs}",
        "ys = {x: 1 for x in xs}",
        "ys = list(x for x in xs)",
        "ys = [a for b in xs for x in b]",
        "ys = [a for b in xs if all(x for x in b)]",
    ] {
        assert_eq!(binds(code, &["x"]), Vec::<String>::new(), "{code:?}");
    }
}

/// The one name a comprehension does write into the enclosing scope: PEP 572
/// says a walrus reaches out of it, wherever in the clause it stands.
#[test]
fn a_walrus_inside_a_comprehension_binds_at_module_level() {
    for code in [
        "ys = [(x := y) for y in items]",
        "ys = [y for y in items if (x := y)]",
        "ys = [y for y in (x := items)]",
        "ys = {(x := y): y for y in items}",
        "ys = list((x := y) for y in items)",
    ] {
        assert_eq!(binds(code, &["x"]), vec!["x".to_owned()], "{code:?}");
    }
}

/// A name evaluated at module level inside a `def`'s decorators, defaults or
/// annotations is still a module binding, even though the body is not.
#[test]
fn a_walrus_in_a_signature_binds_at_module_level() {
    assert_eq!(binds("def f(a=(x := 1)):\n    pass", &["x"]), vec!["x".to_owned()]);
    assert_eq!(binds("@(x := deco)\ndef f():\n    pass", &["x"]), vec!["x".to_owned()]);
    assert_eq!(binds("class A((x := Base)):\n    pass", &["x"]), vec!["x".to_owned()]);
}

/// Only the names asked about are answered, in the order asked, so a caller
/// reads the answer positionally against its own question.
#[test]
fn only_the_names_asked_about_come_back() {
    assert_eq!(
        binds("a = 1\nb = 2", &["b", "c", "a"]),
        vec!["b".to_owned(), "a".to_owned()]
    );
    assert_eq!(binds("a = 1", &[]), Vec::<String>::new());
    // an unparsable snippet binds nothing it can be held to
    assert_eq!(binds("a = (", &["a"]), Vec::<String>::new());
}
