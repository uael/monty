//! Implementation of the `asyncio` module.
//!
//! Provides a minimal implementation of Python's `asyncio` module with:
//! - `run(coro)`: Runs a coroutine to completion, equivalent to `await coro`
//! - `gather(*awaitables)`: Collects coroutines for concurrent execution
//! - `sleep(delay, result=None)`: An awaitable governed by the session's `SleepMode`
//! - `create_task(coro)`, `_set_running_loop(loop)`, `_get_running_loop()`: a
//!   loop that a program sets, as CPython lets a loop of Python run
//!
//! Other asyncio functions (`wait`, etc.) are not implemented.
//! The host acts as the event loop - Monty yields control when tasks are blocked,
//! unless the program set a loop of its own: `gather` and `create_task` then ask
//! that loop for its tasks and futures, as CPython's do.

use std::mem;

use monty_types::{OsFunctionCall, sleep_duration_saturating};
use num_traits::ToPrimitive;

use crate::{
    args::{ArgValues, FromArgs, KwargsValues},
    asyncio::GatherFuture,
    builtins::Builtins,
    bytecode::{CallResult, VM},
    defer_drop, defer_drop_mut,
    exception_private::{ExcType, ExcTypeExt, RunError, RunResult, SimpleException},
    heap::{DropWithContext, Heap, HeapData, HeapId},
    heap_traits::DropGuard,
    intern::StaticStrings,
    modules::{
        ModuleFunctions,
        time::{HostSleep, host_sleep},
    },
    os_dispatch::PostConversionEffect,
    types::{
        EventLoop, List, Module, Partial, PyTrait,
        instance::{call_member_bound, class_defines, class_member, value_class},
    },
    value::Value,
};

/// Async Functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::Display, serde::Serialize, serde::Deserialize)]
#[strum(serialize_all = "lowercase")]
pub(crate) enum AsyncioFunctions {
    Gather,
    Run,
    Sleep,
    #[strum(serialize = "get_running_loop")]
    GetRunningLoop,
    #[strum(serialize = "current_task")]
    CurrentTask,
    #[strum(serialize = "create_task")]
    CreateTask,
    #[strum(serialize = "_set_running_loop")]
    SetRunningLoop,
    #[strum(serialize = "_get_running_loop")]
    GetRunningLoopOrNone,
    /// The callback that each child of a `gather` on a loop of a program calls
    /// when it is done; no module name binds it.
    #[strum(serialize = "_gather_done")]
    GatherDone,
}

/// Creates the `asyncio` module and allocates it on the heap.
///
/// The module contains the `run`, `gather`, `sleep`, `get_running_loop` and
/// `current_task` functions, and the `CancelledError` class. Other asyncio
/// names are not implemented as they would require additional VM/scheduler
/// features.
pub fn create_module(vm: &mut VM<'_>) -> HeapId {
    let mut module = Module::new(StaticStrings::Asyncio, vm.interns);

    module.set_attr(
        StaticStrings::Gather,
        Value::ModuleFunction(ModuleFunctions::Asyncio(AsyncioFunctions::Gather)),
        vm,
    );
    module.set_attr(
        StaticStrings::Run,
        Value::ModuleFunction(ModuleFunctions::Asyncio(AsyncioFunctions::Run)),
        vm,
    );
    module.set_attr(
        StaticStrings::Sleep,
        Value::ModuleFunction(ModuleFunctions::Asyncio(AsyncioFunctions::Sleep)),
        vm,
    );
    module.set_attr(
        StaticStrings::GetRunningLoop,
        Value::ModuleFunction(ModuleFunctions::Asyncio(AsyncioFunctions::GetRunningLoop)),
        vm,
    );
    module.set_attr(
        StaticStrings::CurrentTask,
        Value::ModuleFunction(ModuleFunctions::Asyncio(AsyncioFunctions::CurrentTask)),
        vm,
    );
    for (name, function) in [
        (StaticStrings::CreateTask, AsyncioFunctions::CreateTask),
        (StaticStrings::SetRunningLoop, AsyncioFunctions::SetRunningLoop),
        (
            StaticStrings::GetRunningLoopOrNone,
            AsyncioFunctions::GetRunningLoopOrNone,
        ),
    ] {
        module.set_attr(name, Value::ModuleFunction(ModuleFunctions::Asyncio(function)), vm);
    }
    // Nothing in Monty cancels an await, so this is here to be raised and
    // caught by sandboxed code; see `limitations/asyncio.md`.
    module.set_attr(
        StaticStrings::CancelledError,
        Value::Builtin(Builtins::ExcType(ExcType::CancelledError)),
        vm,
    );

    vm.heap.allocate(HeapData::Module(Box::new(module)))
}
pub(super) fn call(vm: &mut VM<'_>, functions: AsyncioFunctions, args: ArgValues) -> RunResult<CallResult> {
    match functions {
        AsyncioFunctions::Gather => gather(vm, args).map(CallResult::Value),
        AsyncioFunctions::Run => run(vm.heap, args),
        AsyncioFunctions::Sleep => sleep(vm, args),
        AsyncioFunctions::GetRunningLoop => get_running_loop(vm, args).map(CallResult::Value),
        AsyncioFunctions::CurrentTask => current_task(vm, args).map(CallResult::Value),
        AsyncioFunctions::CreateTask => create_task(vm, args),
        AsyncioFunctions::SetRunningLoop => set_running_loop(vm, args).map(CallResult::Value),
        AsyncioFunctions::GetRunningLoopOrNone => {
            args.check_zero_args("_get_running_loop", vm.heap)?;
            Ok(CallResult::Value(running_loop(vm).unwrap_or(Value::None)))
        }
        AsyncioFunctions::GatherDone => gather_done(vm, args).map(CallResult::Value),
    }
}

/// The running loop that a program set, with a reference of its own.
fn running_loop(vm: &mut VM<'_>) -> Option<Value> {
    let id = vm.sys_modules.running_loop()?;
    vm.heap.inc_ref(id);
    Some(Value::Ref(id))
}

/// `asyncio._set_running_loop(loop)`: the loop that `get_running_loop`,
/// `create_task` and `gather` ask from here on, or none for `None`, as CPython
/// sets the loop of its thread. A loop is an object of the program, which
/// answers `create_task`, `create_future` and, through its futures,
/// `add_done_callback`.
fn set_running_loop(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let running = args.get_one_arg("_set_running_loop", vm.heap)?;
    let id = match running {
        Value::None => None,
        Value::Ref(id) => Some(id),
        other => {
            let name = other.py_type_name(vm).into_owned();
            other.drop_with(vm);
            return Err(ExcType::type_error(format!(
                "a running loop is an object, not '{name}'"
            )));
        }
    };
    if let Some(old) = vm.sys_modules.set_running_loop(id) {
        Value::Ref(old).drop_with(vm);
    }
    Ok(Value::None)
}

/// `asyncio.create_task(coro)`: a task of the running loop, which the loop's
/// own `create_task` makes, as in CPython. With no loop that a program set,
/// Monty has no task to give.
fn create_task(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let Some(running) = running_loop(vm) else {
        args.drop_with(vm);
        return Err(SimpleException::new_msg(
            ExcType::RuntimeError,
            "create_task needs a running loop that the program set with asyncio._set_running_loop",
        )
        .into());
    };
    defer_drop!(running, vm);
    method(running, "create_task", args, vm).map(CallResult::Value)
}

/// What the method `name` of an object of the program gives, called at once.
fn method(target: &Value, name: &str, args: ArgValues, vm: &mut VM<'_>) -> RunResult<Value> {
    let member = match target {
        Value::Ref(id) => value_class(*id, vm).and_then(|class_id| class_member(class_id, name, vm)),
        _ => None,
    };
    let (Some(member), Value::Ref(id)) = (member, target) else {
        args.drop_with(vm);
        let type_name = target.py_type_name(vm).into_owned();
        return Err(ExcType::attribute_error(&type_name, name));
    };
    defer_drop!(member, vm);
    call_member_bound(member, *id, args, vm)?.require_synchronous(vm)
}

/// Whether an object of the program is a future, as CPython's `isfuture`
/// tells one: its class binds `_asyncio_future_blocking`.
fn is_future(value: &Value, vm: &VM<'_>) -> bool {
    matches!(value, Value::Ref(id) if value_class(*id, vm)
        .is_some_and(|class_id| class_defines(class_id, "_asyncio_future_blocking", vm)))
}

/// `gather(*awaitables)` on a loop of the program, as CPython's: each coroutine
/// becomes a task of the loop, each future stays itself, and the loop takes an
/// object with `__await__` as it is, where CPython wraps it in a coroutine
/// first, since Monty makes no coroutine of its own. The future that `gather` gives is the loop's
/// own, and is done with the list of the results in order once each child is,
/// or with the first exception, or the cancel of a child, that comes.
fn gather_on(running: &Value, awaitables: Vec<Value>, vm: &mut VM<'_>) -> RunResult<Value> {
    let mut children = Vec::with_capacity(awaitables.len());
    let mut rest = awaitables.into_iter();
    let made = (|| {
        for awaitable in rest.by_ref() {
            let child = if is_future(&awaitable, vm) {
                awaitable
            } else if matches!(&awaitable, Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::Generator(_))
                || value_class(*id, vm).is_some_and(|class_id| class_defines(class_id, "__await__", vm)))
            {
                method(running, "create_task", ArgValues::One(awaitable), vm)?
            } else {
                awaitable.drop_with(vm);
                return Err(ExcType::type_error(
                    "An asyncio.Future, a coroutine or an awaitable is required",
                ));
            };
            children.push(child);
        }
        Ok(())
    })();
    for left in rest {
        left.drop_with(vm);
    }
    if let Err(error) = made {
        children.drop_with(vm);
        return Err(error);
    }
    let outer = method(running, "create_future", ArgValues::Empty, vm);
    let outer = match outer {
        Ok(outer) => outer,
        Err(error) => {
            children.drop_with(vm);
            return Err(error);
        }
    };
    if children.is_empty() {
        let empty = Value::Ref(vm.heap.allocate(HeapData::List(List::new(Vec::new()))));
        method(&outer, "set_result", ArgValues::One(empty), vm)?.drop_with(vm);
        return Ok(outer);
    }
    let held: Vec<Value> = children.iter().map(|child| child.clone_with_heap(vm.heap)).collect();
    let held = Value::Ref(vm.heap.allocate(HeapData::List(List::new(held))));
    let done = Value::ModuleFunction(ModuleFunctions::Asyncio(AsyncioFunctions::GatherDone));
    let bound = ArgValues::ArgsKargs {
        args: vec![done, outer.clone_with_heap(vm.heap), held],
        kwargs: KwargsValues::Empty,
    };
    let callback = Partial::init(vm, bound)?;
    defer_drop!(callback, vm);
    for child in children {
        let added = method(
            &child,
            "add_done_callback",
            ArgValues::One(callback.clone_with_heap(vm.heap)),
            vm,
        );
        child.drop_with(vm);
        added?.drop_with(vm);
    }
    Ok(outer)
}

/// The callback of a child of a `gather` on a loop of the program, as CPython's:
/// the first exception or cancel of a child ends the outer future with it, and
/// once each child is done the outer future is done with their results.
fn gather_done(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let mut given = args.into_pos_only("_gather_done", vm.heap)?;
    let (Some(outer), Some(held), Some(child)) = (given.next(), given.next(), given.next()) else {
        return Err(ExcType::type_error(
            "_gather_done takes the outer future, the children and a child",
        ));
    };
    given.drop_with(vm);
    defer_drop!(outer, vm);
    defer_drop!(held, vm);
    defer_drop!(child, vm);
    let truth = |value: RunResult<Value>, vm: &mut VM<'_>| -> RunResult<bool> {
        let value = value?;
        let truth = value.py_bool(vm);
        value.drop_with(vm);
        truth
    };
    if truth(method(outer, "done", ArgValues::Empty, vm), vm)? {
        // CPython reads the exception of a child that ended after the outer
        // future, so it never says that nothing read it.
        if !truth(method(child, "cancelled", ArgValues::Empty, vm), vm)? {
            method(child, "exception", ArgValues::Empty, vm)?.drop_with(vm);
        }
        return Ok(Value::None);
    }
    let failed = if truth(method(child, "cancelled", ArgValues::Empty, vm), vm)? {
        Some(method(child, "_make_cancelled_error", ArgValues::Empty, vm)?)
    } else {
        let raised = method(child, "exception", ArgValues::Empty, vm)?;
        if matches!(raised, Value::None) {
            None
        } else {
            Some(raised)
        }
    };
    if let Some(failed) = failed {
        method(outer, "set_exception", ArgValues::One(failed), vm)?.drop_with(vm);
        return Ok(Value::None);
    }
    let Value::Ref(held_id) = held else {
        return Err(RunError::internal("gather children are a list"));
    };
    let HeapData::List(list) = vm.heap.get(*held_id) else {
        return Err(RunError::internal("gather children are a list"));
    };
    let children: Vec<Value> = list.as_slice().iter().map(|one| one.clone_with_heap(vm.heap)).collect();
    defer_drop!(children, vm);
    // The outer future waits for each child, and a child that failed ends it
    // through its own callback, as each failing child does in CPython.
    for one in children {
        if !truth(method(one, "done", ArgValues::Empty, vm), vm)?
            || truth(method(one, "cancelled", ArgValues::Empty, vm), vm)?
        {
            return Ok(Value::None);
        }
        let raised = method(one, "exception", ArgValues::Empty, vm)?;
        let failed = !matches!(raised, Value::None);
        raised.drop_with(vm);
        if failed {
            return Ok(Value::None);
        }
    }
    let mut results = Vec::with_capacity(children.len());
    for one in children {
        match method(one, "result", ArgValues::Empty, vm) {
            Ok(result) => results.push(result),
            Err(error) => {
                results.drop_with(vm);
                return Err(error);
            }
        }
    }
    let results = Value::Ref(vm.heap.allocate(HeapData::List(List::new(results))));
    method(outer, "set_result", ArgValues::One(results), vm)?.drop_with(vm);
    Ok(Value::None)
}

/// `asyncio.get_running_loop()`: proof that a loop is running, which here it
/// always is while sandbox code runs.
///
/// CPython raises `RuntimeError: no running event loop` outside one, and a
/// program that calls this to make sure it is inside a loop gets the same
/// answer. What comes back schedules nothing; see [`EventLoop`].
fn get_running_loop(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    args.check_zero_args("get_running_loop", vm.heap)?;
    if let Some(running) = running_loop(vm) {
        return Ok(running);
    }
    Ok(vm.heap.allocate_as(EventLoop).into_value())
}

/// `asyncio.current_task()`: `None`, since nothing here is a `Task`.
///
/// CPython answers `None` outside a task and a `Task` inside one. Monty
/// schedules coroutines without giving a program an object for one, so the
/// answer is always `None`; see `limitations/asyncio.md`.
fn current_task(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    args.check_zero_args("current_task", vm.heap)?;
    Ok(Value::None)
}

/// Returns an awaitable producing `result`, retained by [`PostConversionEffect::SleepResult`].
/// Unlike CPython, waiting starts at the call. The host answers with a future
/// or inline; [`host_sleep`] determines the destination and delay.
/// `Zero` and zero-length system sleeps settle immediately. See `limitations/asyncio.md`.
fn sleep(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let SleepArgs { delay, result } = SleepArgs::from_args(args, vm)?;
    // `result` outlives `delay`: it moves into the awaitable once the delay is valid.
    let mut result_guard = DropGuard::new(result, vm);
    let delay = {
        let (_, vm) = result_guard.as_parts_mut();
        defer_drop!(delay, vm);
        let seconds = delay_seconds(delay, vm)?;
        // NaN is the one delay CPython refuses; the rest clamp.
        let delay = sleep_duration_saturating(seconds)
            .map_err(|_| ExcType::value_error("Invalid delay: NaN (not a number)"))?;
        host_sleep(vm, delay)
    };
    let (result, vm) = result_guard.into_parts();
    let call = match delay {
        Some(HostSleep::CallHost(delay)) => OsFunctionCall::AsyncSleep(delay),
        // Avoid a host round trip for a zero-length system sleep.
        Some(HostSleep::System(delay)) if !delay.is_zero() => OsFunctionCall::AsyncSystemSleep(delay),
        _ => return Ok(CallResult::Value(vm.settled_awaitable(result))),
    };
    Ok(CallResult::OsCallWithEffect {
        call,
        effect: PostConversionEffect::SleepResult { result }.into(),
    })
}

/// `asyncio.sleep(delay, result=None)` — a pure-Python `def` in CPython, so
/// both parameters bind by keyword and neither is type-checked while binding.
#[derive(FromArgs)]
#[from_args(name = "sleep", style = def)]
struct SleepArgs {
    #[from_args(static_string = "Delay")]
    delay: Value,
    #[from_args(static_string = "ResultArg", default = Value::None)]
    result: Value,
}

/// Reads `delay` as float seconds.
///
/// CPython never converts it: it compares `delay <= 0` and hands whatever is
/// left to the loop, so only real numbers work — an `__index__`-able class is
/// rejected here although `time.sleep()` accepts it — and a non-number fails as
/// an unsupported comparison rather than as a bad argument.
fn delay_seconds(delay: &Value, vm: &VM<'_>) -> RunResult<f64> {
    match delay {
        Value::Float(f) => Ok(*f),
        Value::Int(n) => Ok(*n as f64),
        Value::Bool(b) => Ok(f64::from(*b)),
        _ => match delay.as_long_int(vm) {
            Some(n) => Ok(n.to_f64().unwrap_or(f64::INFINITY)),
            None => Err(ExcType::type_error_ordering("<=", &delay.py_type_name(vm), "int")),
        },
    }
}

/// Implementation of `asyncio.run(coro)`.
///
/// Runs a single coroutine to completion, equivalent to `await coro` at the top level.
/// Accepts exactly one positional argument (the coroutine) and no keyword arguments.
///
/// Returns `CallResult::AwaitValue` so the VM executes `exec_get_awaitable` on
/// the value, which handles validation that it's actually a coroutine/awaitable.
fn run(heap: &mut Heap, args: ArgValues) -> RunResult<CallResult> {
    let coroutine = args.get_one_arg("asyncio.run", heap)?;
    Ok(CallResult::AwaitValue(coroutine))
}

/// Implementation of `asyncio.gather(*awaitables)`.
///
/// Collects coroutines and external futures for concurrent execution. Does NOT
/// spawn tasks immediately - just validates and stores the references. Tasks are
/// spawned when the returned `GatherFuture` is awaited (in the `Await` opcode handler).
///
/// # Behavior when awaited
///
/// 1. Each coroutine is spawned as a separate Task
/// 2. External futures are tracked for resolution by the host
/// 3. The current task blocks until all items complete
/// 4. Results are collected in order and returned as a list
/// 5. On any task failure, sibling tasks are cancelled and the exception propagates
///
/// # Arguments
/// * `heap` - The heap for allocating the GatherFuture
/// * `args` - Variadic awaitable arguments (coroutines or external futures)
///
/// # Errors
/// Returns `TypeError` if any argument is not awaitable.
pub(crate) fn gather(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    // TODO: support keyword arguments (e.g. return_exceptions); for now any
    // kwarg is rejected up front by the macro's `kwargs_not_supported_yet`
    // flag with a `NotImplementedError: gather() does not yet support keyword
    // arguments` (CPython would have given a TypeError naming the bad kwarg).
    let GatherArgs { mut awaitables } = GatherArgs::from_args(args, vm)?;
    if let Some(running) = running_loop(vm) {
        defer_drop!(running, vm);
        return gather_on(running, mem::take(&mut awaitables), vm);
    }
    defer_drop_mut!(awaitables, vm);

    // Validate every argument before transferring any references to the gather,
    // so an invalid later argument needs no raw-HeapId rollback.
    for arg in awaitables.iter() {
        if !matches!(
            arg,
            Value::Ref(id)
                if matches!(
                    vm.heap.get(*id),
                    HeapData::Generator(_) | HeapData::ExternalFuture(_) | HeapData::GatherFuture(_)
                )
        ) {
            return Err(ExcType::type_error(
                "An asyncio.Future, a coroutine or an awaitable is required",
            ));
        }
    }

    let items = awaitables
        .drain(..)
        .map(|arg| arg.into_ref_id().expect("validated gather awaitable is heap-backed"))
        .collect();
    let gather_future = GatherFuture::new(items);
    let id = vm.heap.allocate(HeapData::GatherFuture(Box::new(gather_future)));
    Ok(Value::Ref(id))
}

/// `asyncio.gather(*awaitables)` — variadic positional, no kwargs accepted.
///
/// `kwargs_not_supported_yet` rejects any kwarg with the macro's
/// "not yet implemented" `NotImplementedError`, replacing the previous
/// `TypeError: gather() takes no keyword arguments` from `into_pos_only`.
/// When CPython's `return_exceptions=False` is wired up this becomes a
/// regular `kw_only` slot and the flag goes away.
#[derive(FromArgs)]
#[from_args(name = "gather", kwargs_not_supported_yet)]
struct GatherArgs {
    #[from_args(varargs)]
    awaitables: Vec<Value>,
}
