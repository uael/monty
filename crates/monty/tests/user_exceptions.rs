//! Sandbox-defined exception classes: raising, catching, and the paths where
//! Monty's identity handling diverges from CPython.
//!
//! Behaviour that CPython agrees with lives in
//! `test_cases/exception__user_classes.py`; this file covers what only Monty
//! does, plus the two boundaries a comparative test cannot express: an
//! exception escaping to the host, and one raised inside a synchronously
//! evaluated callback.

use monty::MontyRun;
use monty_types::{CompileOptions, ExcType, MontyObject, PrintWriter, ResourceTracker};

fn run(code: &str) -> Result<MontyObject, monty_types::MontyException> {
    let runner = MontyRun::new(code.to_owned(), "user_exc.py", vec![], CompileOptions::default()).unwrap();
    runner.run(vec![], ResourceTracker::default(), PrintWriter::Disabled)
}

/// An escaping sandbox exception reaches the host under its own name, with the
/// nearest builtin ancestor as the type a binding can reconstruct.
#[test]
fn escaping_user_exception_keeps_its_name() {
    let err = run("class Halt(Exception):\n    pass\nraise Halt('done')").unwrap_err();
    assert_eq!(err.user_type(), Some("Halt"));
    assert_eq!(err.exc_type(), ExcType::Exception);
    assert_eq!(err.message(), Some("done"));
    assert_eq!(err.summary(), "Halt: done");
    assert_eq!(err.py_repr(), "Halt('done')");
}

/// The builtin ancestor is the nearest one, not the root: a `ValueError`
/// subclass reaches the host as a `ValueError`.
#[test]
fn escaping_user_exception_reports_its_nearest_builtin_base() {
    let err = run("class Bad(ValueError):\n    pass\nraise Bad('nope')").unwrap_err();
    assert_eq!(err.user_type(), Some("Bad"));
    assert_eq!(err.exc_type(), ExcType::ValueError);
}

/// A builtin exception still has no user type, so nothing about the existing
/// host-facing shape changes.
#[test]
fn escaping_builtin_exception_has_no_user_type() {
    let err = run("raise ValueError('plain')").unwrap_err();
    assert_eq!(err.user_type(), None);
    assert_eq!(err.summary(), "ValueError: plain");
}

/// A raise that unwinds out of a synchronously evaluated callback (here a
/// `sorted` key function, which re-enters the interpreter natively) still
/// reaches a handler naming the sandbox class: the raised object is parked
/// across the boundary rather than rebuilt as its builtin base.
#[test]
fn user_exception_survives_a_synchronous_callback_boundary() {
    let code = concat!(
        "class Halt(Exception):\n",
        "    pass\n",
        "def key(n):\n",
        "    raise Halt('inside')\n",
        "try:\n",
        "    sorted([2, 1], key=key)\n",
        "    caught = 'none'\n",
        "except Halt as exc:\n",
        "    caught = 'halt:' + str(exc)\n",
        "caught\n",
    );
    assert_eq!(run(code).unwrap(), MontyObject::String("halt:inside".to_owned()));
}

/// The same boundary for a `__repr__`, which also runs re-entrantly.
#[test]
fn user_exception_survives_a_repr_boundary() {
    let code = concat!(
        "class Halt(Exception):\n",
        "    pass\n",
        "class Bomb:\n",
        "    def __repr__(self):\n",
        "        raise Halt('from repr')\n",
        "try:\n",
        "    repr(Bomb())\n",
        "    caught = 'none'\n",
        "except Halt as exc:\n",
        "    caught = str(exc)\n",
        "caught\n",
    );
    assert_eq!(run(code).unwrap(), MontyObject::String("from repr".to_owned()));
}

/// An exception raised while another is being handled records the outer one as
/// `__context__` even across that same native boundary.
#[test]
fn user_exception_identity_is_preserved_by_reraise() {
    let code = concat!(
        "class Halt(Exception):\n",
        "    pass\n",
        "err = Halt('once')\n",
        "try:\n",
        "    try:\n",
        "        raise err\n",
        "    except Halt:\n",
        "        raise\n",
        "except Halt as exc:\n",
        "    same = exc is err\n",
        "same\n",
    );
    assert_eq!(run(code).unwrap(), MontyObject::Bool(true));
}

/// `del obj[k]` dispatches `__delitem__`, and a class defining none refuses it
/// with one wording. CPython instead picks its message from the key's type
/// (`doesn't` for an integer index, `does not` otherwise), which the
/// comparative suite therefore cannot pin for the second half.
#[test]
fn delitem_refusal_has_one_wording() {
    let code = "class C:\n    def __init__(self):\n        self.hit = False\n\n    def __delitem__(self, k):\n        self.hit = True\nc = C()\ndel c[1]\nc.hit";
    assert_eq!(run(code).unwrap(), MontyObject::Bool(true));

    let err = run("class C:\n    pass\ndel C()['k']").unwrap_err();
    assert_eq!(err.exc_type(), ExcType::TypeError);
    assert_eq!(err.message(), Some("'C' object doesn't support item deletion"));
}
