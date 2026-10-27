//! Implementation of the map() builtin function.

use crate::{
    args::{ArgValues, FromArgs},
    builtins::{Builtins, BuiltinsFunctions},
    bytecode::{CallResult, VM},
    exception_private::{ExcType, ExcTypeExt, RunResult},
    frozen::FrozenFunction,
    heap::DropWithContext,
    types::{Type, allocate_tuple, tuple::TupleVec},
    value::Value,
};

/// The loop of `map()`, which runs on the VM's own loop as a frame of its own.
///
/// So the function, the reads of the iterables and a generator among them all
/// step there, and a call to the host in any of them suspends the run. The
/// iterators are all made before the first call, as CPython makes them.
pub(crate) const MAP_SOURCE: &str = r"
def __monty_map(function, first, rest, iter_, next_, stop_iteration):
    out = []
    if not rest:
        for item in first:
            out.append(function(item))
        return out
    iterators = [iter_(first)]
    for iterable in rest:
        iterators.append(iter_(iterable))
    while True:
        args = []
        for iterator in iterators:
            try:
                args.append(next_(iterator))
            except stop_iteration:
                return out
        out.append(function(*args))
";

/// Implementation of the map() builtin function.
///
/// Applies a function to every item of one or more iterables and returns a list of results.
/// With multiple iterables, stops when the shortest iterable is exhausted.
/// The loop is [`MAP_SOURCE`], a frame on the VM's own loop, so the function may call the host.
///
/// Note: In Python this returns an iterator, but we return a list for simplicity.
/// Note: The `strict=` parameter is not yet supported.
///
/// Examples:
/// ```python
/// map(abs, [-1, 0, 1, 2])           # [1, 0, 1, 2]
/// map(pow, [2, 3], [3, 2])          # [8, 9]
/// map(str, [1, 2, 3])               # ['1', '2', '3']
/// ```
pub fn builtin_map(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    // CPython's map() uses a bespoke arity message
    // (`map() must have at least two arguments.`) rather than the generic
    // "missing N required positional arguments" wording the macro would
    // otherwise produce. Pre-check before delegating to MapArgs so we
    // match byte-for-byte; cleanup is handled by the early-return drop.
    //
    // Only fire the arity check when no kwargs are present — otherwise
    // `map(abs, bogus=1)` would report the arity error when CPython reports
    // the unknown-kwarg error. Delegating to the macro produces the
    // correct `got an unexpected keyword argument` message instead.
    let kwargs_empty = match &args {
        ArgValues::Kwargs(kwargs) => kwargs.is_empty(),
        ArgValues::ArgsKargs { kwargs, .. } => kwargs.is_empty(),
        _ => true,
    };
    if args.count() < 2 && kwargs_empty {
        args.drop_with(vm.heap);
        return Err(ExcType::type_error_map_arity());
    }
    let MapArgs {
        function,
        first_iterable,
        extra_iterables,
    } = MapArgs::from_args(args, vm)?;
    let rest = allocate_tuple(TupleVec::from_vec(extra_iterables), vm.heap);
    vm.call_frozen_exact(
        FrozenFunction::Map,
        [
            function,
            first_iterable,
            rest,
            Value::Builtin(Builtins::Type(Type::Iterator)),
            Value::Builtin(Builtins::Function(BuiltinsFunctions::Next)),
            Value::Builtin(Builtins::ExcType(ExcType::StopIteration)),
        ],
    )
}

/// Argument shape for `map(function, iterable, *iterables)`.
///
/// `function` and the first `iterable` are required; any further iterables
/// are collected by `extra_iterables`. `map` doesn't accept kwargs, so the
/// macro's default unknown-kwarg error path is exactly what we want.
#[derive(FromArgs)]
#[from_args(name = "map")]
struct MapArgs {
    #[from_args(pos_only)]
    function: Value,
    #[from_args(pos_only)]
    first_iterable: Value,
    #[from_args(varargs)]
    extra_iterables: Vec<Value>,
}
