//! The `__dataclass_fields__` mapping `@dataclass` writes and the `Field`
//! objects in it, where the behaviour cannot be dual-run against CPython:
//! Monty stringizes annotations, reveals no object addresses, and has no
//! `mappingproxy` or `_FIELD` sentinel.
//!
//! Everything the two interpreters agree on lives in
//! `test_cases/dataclass__field.py` and `test_cases/dataclass__is_dataclass.py`
//! instead.

use insta::assert_snapshot;
use monty::MontyRun;
use monty_types::{CompileOptions, MontyObject};

const POINT: &str = r"
from dataclasses import dataclass
import typing

@dataclass
class Point:
    x: int
    y: int = 5
    seen: typing.ClassVar[int] = 0
";

/// Runs `POINT` followed by `expr` and returns the string it evaluates to.
fn eval_str(expr: &str) -> String {
    let code = format!("{POINT}\n{expr}\n");
    let run = MontyRun::new(code, "test.py", vec![], CompileOptions::default()).expect("code should compile");
    match run.run_no_limits(vec![]).expect("code should run") {
        MontyObject::String(s) => s,
        other => panic!("expected a string, got {other:?}"),
    }
}

/// Runs `POINT` followed by `expr` and returns the exception message, falling
/// back to the rendered exception so a message-less failure is reported rather
/// than panicking with its type and traceback lost.
fn expect_error(expr: &str) -> String {
    let code = format!("{POINT}\n{expr}\n");
    let run = MontyRun::new(code, "test.py", vec![], CompileOptions::default()).expect("code should compile");
    match run.run_no_limits(vec![]) {
        Ok(value) => panic!("expected an exception, got {value:?}"),
        Err(err) => err.message().map_or_else(|| err.to_string(), ToOwned::to_owned),
    }
}

/// CPython's `Field.__repr__` attribute for attribute, bar the three spellings
/// Monty cannot produce: `type` is annotation text (CPython evaluates it to
/// `<class 'int'>`), `MISSING` carries no address, and `metadata` is a plain
/// dict where CPython wraps it in a `mappingproxy`.
#[test]
fn field_repr_matches_cpythons_layout() {
    assert_snapshot!(
        eval_str("repr(Point.__dataclass_fields__['y'])"),
        @"Field(name='y',type='int',default=5,default_factory=<dataclasses._MISSING_TYPE object>,init=True,repr=True,hash=None,compare=True,metadata={},kw_only=False,doc=None,_field_type=_FIELD)"
    );
    assert_snapshot!(
        eval_str("repr(Point.__dataclass_fields__['x'])"),
        @"Field(name='x',type='int',default=<dataclasses._MISSING_TYPE object>,default_factory=<dataclasses._MISSING_TYPE object>,init=True,repr=True,hash=None,compare=True,metadata={},kw_only=False,doc=None,_field_type=_FIELD)"
    );
}

/// A `field()` spec no class has claimed reports `None` for the two attributes
/// the decorator fills in, exactly as CPython's does.
#[test]
fn an_unclaimed_field_spec_has_no_name_or_type() {
    let code = r"
from dataclasses import field
f = field(default=1)
repr((f.name, f.type))
";
    let run =
        MontyRun::new(code.to_owned(), "test.py", vec![], CompileOptions::default()).expect("code should compile");
    let MontyObject::String(rendered) = run.run_no_limits(vec![]).expect("code should run") else {
        panic!("expected a string")
    };
    assert_snapshot!(rendered, @"(None, None)");
}

/// `_field_type` is CPython's internal `_FIELD` / `_FIELD_CLASSVAR` marker.
/// Monty has no field kinds at all (see `classvars_are_absent_from_the_mapping`
/// below), so it reports the gap rather than inventing a sentinel.
#[test]
fn field_type_marker_is_not_implemented() {
    assert_snapshot!(
        expect_error("Point.__dataclass_fields__['y']._field_type"),
        @"Field._field_type is not yet supported, dataclasses._FIELD is not implemented"
    );
}

/// CPython keeps `ClassVar` entries in `__dataclass_fields__` (marked
/// `_FIELD_CLASSVAR`) and filters them in `fields()`. Monty has no field kinds,
/// so the mapping *is* the field list and class variables never enter it.
#[test]
fn classvars_are_absent_from_the_mapping() {
    assert_snapshot!(eval_str("repr(list(Point.__dataclass_fields__))"), @"['x', 'y']");
}

/// A field's `default_factory` can be called, not only read.
///
/// `obj.name(...)` is `getattr(obj, "name")(...)`; the opcode that fuses the
/// two is an optimization, so a type that answers a name through attribute
/// lookup and holds no method of that name has to answer a call of it too.
#[test]
fn a_native_attribute_holding_a_callable_can_be_called() {
    let run = MontyRun::new(
        "\nfrom dataclasses import dataclass, field, fields\n\n@dataclass\nclass Holder:\n    xs: list = field(default_factory=list)\n\nf = fields(Holder)[0]\n(f.default_factory(), fields(Holder)[0].default_factory())\n"
            .to_owned(),
        "test.py",
        vec![],
        CompileOptions::default(),
    )
    .expect("code should compile");
    assert_eq!(
        run.run_no_limits(vec![]).expect("code should run"),
        MontyObject::Tuple(vec![MontyObject::List(vec![]), MontyObject::List(vec![])])
    );
}

/// A name the type answers with nothing still reports itself as absent,
/// rather than as something that is not callable.
#[test]
fn a_native_attribute_that_is_absent_is_still_an_attribute_error() {
    assert_eq!(
        expect_error("Point.__dataclass_fields__['x'].nonesuch()"),
        "'Field' object has no attribute 'nonesuch'"
    );
}
