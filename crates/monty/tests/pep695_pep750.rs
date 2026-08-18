//! Monty-specific behaviour of PEP 695 type aliases and PEP 750 template
//! strings.
//!
//! The parts that match CPython are covered by the dual-run fixtures
//! `test_cases/typealias__pep695.py` and `test_cases/tstring__all.py`. What
//! lives here is everything that *diverges*, and so cannot be asserted against
//! both engines:
//!
//! - `type(x).__name__` is Monty's module-qualified name, not CPython's bare one
//! - the type objects are not constructible, and the aliases are read-only
//! - PEP 695 type parameters parse but bind nothing
//! - `import string.templatelib` without an alias is rejected

use monty::MontyRun;
use monty_types::{CompileOptions, ExcType, MontyException, MontyObject};

/// Runs `code` to completion with no resource limits and returns the value of
/// its final expression.
fn eval(code: &str) -> MontyObject {
    MontyRun::new(code.to_owned(), "test.py", vec![], CompileOptions::default())
        .unwrap()
        .run_no_limits(vec![])
        .unwrap()
}

/// Runs `code` and returns the exception it raises, whether the failure came
/// from compilation or from execution.
fn run_err(code: &str) -> MontyException {
    match MontyRun::new(code.to_owned(), "test.py", vec![], CompileOptions::default()) {
        Err(err) => err,
        Ok(run) => run.run_no_limits(vec![]).expect_err("expected an exception"),
    }
}

/// Asserts `code` raises `exc_type` with exactly `message`.
fn assert_raises(code: &str, exc_type: ExcType, message: &str) {
    let err = run_err(code);
    assert_eq!(err.exc_type(), exc_type, "for {code:?}");
    assert_eq!(err.message().unwrap(), message, "for {code:?}");
}

// ---------------------------------------------------------------------------
// Type names are module-qualified
// ---------------------------------------------------------------------------

/// Monty names every non-builtin type by its module-qualified path (as it
/// already does for `re.Match` and `collections.deque`), so `__name__` on the
/// *type* diverges from CPython's bare name while `repr()` of the type and
/// every error message that names it agree with CPython.
#[test]
fn type_names_are_module_qualified() {
    for (code, expected) in [
        ("type X = int\ntype(X).__name__", "typing.TypeAliasType"),
        ("type(t'').__name__", "string.templatelib.Template"),
        (
            "type(t'{1}'.interpolations[0]).__name__",
            "string.templatelib.Interpolation",
        ),
    ] {
        assert_eq!(eval(code), MontyObject::String(expected.to_owned()), "for {code:?}");
    }
}

// ---------------------------------------------------------------------------
// PEP 695 aliases
// ---------------------------------------------------------------------------

/// Every attribute of an alias is read-only, and only `__name__`/`__value__`
/// exist. CPython says `readonly attribute` for the assignment and exposes
/// `__type_params__` and `__module__` as well.
#[test]
fn alias_attributes_are_read_only_and_minimal() {
    assert_raises(
        "type X = int\nX.__name__ = 'other'",
        ExcType::AttributeError,
        "'typing.TypeAliasType' object has no attribute '__name__' and no __dict__ for setting new attributes",
    );
    for attr in ["__module__", "__type_params__"] {
        assert_raises(
            &format!("type X = int\nX.{attr}"),
            ExcType::AttributeError,
            &format!("'typing.TypeAliasType' object has no attribute '{attr}'"),
        );
    }
}

/// CPython makes `X[int]` a `types.GenericAlias`; Monty has no generic-alias
/// machinery.
#[test]
fn alias_is_not_subscriptable() {
    assert_raises(
        "type X = int\nX[int]",
        ExcType::TypeError,
        "'typing.TypeAliasType' object is not subscriptable",
    );
}

/// The value is a deferred thunk, so an error inside it surfaces on the first
/// `__value__` read rather than at the `type` statement.
#[test]
fn alias_value_errors_surface_on_first_read() {
    assert_eq!(eval("type X = undefined_name\n1"), MontyObject::Int(1));
    assert_raises(
        "type X = undefined_name\nX.__value__",
        ExcType::NameError,
        "name 'undefined_name' is not defined",
    );
}

// ---------------------------------------------------------------------------
// PEP 695 type parameters bind nothing
// ---------------------------------------------------------------------------

/// CPython puts each type parameter in an implicit scope holding a `TypeVar`;
/// Monty drops them, so reading one is an ordinary unresolved name.
#[test]
fn type_parameters_do_not_bind() {
    assert_raises(
        "def f[T](x):\n    return T\nf(1)",
        ExcType::NameError,
        "name 'T' is not defined",
    );
}

/// The one case that yields a wrong value rather than an error: an outer
/// binding of the same name shows through where CPython would see the
/// `TypeVar`.
#[test]
fn type_parameter_is_shadowed_by_an_outer_binding() {
    assert_eq!(
        eval("T = 'outer'\ndef f[T](x):\n    return T\nf(1)"),
        MontyObject::String("outer".to_owned())
    );
}

/// Bounds and defaults are parsed and discarded, so a bound that would fail to
/// evaluate never raises.
#[test]
fn type_parameter_bounds_are_never_evaluated() {
    assert_eq!(
        eval("def f[T: undefined_bound](x):\n    return x\nf(1)"),
        MontyObject::Int(1)
    );
}

// ---------------------------------------------------------------------------
// PEP 750 templates
// ---------------------------------------------------------------------------

/// The type objects exist so `isinstance` and `type(...) is ...` work, but a
/// template can only come from a `t"..."` literal. CPython constructs both.
#[test]
fn template_types_are_not_constructible() {
    for (type_name, expr) in [
        ("Template", "Template('a')"),
        ("Interpolation", "Interpolation(1, 'x')"),
    ] {
        assert_raises(
            &format!("from string.templatelib import Interpolation, Template\n{expr}"),
            ExcType::TypeError,
            &format!("cannot create 'string.templatelib.{type_name}' instances"),
        );
    }
}

/// CPython supports `Template + Template` and `Template + str`.
#[test]
fn templates_do_not_concatenate() {
    assert_raises(
        "t'a' + t'b'",
        ExcType::TypeError,
        "unsupported operand type(s) for +: 'string.templatelib.Template' and 'string.templatelib.Template'",
    );
}

/// A template is immutable and has no `__dict__`, matching CPython's rejection
/// though not its wording.
#[test]
fn template_attributes_are_read_only() {
    assert_raises(
        "x = t'a'\nx.extra = 1",
        ExcType::AttributeError,
        "'string.templatelib.Template' object has no attribute 'extra' and no __dict__ for setting new attributes",
    );
}

/// `Interpolation.expression` is the source text between `{` and the field's
/// terminator: leading whitespace survives, trailing whitespace does not. This
/// matches CPython exactly, but cannot be pinned in `test_cases/tstring__all.py`
/// because `ruff format` normalises the interior spaces away.
#[test]
fn interpolation_expression_keeps_leading_whitespace() {
    for (code, expected) in [
        ("t'{ x }'.interpolations[0].expression", " x"),
        ("t'{ x + 1 }'.interpolations[0].expression", " x + 1"),
        ("t'{x !r}'.interpolations[0].expression", "x"),
        ("t'{ x :>5}'.interpolations[0].expression", " x"),
    ] {
        let code = format!("x = 42\n{code}");
        assert_eq!(eval(&code), MontyObject::String(expected.to_owned()), "for {code:?}");
    }
}

/// The `=` debug form keeps its whole source span, spacing included, in the
/// preceding literal segment.
#[test]
fn debug_interpolation_keeps_its_spacing() {
    assert_eq!(
        eval("x = 42\nt'{  x  = }'.strings[0]"),
        MontyObject::String("  x  = ".to_owned()),
    );
}

// ---------------------------------------------------------------------------
// Importing the module
// ---------------------------------------------------------------------------

/// The plain `import a.b` form binds the package `a`, as CPython does, so it
/// needs `a` to be a module Monty implements. `os.path` qualifies; `string`
/// does not, leaving `string.templatelib` reachable only by the two forms that
/// name the submodule directly.
#[test]
fn dotted_import_without_alias_is_rejected() {
    assert_raises(
        "import string.templatelib",
        ExcType::NotImplementedError,
        "importing a submodule of `string`, which is not itself implemented; use \
         `import string.templatelib as <name>` or `from string.templatelib import <name>`",
    );
}

/// The two forms that do work.
#[test]
fn dotted_import_with_alias_and_from_import_work() {
    assert_eq!(
        eval("import string.templatelib as tl\ntype(t'') is tl.Template"),
        MontyObject::Bool(true),
    );
    assert_eq!(
        eval("from string.templatelib import Template\ntype(t'') is Template"),
        MontyObject::Bool(true),
    );
}

// ---------------------------------------------------------------------------
// `del`
// ---------------------------------------------------------------------------

/// Monty implements neither slice assignment nor slice deletion, so `del`
/// reports the same `TypeError` the assignment form does. CPython deletes the
/// slice.
#[test]
fn slice_deletion_is_rejected() {
    assert_raises(
        "lst = [1, 2, 3]\ndel lst[0:2]",
        ExcType::TypeError,
        "list indices must be integers or slices, not slice",
    );
}

/// The module dunders are resolved on read rather than stored, so there is no
/// namespace entry for `del` to remove. CPython deletes the module-dict entry.
#[test]
fn module_dunders_cannot_be_deleted() {
    assert_raises("del __name__", ExcType::NameError, "name '__name__' is not defined");
}
