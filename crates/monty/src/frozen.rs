//! Shared compilation and dispatch for internal Python implementations.
//!
//! Internal functions use ordinary VM frames and suspension but are hidden from
//! tracebacks and namespace introspection. Private native helpers arrive as
//! explicit arguments, never through user globals or importable modules.

use monty_types::CompileOptions;
use serde::{Deserialize, Serialize};

use crate::{
    annotations,
    args::ArgValues,
    builtins::{filter, map, min_max, next, zip},
    bytecode::{CallResult, Compiler, VM},
    exception_private::RunResult,
    intern::{CompileInterns, FunctionId, Interns},
    modules::{functools, typing},
    name_map::NameMap,
    parse::parse_with_interner,
    prepare::prepare_with_existing_names,
    sorting,
    types::{forward_ref, generic_alias, iter, list, lru_cache},
};

/// Identifies an internal implementation with one authoritative source definition.
/// Qualified names persist in dumps; the registry uses compact enum keys at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) enum FrozenFunction {
    /// Suspendable fold loop behind `functools.reduce`.
    #[serde(rename = "functools.reduce")]
    FunctoolsReduce,
    /// Validate the result of a suspendable user `__iter__` method.
    #[serde(rename = "builtins.iter")]
    Iter,
    /// Store a cached call's result only after it returns successfully.
    #[serde(rename = "functools._lru_cache_miss")]
    LruCacheMiss,
    /// Return a default when a suspendable user `__next__` method is exhausted.
    #[serde(rename = "builtins.next_default")]
    NextDefault,
    /// Call the function of `map()` on each item.
    #[serde(rename = "builtins.map")]
    Map,
    /// Call the predicate of `filter()` on each item, or test each item when it is `None`.
    #[serde(rename = "builtins.filter")]
    Filter,
    /// Call the key of `sorted()` or `list.sort()` on each item, then sort by the keys.
    #[serde(rename = "builtins.sort_by_key")]
    SortByKey,
    /// Call the key of `min()` or `max()` on each item and keep the item whose key wins.
    #[serde(rename = "builtins.min_max_key")]
    MinMaxKey,
    /// Step the iterables of `zip()` together, when one of them is a generator.
    #[serde(rename = "builtins.zip")]
    Zip,
    /// Evaluate annotations for `annotationlib.get_annotations` and `typing.get_type_hints`.
    #[serde(rename = "annotationlib.get_annotations")]
    GetAnnotations,
    /// `ForwardRef.evaluate()`, applying the format's rules to one evaluation.
    #[serde(rename = "annotationlib.ForwardRef.evaluate")]
    ForwardRefEvaluate,
    /// Evaluate and cache a class's `__annotations__` from its `__annotate__`.
    #[serde(rename = "type.__annotations__")]
    ClassAnnotations,
}

impl FrozenFunction {
    /// Prepares an unpublished implementation using this session's intern IDs.
    /// The caller publishes the overlay before installing its frame.
    pub(crate) fn compile(self, interns: &Interns) -> RunResult<(FunctionId, CompileInterns<'_>)> {
        let source = self.source();
        let mut overlay = CompileInterns::new(interns);
        let index = overlay.functions_len();
        let id = overlay.next_function_id();
        let mut globals = NameMap::new();
        let parsed =
            parse_with_interner(source, "<monty-frozen>", &mut overlay, false).map_err(|e| e.into_run_error(source))?;
        let nodes =
            prepare_with_existing_names(parsed.nodes, &overlay, &mut globals).map_err(|e| e.into_run_error(source))?;
        Compiler::compile_module(&nodes, &mut overlay, &globals, CompileOptions::default(), false)
            .map_err(|e| e.into_run_error(source))?;
        assert_eq!(
            overlay.functions_len(),
            index + 1,
            "frozen source must define one function"
        );
        assert_eq!(
            globals.len(),
            1,
            "frozen source must take its dependencies as arguments"
        );
        Ok((id, overlay))
    }

    /// Selects one self-contained function; dependencies arrive as arguments, not globals.
    fn source(self) -> &'static str {
        match self {
            Self::FunctoolsReduce => functools::REDUCE_SOURCE,
            Self::Iter => iter::ITER_SOURCE,
            Self::LruCacheMiss => lru_cache::CACHE_MISS_SOURCE,
            Self::NextDefault => next::NEXT_DEFAULT_SOURCE,
            Self::Map => map::MAP_SOURCE,
            Self::Filter => filter::FILTER_SOURCE,
            Self::SortByKey => sorting::SORT_BY_KEY_SOURCE,
            Self::MinMaxKey => min_max::MIN_MAX_KEY_SOURCE,
            Self::Zip => zip::ZIP_SOURCE,
            Self::GetAnnotations => typing::GET_ANNOTATIONS_SOURCE,
            Self::ForwardRefEvaluate => forward_ref::EVALUATE_SOURCE,
            Self::ClassAnnotations => annotations::CLASS_ANNOTATIONS_SOURCE,
        }
    }
}

/// Native primitives passed directly to internal Python functions.
/// These identities and their signatures are part of the captured bytecode's ABI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, strum::Display)]
#[strum(serialize_all = "snake_case")]
pub(crate) enum FrozenFunctions {
    /// Rebuild a generic alias around resolved arguments (see `generic_alias::call_alias_with`).
    AliasWith,
    /// Cache a class's evaluated `__annotations__` under `__annotations_cache__`.
    CacheClassAnnotations,
    /// Call an `__annotate__` function in an `annotationlib.Format` (see `annotations::call_annotate_helper`).
    CallAnnotate,
    /// Evaluate one annotation string in explicit namespaces and name-resolution mode.
    EvalHint,
    /// Call a user `__iter__` without validating its result yet.
    GetIter,
    /// Insert a successful result into a cache wrapper.
    Store,
    /// `annotationlib.type_repr`: a type by name, anything else by repr.
    TypeRepr,
    /// Check the result returned by a user `__iter__` method.
    ValidateIterator,
    /// Sort a private list in place by a list of its keys.
    OrderByKeys,
    /// Give the items of a sort back to the list that `list.sort()` detached.
    RestoreSorted,
    /// Whether a key of `min()` or `max()` wins over the best key so far.
    KeyWins,
}

/// Dispatches private helpers using the same argument ownership rules as public natives.
pub(crate) fn call_helper(function: FrozenFunctions, args: ArgValues, vm: &mut VM<'_>) -> RunResult<CallResult> {
    match function {
        FrozenFunctions::AliasWith => generic_alias::call_alias_with(args, vm).map(CallResult::Value),
        FrozenFunctions::CacheClassAnnotations => {
            annotations::cache_class_annotations_helper(args, vm).map(CallResult::Value)
        }
        FrozenFunctions::CallAnnotate => annotations::call_annotate_helper(args, vm),
        FrozenFunctions::EvalHint => typing::call_eval_hint(args, vm),
        FrozenFunctions::GetIter => iter::call_get_iter(args, vm),
        FrozenFunctions::Store => lru_cache::call_store(args, vm).map(CallResult::Value),
        FrozenFunctions::TypeRepr => typing::call_type_repr(args, vm).map(CallResult::Value),
        FrozenFunctions::ValidateIterator => iter::call_validate_iterator(args, vm).map(CallResult::Value),
        FrozenFunctions::OrderByKeys => sorting::call_order_by_keys(args, vm).map(CallResult::Value),
        FrozenFunctions::RestoreSorted => list::call_restore_sorted(args, vm).map(CallResult::Value),
        FrozenFunctions::KeyWins => min_max::call_key_wins(args, vm).map(CallResult::Value),
    }
}
