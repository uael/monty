//! Template string (PEP 750) AST types.
//!
//! A t-string (`t"a{x!r:>5}b"`) does *not* build a `str`. It builds a
//! `string.templatelib.Template` holding the literal segments verbatim and one
//! `Interpolation` per replacement field, so a consumer can inspect each
//! substituted expression before deciding how to render it. The runtime types
//! live in [`crate::types::template`]; this module is only the parse-time shape
//! the compiler emits from.

use crate::{
    expressions::ExprLoc,
    fstring::{ConversionFlag, FStringPart},
    intern::StringId,
};

/// A parsed t-string, normalized so the two vectors line up the way
/// `Template.strings` / `Template.interpolations` must at runtime.
///
/// **Invariant:** `strings.len() == interpolations.len() + 1`. CPython requires
/// it (a `Template` always has one more string than interpolation, empty
/// strings included), so the parser establishes it once here rather than
/// leaving the compiler to reconstruct it from an interleaved list.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ParsedTemplate {
    /// Literal segments, including the empty ones between adjacent
    /// interpolations and at the ends.
    pub strings: Vec<StringId>,
    /// One entry per replacement field, in source order.
    pub interpolations: Vec<TemplateInterpolation>,
}

/// One `{...}` replacement field of a t-string.
///
/// Everything except [`expr`](Self::expr) is *data about* the field rather than
/// something applied to it: a t-string never converts or formats, it hands both
/// to the consumer as `Interpolation.conversion` and `Interpolation.format_spec`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TemplateInterpolation {
    /// The expression to evaluate; its result becomes `Interpolation.value`.
    pub expr: Box<ExprLoc>,
    /// Source text of the expression, verbatim, as `Interpolation.expression`.
    pub expression: StringId,
    /// `!s` / `!r` / `!a`, surfaced as `'s'` / `'r'` / `'a'` or `None`.
    pub conversion: ConversionFlag,
    /// The format spec's parts, concatenated at runtime into
    /// `Interpolation.format_spec` (`''` when the field has no spec).
    ///
    /// Kept as f-string parts rather than a parsed spec because a t-string's
    /// spec is never *applied*: CPython stores the rendered text, so a nested
    /// interpolation (`t"{x:>{w}}"`) must be evaluated and concatenated, and a
    /// static spec must survive verbatim rather than being bit-packed.
    pub format_spec: Vec<FStringPart>,
}
