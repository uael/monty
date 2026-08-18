//! The `asyncio` module.
//!
//! The names are CPython's and so are the behaviours, because the surface is
//! the point: a program written against real `asyncio` has to run here
//! unchanged. What is missing, and where a behaviour differs, is listed in
//! `limitations/asyncio.md`.
//!
//! There is no loop object. The scheduler inside the VM *is* the loop, so
//! `get_event_loop` and its relatives have nothing to hand back; everything
//! those are normally used for (`create_task`, `call_later`, `create_future`)
//! is reachable as a module function instead.

use std::mem;

use ahash::AHashMap;

use crate::{
    args::{ArgValues, FromArgs},
    asyncio::{CombinatorKind, FutureKind, FutureState, ReturnWhen},
    builtins::Builtins,
    bytecode::{CallResult, Outcome, VM},
    defer_drop, defer_drop_mut,
    exception_private::{ExcType, ExcTypeExt, RunResult, SimpleException},
    heap::{DropWithContext, HeapData, HeapId, HeapReadOutput},
    intern::StaticStrings,
    modules::ModuleFunctions,
    types::{
        Module, PyTrait, Type,
        asyncio::{AsyncPrimitive, Event, TaskGroup, Timeout, allocate_primitive},
        collect_iterable,
        str::allocate_string,
    },
    value::Value,
};

/// The functions `asyncio` exposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::Display, serde::Serialize, serde::Deserialize)]
#[strum(serialize_all = "snake_case")]
pub(crate) enum AsyncioFunctions {
    Gather,
    Run,
    Sleep,
    EnsureFuture,
    CreateTask,
    Wait,
    AsCompleted,
    CurrentTask,
    IsCoroutine,
    IsFuture,
    Timeout,
    TimeoutAt,
}

/// Creates the `asyncio` module and allocates it on the heap.
///
/// # Panics
/// Panics if the required strings have not been pre-interned during prepare phase.
pub fn create_module(vm: &mut VM<'_>) -> HeapId {
    let mut module = Module::new(StaticStrings::Asyncio);

    for (name, function) in [
        (StaticStrings::Gather, AsyncioFunctions::Gather),
        (StaticStrings::Run, AsyncioFunctions::Run),
        (StaticStrings::Sleep, AsyncioFunctions::Sleep),
        (StaticStrings::EnsureFuture, AsyncioFunctions::EnsureFuture),
        (StaticStrings::CreateTask, AsyncioFunctions::CreateTask),
        (StaticStrings::Wait, AsyncioFunctions::Wait),
        (StaticStrings::AsCompleted, AsyncioFunctions::AsCompleted),
        (StaticStrings::CurrentTask, AsyncioFunctions::CurrentTask),
        (StaticStrings::Iscoroutine, AsyncioFunctions::IsCoroutine),
        (StaticStrings::Isfuture, AsyncioFunctions::IsFuture),
        (StaticStrings::Timeout, AsyncioFunctions::Timeout),
        (StaticStrings::TimeoutAt, AsyncioFunctions::TimeoutAt),
    ] {
        module.set_attr(name, Value::ModuleFunction(ModuleFunctions::Asyncio(function)), vm);
    }

    for (name, type_) in [
        (StaticStrings::FutureClass, Type::Future),
        (StaticStrings::TaskClass, Type::Task),
        (StaticStrings::LockClass, Type::Lock),
        (StaticStrings::EventClass, Type::Event),
        (StaticStrings::SemaphoreClass, Type::Semaphore),
        (StaticStrings::BoundedSemaphoreClass, Type::BoundedSemaphore),
        (StaticStrings::BarrierClass, Type::Barrier),
        (StaticStrings::QueueClass, Type::Queue),
        (StaticStrings::TaskGroupClass, Type::TaskGroup),
    ] {
        module.set_attr(name, Value::Builtin(Builtins::Type(type_)), vm);
    }

    for (name, exc) in [
        (StaticStrings::CancelledErrorClass, ExcType::CancelledError),
        (StaticStrings::InvalidStateErrorClass, ExcType::InvalidStateError),
        (StaticStrings::QueueEmptyClass, ExcType::QueueEmpty),
        (StaticStrings::QueueFullClass, ExcType::QueueFull),
        // `asyncio.TimeoutError` has been the builtin `TimeoutError` since 3.11.
        (StaticStrings::TimeoutErrorClass, ExcType::TimeoutError),
    ] {
        module.set_attr(name, Value::Builtin(Builtins::ExcType(exc)), vm);
    }

    for (name, flag) in [
        (StaticStrings::FirstCompleted, "FIRST_COMPLETED"),
        (StaticStrings::FirstException, "FIRST_EXCEPTION"),
        (StaticStrings::AllCompleted, "ALL_COMPLETED"),
    ] {
        let value = allocate_string(flag.to_owned(), vm.heap);
        module.set_attr(name, value, vm);
    }

    vm.heap.allocate(HeapData::Module(Box::new(module)))
}

pub(super) fn call(vm: &mut VM<'_>, functions: AsyncioFunctions, args: ArgValues) -> RunResult<CallResult> {
    match functions {
        AsyncioFunctions::Gather => gather(vm, args).map(CallResult::Value),
        AsyncioFunctions::Run => run(vm, args),
        AsyncioFunctions::Sleep => sleep(vm, args).map(CallResult::Value),
        AsyncioFunctions::EnsureFuture | AsyncioFunctions::CreateTask => ensure_future(vm, args).map(CallResult::Value),
        AsyncioFunctions::Wait => wait(vm, args).map(CallResult::Value),
        AsyncioFunctions::AsCompleted => as_completed(vm, args).map(CallResult::Value),
        AsyncioFunctions::CurrentTask => current_task(vm, args).map(CallResult::Value),
        AsyncioFunctions::IsCoroutine => is_kind(vm, args, "iscoroutine").map(CallResult::Value),
        AsyncioFunctions::IsFuture => is_kind(vm, args, "isfuture").map(CallResult::Value),
        AsyncioFunctions::Timeout => timeout(vm, args, false).map(CallResult::Value),
        AsyncioFunctions::TimeoutAt => timeout(vm, args, true).map(CallResult::Value),
    }
}

/// `asyncio.run(coro)`.
///
/// Awaits the coroutine on the task that called it rather than starting a loop
/// of its own, since the scheduler is already running. Everything a running
/// loop provides works inside it.
fn run(vm: &mut VM<'_>, args: ArgValues) -> RunResult<CallResult> {
    let coroutine = args.get_one_arg("asyncio.run", vm.heap)?;
    Ok(CallResult::AwaitValue(coroutine))
}

/// Turns one awaitable into a future: a coroutine becomes a task, a future is
/// already one. The caller owns the returned reference.
fn as_future(value: &Value, vm: &mut VM<'_>) -> RunResult<HeapId> {
    match value {
        Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::Future(_)) => {
            vm.heap.inc_ref(*id);
            Ok(*id)
        }
        Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::Coroutine(_)) => {
            let task = vm.spawn_task(*id)?;
            task.into_ref_id()
                .ok_or_else(|| ExcType::type_error("spawn_task returned a non-reference"))
        }
        _ => Err(ExcType::type_error(
            "An asyncio.Future, a coroutine or an awaitable is required",
        )),
    }
}

/// Turns a list of awaitables into futures, giving the same awaitable the same
/// future however many times it appears.
///
/// CPython's `gather` keys its children by the argument object for exactly this
/// reason: `gather(coro, coro)` runs the coroutine once and fills both slots.
fn as_futures(values: &[Value], vm: &mut VM<'_>) -> RunResult<Vec<HeapId>> {
    let mut seen: AHashMap<HeapId, HeapId> = AHashMap::new();
    let mut futures = Vec::with_capacity(values.len());
    for value in values {
        let already = match value {
            Value::Ref(id) => seen.get(id).copied(),
            _ => None,
        };
        let future = match already {
            Some(future) => {
                vm.heap.inc_ref(future);
                Ok(future)
            }
            None => as_future(value, vm),
        };
        match future {
            Ok(future) => {
                if let Value::Ref(id) = value {
                    seen.insert(*id, future);
                }
                futures.push(future);
            }
            Err(error) => {
                for future in futures {
                    vm.heap.dec_ref(future);
                }
                return Err(error);
            }
        }
    }
    Ok(futures)
}

/// `asyncio.ensure_future(aw)`, which `create_task` shares.
///
/// `create_task` refuses anything but a coroutine in CPython; both are accepted
/// here, which only widens what is allowed.
fn ensure_future(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let value = args.get_one_arg("ensure_future", vm.heap)?;
    defer_drop!(value, vm);
    Ok(Value::Ref(as_future(value, vm)?))
}

/// `asyncio.gather(*aws, return_exceptions=False)`.
pub(crate) fn gather(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let GatherArgs {
        awaitables,
        return_exceptions,
    } = GatherArgs::from_args(args, vm)?;
    defer_drop_mut!(awaitables, vm);
    let return_exceptions = match return_exceptions {
        Some(flag) => {
            let truth = flag.py_bool(vm)?;
            flag.drop_with(vm);
            truth
        }
        None => false,
    };

    let children = as_futures(awaitables, vm)?;
    let output = vm.start_combinator(CombinatorKind::Gather { return_exceptions }, children);
    Ok(Value::Ref(output))
}

/// `asyncio.gather(*aws, return_exceptions=False)`, whose only keyword is the
/// one CPython accepts.
#[derive(FromArgs)]
#[from_args(name = "gather")]
struct GatherArgs {
    #[from_args(varargs)]
    awaitables: Vec<Value>,
    #[from_args(kw_only, default, static_string = "ReturnExceptions")]
    return_exceptions: Option<Value>,
}

/// `asyncio.sleep(delay, result=None)`.
///
/// The delay is measured on the scheduler's clock, which only moves when
/// nothing else can run, so two runs of the same program sleep through exactly
/// the same interleaving.
fn sleep(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let SleepArgs { delay, result } = SleepArgs::from_args(args, vm)?;
    let seconds = value_as_seconds(&delay, vm, "sleep")?;
    delay.drop_with(vm);
    let result = result.unwrap_or(Value::None);
    let future = vm.arm_timer(seconds_to_nanos(seconds), result, None);
    Ok(Value::Ref(future))
}

/// `asyncio.sleep(delay, result=None)`.
#[derive(FromArgs)]
#[from_args(name = "sleep", style = def)]
struct SleepArgs {
    delay: Value,
    #[from_args(default, static_string = "Result")]
    result: Option<Value>,
}

/// `asyncio.wait(fs, *, timeout=None, return_when=ALL_COMPLETED)`.
///
/// `timeout` is rejected rather than ignored: silently waiting forever is the
/// kind of divergence that is impossible to debug from inside the sandbox.
fn wait(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let WaitArgs {
        fs,
        timeout,
        return_when,
    } = WaitArgs::from_args(args, vm)?;
    defer_drop!(fs, vm);
    if let Some(timeout) = timeout {
        let given = !matches!(timeout, Value::None);
        timeout.drop_with(vm);
        if given {
            return_when.drop_with(vm);
            return Err(ExcType::not_implemented(
                "asyncio.wait() does not support the timeout argument; use asyncio.timeout()",
            )
            .into());
        }
    }
    let return_when = match return_when {
        Some(value) => {
            let name = value.py_str(vm)?;
            let rendered = name.to_str(vm)?.to_owned();
            name.drop_with(vm);
            value.drop_with(vm);
            match rendered.as_str() {
                "FIRST_COMPLETED" => ReturnWhen::FirstCompleted,
                "FIRST_EXCEPTION" => ReturnWhen::FirstException,
                "ALL_COMPLETED" => ReturnWhen::AllCompleted,
                other => return Err(ExcType::value_error(format!("Invalid return_when value: {other}"))),
            }
        }
        None => ReturnWhen::AllCompleted,
    };

    let items = collect_iterable(fs, vm)?;
    let children = match as_futures(&items, vm) {
        Ok(children) => children,
        Err(error) => {
            items.drop_with(vm);
            return Err(error);
        }
    };
    items.drop_with(vm);
    if children.is_empty() {
        return Err(ExcType::value_error("Set of Tasks/Futures is empty."));
    }
    let output = vm.start_combinator(CombinatorKind::Wait { return_when }, children);
    Ok(Value::Ref(output))
}

/// `asyncio.wait(fs, *, timeout=None, return_when=ALL_COMPLETED)`.
#[derive(FromArgs)]
#[from_args(name = "wait", style = def)]
struct WaitArgs {
    fs: Value,
    #[from_args(kw_only, default, static_string = "Timeout")]
    timeout: Option<Value>,
    #[from_args(kw_only, default, static_string = "ReturnWhen")]
    return_when: Option<Value>,
}

/// `asyncio.as_completed(fs)`.
///
/// Returns the async iterator form (`async for fut in as_completed(fs)`), which
/// hands each awaitable back as it settles.
fn as_completed(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let fs = args.get_one_arg("as_completed", vm.heap)?;
    defer_drop!(fs, vm);
    let items = collect_iterable(fs, vm)?;
    let children = match as_futures(&items, vm) {
        Ok(children) => children,
        Err(error) => {
            items.drop_with(vm);
            return Err(error);
        }
    };
    items.drop_with(vm);
    Ok(Value::Ref(vm.start_as_completed(children)))
}

/// `asyncio.current_task()`, which is `None` on module-level code since that is
/// not a task.
fn current_task(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    args.check_zero_args("current_task", vm.heap)?;
    Ok(vm.current_task_future())
}

/// `asyncio.iscoroutine(obj)` and `asyncio.isfuture(obj)`.
fn is_kind(vm: &mut VM<'_>, args: ArgValues, name: &'static str) -> RunResult<Value> {
    let value = args.get_one_arg(name, vm.heap)?;
    let answer = matches!(value, Value::Ref(id) if match vm.heap.get(id) {
        HeapData::Coroutine(_) => name == "iscoroutine",
        HeapData::Future(_) => name == "isfuture",
        _ => false,
    });
    value.drop_with(vm);
    Ok(Value::Bool(answer))
}

/// `asyncio.timeout(delay)` and `asyncio.timeout_at(when)`.
fn timeout(vm: &mut VM<'_>, args: ArgValues, absolute: bool) -> RunResult<Value> {
    let name = if absolute { "timeout_at" } else { "timeout" };
    let value = args.get_one_arg(name, vm.heap)?;
    let deadline = if matches!(value, Value::None) {
        None
    } else {
        let seconds = value_as_seconds(&value, vm, name)?;
        let nanos = seconds_to_nanos(seconds);
        Some(if absolute { nanos } else { vm.scheduler_now() + nanos })
    };
    value.drop_with(vm);
    Ok(allocate_primitive(
        AsyncPrimitive::Timeout(Timeout {
            deadline,
            timer: None,
            expired: false,
        }),
        vm,
    ))
}

/// Reads a `float`-ish delay, the way `asyncio` accepts `int` and `float`.
fn value_as_seconds(value: &Value, vm: &mut VM<'_>, name: &str) -> RunResult<f64> {
    match value {
        Value::Int(seconds) => Ok(*seconds as f64),
        Value::Float(seconds) => Ok(*seconds),
        Value::Bool(flag) => Ok(f64::from(u8::from(*flag))),
        other => {
            let type_name = other.py_type_name(vm).into_owned();
            Err(ExcType::type_error(format!(
                "{name}() argument must be a number, not {type_name}"
            )))
        }
    }
}

/// Seconds to the scheduler's nanoseconds, clamping anything at or below zero
/// to "as soon as nothing else can run".
fn seconds_to_nanos(seconds: f64) -> u64 {
    if seconds.is_nan() || seconds <= 0.0 {
        return 0;
    }
    // Saturating rather than wrapping: a delay past the counter's range is a
    // deadline no run reaches, which is what an unreachably long sleep means.
    let nanos = seconds * 1e9;
    if nanos >= 1.844_674_407_370_955e19 {
        u64::MAX
    } else {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "bounded above and below by the two checks here"
        )]
        {
            nanos as u64
        }
    }
}

/// Every method the coordination primitives answer.
///
/// One dispatcher rather than one per type: they share a heap variant, and the
/// method names barely overlap, so a single match reads as the whole surface.
pub(crate) fn call_primitive_method(
    self_id: HeapId,
    method: &str,
    vm: &mut VM<'_>,
    args: ArgValues,
) -> RunResult<Value> {
    let kind = match vm.heap.get(self_id) {
        HeapData::AsyncPrimitive(primitive) => primitive.py_type(),
        other => panic!("call_primitive_method on {}", other.py_type()),
    };
    match (kind, method) {
        // --- Lock ---
        (Type::Lock, "acquire" | "__aenter__") => {
            args.check_zero_args(method, vm.heap)?;
            Ok(lock_acquire(self_id, vm))
        }
        (Type::Lock, "release") => {
            args.check_zero_args("release", vm.heap)?;
            lock_release(self_id, vm)?;
            Ok(Value::None)
        }
        (Type::Lock, "__aexit__") => {
            args.drop_with(vm);
            lock_release(self_id, vm)?;
            Ok(vm.settled_future(Outcome::Value(Value::None)))
        }
        (Type::Lock, "locked") => {
            args.check_zero_args("locked", vm.heap)?;
            let HeapData::AsyncPrimitive(primitive) = vm.heap.get(self_id) else {
                unreachable!("checked above")
            };
            let AsyncPrimitive::Lock(lock) = primitive.as_ref() else {
                unreachable!("checked above")
            };
            Ok(Value::Bool(lock.locked))
        }

        // --- Event ---
        (Type::Event, "set") => {
            args.check_zero_args("set", vm.heap)?;
            event_set(self_id, vm);
            Ok(Value::None)
        }
        (Type::Event, "clear") => {
            args.check_zero_args("clear", vm.heap)?;
            with_event(self_id, vm, |event| event.is_set = false);
            Ok(Value::None)
        }
        (Type::Event, "is_set") => {
            args.check_zero_args("is_set", vm.heap)?;
            let mut answer = false;
            with_event(self_id, vm, |event| answer = event.is_set);
            Ok(Value::Bool(answer))
        }
        (Type::Event, "wait") => {
            args.check_zero_args("wait", vm.heap)?;
            Ok(event_wait(self_id, vm))
        }

        // --- Semaphore ---
        (Type::Semaphore | Type::BoundedSemaphore, "acquire" | "__aenter__") => {
            args.check_zero_args(method, vm.heap)?;
            Ok(semaphore_acquire(self_id, vm))
        }
        (Type::Semaphore | Type::BoundedSemaphore, "release") => {
            args.check_zero_args("release", vm.heap)?;
            semaphore_release(self_id, vm)?;
            Ok(Value::None)
        }
        (Type::Semaphore | Type::BoundedSemaphore, "__aexit__") => {
            args.drop_with(vm);
            semaphore_release(self_id, vm)?;
            Ok(vm.settled_future(Outcome::Value(Value::None)))
        }
        (Type::Semaphore | Type::BoundedSemaphore, "locked") => {
            args.check_zero_args("locked", vm.heap)?;
            let HeapData::AsyncPrimitive(primitive) = vm.heap.get(self_id) else {
                unreachable!("checked above")
            };
            let AsyncPrimitive::Semaphore(semaphore) = primitive.as_ref() else {
                unreachable!("checked above")
            };
            Ok(Value::Bool(semaphore.value == 0 || !semaphore.waiters.is_empty()))
        }

        // --- Barrier ---
        (Type::Barrier, "wait" | "__aenter__") => {
            args.check_zero_args(method, vm.heap)?;
            Ok(barrier_wait(self_id, vm))
        }
        (Type::Barrier, "__aexit__") => {
            args.drop_with(vm);
            Ok(vm.settled_future(Outcome::Value(Value::None)))
        }

        // --- Queue ---
        (Type::Queue, "qsize") => {
            args.check_zero_args("qsize", vm.heap)?;
            Ok(Value::Int(queue_len(self_id, vm)))
        }
        (Type::Queue, "empty") => {
            args.check_zero_args("empty", vm.heap)?;
            Ok(Value::Bool(queue_len(self_id, vm) == 0))
        }
        (Type::Queue, "full") => {
            args.check_zero_args("full", vm.heap)?;
            Ok(Value::Bool(queue_is_full(self_id, vm)))
        }
        (Type::Queue, "put_nowait") => {
            let Some(item) = args.get_zero_one_arg("put_nowait", vm.heap)? else {
                return Err(ExcType::type_error(
                    "Queue.put_nowait() takes exactly one argument (0 given)",
                ));
            };
            queue_put_nowait(self_id, item, vm)?;
            Ok(Value::None)
        }
        (Type::Queue, "put") => {
            let Some(item) = args.get_zero_one_arg("put", vm.heap)? else {
                return Err(ExcType::type_error("Queue.put() takes exactly one argument (0 given)"));
            };
            queue_put(self_id, item, vm)
        }
        (Type::Queue, "get_nowait") => {
            args.check_zero_args("get_nowait", vm.heap)?;
            queue_get_nowait(self_id, vm)
        }
        (Type::Queue, "get") => {
            args.check_zero_args("get", vm.heap)?;
            Ok(queue_get(self_id, vm))
        }
        (Type::Queue, "task_done") => {
            args.check_zero_args("task_done", vm.heap)?;
            queue_task_done(self_id, vm)?;
            Ok(Value::None)
        }
        (Type::Queue, "join") => {
            args.check_zero_args("join", vm.heap)?;
            Ok(queue_join(self_id, vm))
        }

        // --- TaskGroup ---
        (Type::TaskGroup, "__aenter__") => {
            args.check_zero_args("__aenter__", vm.heap)?;
            with_taskgroup(self_id, vm, |group| group.entered = true);
            vm.heap.inc_ref(self_id);
            Ok(vm.settled_future(Outcome::Value(Value::Ref(self_id))))
        }
        (Type::TaskGroup, "create_task") => {
            let Some(coroutine) = args.get_zero_one_arg("create_task", vm.heap)? else {
                return Err(ExcType::type_error(
                    "TaskGroup.create_task() takes exactly one argument (0 given)",
                ));
            };
            defer_drop!(coroutine, vm);
            taskgroup_create_task(self_id, coroutine, vm)
        }
        (Type::TaskGroup, "__aexit__") => Ok(taskgroup_aexit(self_id, args, vm)),

        // --- Timeout ---
        (Type::Timeout, "__aenter__") => {
            args.check_zero_args("__aenter__", vm.heap)?;
            timeout_enter(self_id, vm)
        }
        (Type::Timeout, "__aexit__") => Ok(timeout_exit(self_id, args, vm)),

        _ => {
            let method = method.to_owned();
            args.drop_with(vm);
            Err(ExcType::attribute_error(kind, &method))
        }
    }
}

/// Runs `f` on the primitive at `id`, which the caller has already established
/// is of the matching kind.
macro_rules! with_primitive {
    ($id:expr, $vm:expr, $variant:ident, |$state:ident| $body:expr) => {{
        let HeapReadOutput::AsyncPrimitive(mut primitive) = $vm.heap.read($id) else {
            unreachable!("not an asyncio primitive")
        };
        let AsyncPrimitive::$variant($state) = primitive.get_mut($vm.heap) else {
            unreachable!("wrong asyncio primitive")
        };
        let result = $body;
        drop(primitive);
        result
    }};
}

/// `Lock.acquire()`, which is also `__aenter__`.
fn lock_acquire(id: HeapId, vm: &mut VM<'_>) -> Value {
    let free = with_primitive!(id, vm, Lock, |lock| {
        if lock.locked || !lock.waiters.is_empty() {
            false
        } else {
            lock.locked = true;
            true
        }
    });
    if free {
        return vm.settled_future(Outcome::Value(Value::Bool(true)));
    }
    let future = vm.alloc_future(FutureKind::Future, None);
    vm.heap.inc_ref(future);
    with_primitive!(id, vm, Lock, |lock| lock.waiters.push_back(future));
    Value::Ref(future)
}

/// `Lock.release()`, which hands the lock to the next waiter rather than
/// letting it go free, so a queued acquirer cannot be overtaken.
fn lock_release(id: HeapId, vm: &mut VM<'_>) -> RunResult<()> {
    let held = with_primitive!(id, vm, Lock, |lock| lock.locked);
    if !held {
        return Err(SimpleException::new_msg(ExcType::RuntimeError, "Lock is not acquired.").into());
    }
    loop {
        let next = with_primitive!(id, vm, Lock, |lock| lock.waiters.pop_front());
        let Some(future) = next else {
            with_primitive!(id, vm, Lock, |lock| lock.locked = false);
            return Ok(());
        };
        // A settled waiter was cancelled and never took the lock; try the next.
        let taken = !future_is_settled(future, vm);
        if taken {
            vm.settle_future(future, Outcome::Value(Value::Bool(true)));
        }
        vm.heap.dec_ref(future);
        if taken {
            return Ok(());
        }
    }
}

/// Runs `f` on the `Event` at `id`.
fn with_event(id: HeapId, vm: &mut VM<'_>, f: impl FnOnce(&mut Event)) {
    with_primitive!(id, vm, Event, |event| f(event));
}

/// `Event.set()`: raises the flag and settles every waiter.
fn event_set(id: HeapId, vm: &mut VM<'_>) {
    let waiters = with_primitive!(id, vm, Event, |event| {
        event.is_set = true;
        mem::take(&mut event.waiters)
    });
    for future in waiters {
        vm.settle_future(future, Outcome::Value(Value::Bool(true)));
        vm.heap.dec_ref(future);
    }
}

/// `Event.wait()`.
fn event_wait(id: HeapId, vm: &mut VM<'_>) -> Value {
    if with_primitive!(id, vm, Event, |event| event.is_set) {
        return vm.settled_future(Outcome::Value(Value::Bool(true)));
    }
    let future = vm.alloc_future(FutureKind::Future, None);
    vm.heap.inc_ref(future);
    with_primitive!(id, vm, Event, |event| event.waiters.push_back(future));
    Value::Ref(future)
}

/// `Semaphore.acquire()`, which is also `__aenter__`.
fn semaphore_acquire(id: HeapId, vm: &mut VM<'_>) -> Value {
    let taken = with_primitive!(id, vm, Semaphore, |semaphore| {
        if semaphore.value > 0 && semaphore.waiters.is_empty() {
            semaphore.value -= 1;
            true
        } else {
            false
        }
    });
    if taken {
        return vm.settled_future(Outcome::Value(Value::Bool(true)));
    }
    let future = vm.alloc_future(FutureKind::Future, None);
    vm.heap.inc_ref(future);
    with_primitive!(id, vm, Semaphore, |semaphore| semaphore.waiters.push_back(future));
    Value::Ref(future)
}

/// `Semaphore.release()`: the permit goes straight to the next waiter when
/// there is one, and back into the count when there is not.
fn semaphore_release(id: HeapId, vm: &mut VM<'_>) -> RunResult<()> {
    let over_bound = with_primitive!(id, vm, Semaphore, |semaphore| {
        semaphore.bound.is_some_and(|bound| semaphore.value >= bound)
    });
    if over_bound {
        return Err(ExcType::value_error("BoundedSemaphore released too many times"));
    }
    loop {
        let next = with_primitive!(id, vm, Semaphore, |semaphore| semaphore.waiters.pop_front());
        let Some(future) = next else {
            with_primitive!(id, vm, Semaphore, |semaphore| semaphore.value += 1);
            return Ok(());
        };
        let taken = !future_is_settled(future, vm);
        if taken {
            vm.settle_future(future, Outcome::Value(Value::Bool(true)));
        }
        vm.heap.dec_ref(future);
        if taken {
            return Ok(());
        }
    }
}

/// `Barrier.wait()`: settles every party's future with its arrival index once
/// the last one arrives.
fn barrier_wait(id: HeapId, vm: &mut VM<'_>) -> Value {
    let future = vm.alloc_future(FutureKind::Future, None);
    vm.heap.inc_ref(future);
    let full = with_primitive!(id, vm, Barrier, |barrier| {
        barrier.waiters.push_back(future);
        i64::try_from(barrier.waiters.len()).unwrap_or(i64::MAX) >= barrier.parties
    });
    if full {
        let waiters = with_primitive!(id, vm, Barrier, |barrier| mem::take(&mut barrier.waiters));
        for (index, waiting) in waiters.into_iter().enumerate() {
            vm.settle_future(
                waiting,
                Outcome::Value(Value::Int(i64::try_from(index).unwrap_or(i64::MAX))),
            );
            vm.heap.dec_ref(waiting);
        }
    }
    Value::Ref(future)
}

/// The number of items sitting in the queue.
fn queue_len(id: HeapId, vm: &mut VM<'_>) -> i64 {
    with_primitive!(id, vm, Queue, |queue| i64::try_from(queue.items.len())
        .unwrap_or(i64::MAX))
}

/// Whether a bounded queue has no room left.
fn queue_is_full(id: HeapId, vm: &mut VM<'_>) -> bool {
    with_primitive!(id, vm, Queue, |queue| {
        queue.maxsize > 0 && i64::try_from(queue.items.len()).unwrap_or(i64::MAX) >= queue.maxsize
    })
}

/// `Queue.put_nowait(item)`.
fn queue_put_nowait(id: HeapId, item: Value, vm: &mut VM<'_>) -> RunResult<()> {
    // A waiting `get()` takes the item without it ever entering the queue.
    loop {
        let getter = with_primitive!(id, vm, Queue, |queue| queue.getters.pop_front());
        match getter {
            Some(future) if !future_is_settled(future, vm) => {
                with_primitive!(id, vm, Queue, |queue| queue.unfinished += 1);
                vm.settle_future(future, Outcome::Value(item));
                vm.heap.dec_ref(future);
                return Ok(());
            }
            Some(future) => vm.heap.dec_ref(future),
            None => break,
        }
    }
    if queue_is_full(id, vm) {
        item.drop_with(vm);
        return Err(SimpleException::new_none(ExcType::QueueFull).into());
    }
    with_primitive!(id, vm, Queue, |queue| {
        queue.items.push_back(item);
        queue.unfinished += 1;
    });
    Ok(())
}

/// `Queue.put(item)`.
fn queue_put(id: HeapId, item: Value, vm: &mut VM<'_>) -> RunResult<Value> {
    if !queue_is_full(id, vm) {
        queue_put_nowait(id, item, vm)?;
        return Ok(vm.settled_future(Outcome::Value(Value::None)));
    }
    let future = vm.alloc_future(FutureKind::Future, None);
    vm.heap.inc_ref(future);
    with_primitive!(id, vm, Queue, |queue| queue.putters.push_back((future, item)));
    Ok(Value::Ref(future))
}

/// `Queue.get_nowait()`.
fn queue_get_nowait(id: HeapId, vm: &mut VM<'_>) -> RunResult<Value> {
    let item = with_primitive!(id, vm, Queue, |queue| queue.items.pop_front());
    let Some(item) = item else {
        return Err(SimpleException::new_none(ExcType::QueueEmpty).into());
    };
    // A blocked `put()` can now hand its item over.
    loop {
        if queue_is_full(id, vm) {
            break;
        }
        let putter = with_primitive!(id, vm, Queue, |queue| queue.putters.pop_front());
        match putter {
            Some((future, pending)) if !future_is_settled(future, vm) => {
                with_primitive!(id, vm, Queue, |queue| {
                    queue.items.push_back(pending);
                    queue.unfinished += 1;
                });
                vm.settle_future(future, Outcome::Value(Value::None));
                vm.heap.dec_ref(future);
                break;
            }
            Some((future, pending)) => {
                pending.drop_with(vm);
                vm.heap.dec_ref(future);
            }
            None => break,
        }
    }
    Ok(item)
}

/// `Queue.get()`.
fn queue_get(id: HeapId, vm: &mut VM<'_>) -> Value {
    if queue_len(id, vm) > 0 {
        return match queue_get_nowait(id, vm) {
            Ok(item) => vm.settled_future(Outcome::Value(item)),
            Err(error) => vm.settled_future(Outcome::Error(error)),
        };
    }
    let future = vm.alloc_future(FutureKind::Future, None);
    vm.heap.inc_ref(future);
    with_primitive!(id, vm, Queue, |queue| queue.getters.push_back(future));
    Value::Ref(future)
}

/// `Queue.task_done()`.
fn queue_task_done(id: HeapId, vm: &mut VM<'_>) -> RunResult<()> {
    let drained = with_primitive!(id, vm, Queue, |queue| {
        if queue.unfinished <= 0 {
            None
        } else {
            queue.unfinished -= 1;
            Some(queue.unfinished == 0)
        }
    });
    let Some(drained) = drained else {
        return Err(ExcType::value_error("task_done() called too many times"));
    };
    if drained {
        let waiters = with_primitive!(id, vm, Queue, |queue| mem::take(&mut queue.join_waiters));
        for future in waiters {
            vm.settle_future(future, Outcome::Value(Value::None));
            vm.heap.dec_ref(future);
        }
    }
    Ok(())
}

/// `Queue.join()`.
fn queue_join(id: HeapId, vm: &mut VM<'_>) -> Value {
    if with_primitive!(id, vm, Queue, |queue| queue.unfinished == 0) {
        return vm.settled_future(Outcome::Value(Value::None));
    }
    let future = vm.alloc_future(FutureKind::Future, None);
    vm.heap.inc_ref(future);
    with_primitive!(id, vm, Queue, |queue| queue.join_waiters.push_back(future));
    Value::Ref(future)
}

/// Runs `f` on the `TaskGroup` at `id`.
fn with_taskgroup(id: HeapId, vm: &mut VM<'_>, f: impl FnOnce(&mut TaskGroup)) {
    with_primitive!(id, vm, TaskGroup, |group| f(group));
}

/// `TaskGroup.create_task(coro)`.
fn taskgroup_create_task(id: HeapId, coroutine: &Value, vm: &mut VM<'_>) -> RunResult<Value> {
    if !with_primitive!(id, vm, TaskGroup, |group| group.entered) {
        return Err(SimpleException::new_msg(ExcType::RuntimeError, "TaskGroup has not been entered").into());
    }
    let task = as_future(coroutine, vm)?;
    vm.heap.inc_ref(task);
    with_primitive!(id, vm, TaskGroup, |group| group.children.push(task));
    Ok(Value::Ref(task))
}

/// `TaskGroup.__aexit__`: waits for every child, cancelling the rest as soon as
/// one fails, and re-raising the first failure.
fn taskgroup_aexit(id: HeapId, args: ArgValues, vm: &mut VM<'_>) -> Value {
    let raised = args.clone_positional(vm.heap);
    args.drop_with(vm);
    let body_failed = matches!(raised.first(), Some(value) if !matches!(value, Value::None));
    raised.drop_with(vm);
    let children = with_primitive!(id, vm, TaskGroup, |group| {
        group.entered = false;
        mem::take(&mut group.children)
    });
    if body_failed {
        for child in &children {
            vm.cancel_future(*child, Value::None);
        }
    }
    Value::Ref(vm.start_combinator(CombinatorKind::TaskGroup, children))
}

/// `asyncio.timeout(...).__aenter__`: arms the deadline against the running task.
fn timeout_enter(id: HeapId, vm: &mut VM<'_>) -> RunResult<Value> {
    let deadline = with_primitive!(id, vm, Timeout, |timeout| timeout.deadline);
    let Some(deadline) = deadline else {
        return Ok(vm.settled_future(Outcome::Value(Value::None)));
    };
    let Some(task) = vm.current_task_future_id() else {
        return Err(SimpleException::new_msg(ExcType::RuntimeError, "timeout() can only be used inside a task").into());
    };
    let delay = deadline.saturating_sub(vm.scheduler_now());
    let timer = vm.arm_timer(delay, Value::None, Some(task));
    with_primitive!(id, vm, Timeout, |timeout| timeout.timer = Some(timer));
    Ok(vm.settled_future(Outcome::Value(Value::None)))
}

/// `asyncio.timeout(...).__aexit__`: disarms the deadline, and turns the
/// cancellation it caused into the `TimeoutError` CPython raises.
fn timeout_exit(id: HeapId, args: ArgValues, vm: &mut VM<'_>) -> Value {
    let raised = args.clone_positional(vm.heap);
    args.drop_with(vm);
    let cancelled = raised.first().is_some_and(|value| {
        matches!(value, Value::Builtin(Builtins::ExcType(ExcType::CancelledError)))
            || matches!(value, Value::Ref(exc) if matches!(
                vm.heap.get(*exc),
                HeapData::Exception(e) if e.exc_type() == ExcType::CancelledError
            ))
    });
    raised.drop_with(vm);

    let timer = with_primitive!(id, vm, Timeout, |timeout| timeout.timer.take());
    let expired = match timer {
        Some(timer) => {
            let fired = future_is_settled(timer, vm);
            vm.disarm_timers(timer);
            vm.heap.dec_ref(timer);
            fired
        }
        None => false,
    };
    with_primitive!(id, vm, Timeout, |timeout| timeout.expired = expired);
    if expired && cancelled {
        let error = SimpleException::new_none(ExcType::TimeoutError).into();
        return vm.settled_future(Outcome::Error(error));
    }
    vm.settled_future(Outcome::Value(Value::None))
}

/// Whether the future at `id` has settled, which is how a primitive tells a
/// live waiter from a cancelled one.
fn future_is_settled(id: HeapId, vm: &mut VM<'_>) -> bool {
    match vm.heap.get(id) {
        HeapData::Future(future) => !matches!(future.state, FutureState::Pending),
        _ => false,
    }
}
