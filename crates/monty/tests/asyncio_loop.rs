//! Tests for `asyncio.get_running_loop()` and `asyncio.current_task()` where
//! Monty's scheduler model diverges from CPython's, so the dual-run
//! `test_cases/async__loop.py` cannot cover them.
//!
//! Monty is inside its own loop whenever sandbox code runs, so there is no
//! outside for `get_running_loop()` to raise in, and it schedules coroutines
//! without giving a program a `Task` object for one. See
//! `limitations/asyncio.md`.

use monty::MontyRun;
use monty_types::{CompileOptions, MontyObject};

/// Runs `code` to completion with no resource limits and returns the value of
/// its final expression.
fn eval(code: &str) -> MontyObject {
    MontyRun::new(code.to_owned(), "test.py", vec![], CompileOptions::default())
        .unwrap()
        .run_no_limits(vec![])
        .unwrap()
}

#[test]
fn a_loop_is_running_at_module_level_too() {
    // CPython raises `RuntimeError: no running event loop` here, having no
    // loop until `asyncio.run()` starts one.
    assert_eq!(
        eval("import asyncio\nasyncio.get_running_loop().is_running()"),
        MontyObject::bool(true)
    );
    assert_eq!(
        eval("import asyncio\nasyncio.get_running_loop().is_closed()"),
        MontyObject::bool(false)
    );
}

#[test]
fn two_calls_give_two_loops() {
    // CPython hands back the one running loop each time. Nothing here keeps
    // loop state for two objects to share, so they are only equal by identity
    // with themselves.
    assert_eq!(
        eval("import asyncio\nasyncio.get_running_loop() is asyncio.get_running_loop()"),
        MontyObject::bool(false)
    );
    assert_eq!(
        eval("import asyncio\nloop = asyncio.get_running_loop()\nloop is loop"),
        MontyObject::bool(true)
    );
}

#[test]
fn the_loop_answers_nothing_that_schedules() {
    assert_eq!(
        eval(
            "import asyncio\n\
             said = ''\n\
             try:\n    asyncio.get_running_loop().create_future()\n\
             except AttributeError as exc:\n    said = str(exc)\n\
             said"
        ),
        MontyObject::string("'EventLoop' object has no attribute 'create_future'".to_owned())
    );
}

#[test]
fn there_is_no_current_task_inside_a_coroutine_either() {
    // CPython answers `None` outside a task and a `Task` inside one; Monty has
    // no object for a task, so the answer is always `None`.
    assert_eq!(
        eval(
            "import asyncio\n\
             async def main():\n    return asyncio.current_task()\n\
             asyncio.run(main())"
        ),
        MontyObject::none()
    );
    assert_eq!(eval("import asyncio\nasyncio.current_task()"), MontyObject::none());
}
