//! Builtins that call back into Python, and the host calls a callback makes.
//!
//! A builtin that calls a function back runs it in a frame on the VM's own
//! loop, so a call to the host in the callback suspends the run as it would
//! anywhere, a dump taken while it waits holds the call whole, and what the
//! callback raises reaches the handlers around the builtin. What matches
//! CPython in the values is proven in `test_cases/`, which runs against both
//! interpreters; here is what only a host can see.

use std::mem;

use monty::{Dump, MontyRun, RunProgress, Session, SessionRef, dump};
use monty_types::{
    CompileOptions, ExcType, ExtFunctionResult, MontyException, MontyObject, NameLookupResult, PrintWriter,
    ResourceTracker,
};

/// Runs `code`, answering each call to the host, `fetch(x)`, with `x * 10`,
/// each `os.getenv()` with `'env'`, and each lookup of a name with a host
/// function, until the run ends. With `dumping`, the run goes through a dump
/// and a load at each call. Gives the value of the last line and the argument
/// of each `fetch`, in order.
fn answer(code: &str, dumping: bool) -> (MontyObject, Vec<i64>) {
    let run = MontyRun::new(code.to_owned(), "cb.py", vec![], CompileOptions::default()).unwrap();
    let mut progress = run
        .start(vec![], ResourceTracker::default(), PrintWriter::Stdout)
        .unwrap();
    let mut asked = Vec::new();
    loop {
        if dumping && matches!(progress, RunProgress::FunctionCall(_) | RunProgress::OsCall(_)) {
            let bytes = dump("cb.py", None, &[], SessionRef::Running(&progress)).unwrap();
            let Session::Running(back) = Dump::load(&bytes).unwrap().state else {
                panic!("dumped a run, loaded something else")
            };
            // end the dumped original, which releases its heap; the restored copy carries on
            let original = mem::replace(&mut progress, *back);
            let stop = MontyException::new(ExcType::RuntimeError, None);
            match original {
                RunProgress::FunctionCall(call) => call.abort(stop, PrintWriter::Disabled),
                RunProgress::OsCall(call) => call.abort(stop, PrintWriter::Disabled),
                _ => unreachable!("only a call is dumped"),
            }
            .unwrap_err();
        }
        progress = match progress {
            RunProgress::NameLookup(lookup) => {
                let name = lookup.name.clone();
                lookup
                    .resume(
                        NameLookupResult::Value(MontyObject::function(name, None)),
                        PrintWriter::Stdout,
                    )
                    .unwrap()
            }
            RunProgress::FunctionCall(call) => {
                assert_eq!(call.function_name, "fetch");
                let x: i64 = call.args.arg(0).unwrap().try_into().unwrap();
                asked.push(x);
                call.resume(ExtFunctionResult::Return(MontyObject::int(x * 10)), PrintWriter::Stdout)
                    .unwrap()
            }
            RunProgress::OsCall(call) => {
                assert_eq!(call.function_call.name(), "os.getenv");
                call.resume(MontyObject::string("env".to_owned()), PrintWriter::Stdout)
                    .unwrap()
            }
            done => return (done.into_complete().expect("the run completes"), asked),
        };
    }
}

/// Each case is a program whose last line is `True` in CPython when `fetch(x)`
/// gives `x * 10` and `os.getenv()` gives `'env'`, with the arguments that its
/// callbacks give `fetch`, in the order CPython calls them.
const CALLS: &[(&str, &[i64])] = &[
    (
        "import os\nlist(map(lambda x: fetch(x) + len(os.getenv('K')), [1, 2])) == [13, 23]",
        &[1, 2],
    ),
    (
        "list(map(lambda a, b: fetch(a) + b, [1, 2, 3], [5, 6])) == [15, 26]",
        &[1, 2],
    ),
    ("list(map(str, (fetch(x) for x in [1, 2]))) == ['10', '20']", &[1, 2]),
    (
        "import os\nlist(filter(lambda x: fetch(x) > 15 and os.getenv('K') == 'env', [1, 2, 3])) == [2, 3]",
        &[1, 2, 3],
    ),
    ("list(filter(None, (fetch(x) - 10 for x in [1, 2]))) == [10]", &[1, 2]),
    ("sorted([3, 1, 2], key=lambda x: -fetch(x)) == [3, 2, 1]", &[3, 1, 2]),
    (
        "sorted((fetch(x) for x in [2, 1]), key=lambda v: -fetch(v)) == [20, 10]",
        &[2, 1, 20, 10],
    ),
    (
        "import os\nxs = [3, 1, 2]\nxs.sort(key=lambda x: (os.getenv('K'), fetch(x)), reverse=True)\nxs == [3, 2, 1]",
        &[3, 1, 2],
    ),
    ("min([3, 1, 2], key=lambda x: fetch(x)) == 1", &[3, 1, 2]),
    (
        "import os\nmax(3, 1, 2, key=lambda x: (os.getenv('K'), -fetch(x))) == 1",
        &[3, 1, 2],
    ),
    ("max((fetch(x) for x in [1, 3, 2]), key=lambda v: -v) == 10", &[1, 3, 2]),
    ("max([], key=fetch, default='none') == 'none'", &[]),
    (
        "list(zip((fetch(x) for x in [1, 2, 3]), 'ab')) == [(10, 'a'), (20, 'b')]",
        &[1, 2, 3],
    ),
    (
        "list(zip('ab', (fetch(x) for x in [1, 2, 3]))) == [('a', 10), ('b', 20)]",
        &[1, 2],
    ),
    (
        "list(enumerate(fetch(x) for x in [1, 2])) == [(0, 10), (1, 20)]",
        &[1, 2],
    ),
];

/// What a callback raises, a builtin whose callback raises, and an error of
/// the builtin around a callback that called the host, with the arguments
/// that the callbacks give `fetch`. A handler around the builtin catches it.
const RAISES: &[(&str, &[i64])] = &[
    (
        "def boom(x):\n    if fetch(x) > 15:\n        raise ValueError('big')\n    return x\n\ntry:\n    list(map(boom, [1, 2, 3]))\n    out = 'no'\nexcept ValueError as e:\n    out = str(e)\nout == 'big'",
        &[1, 2],
    ),
    (
        "def boom(x):\n    if fetch(x) > 15:\n        raise ValueError('big')\n    return x\n\ntry:\n    list(filter(boom, [1, 2, 3]))\n    out = 'no'\nexcept ValueError as e:\n    out = str(e)\nout == 'big'",
        &[1, 2],
    ),
    (
        "def boom(x):\n    if fetch(x) > 15:\n        raise ValueError('big')\n    return x\n\ntry:\n    sorted([1, 2, 3], key=boom)\n    out = 'no'\nexcept ValueError as e:\n    out = str(e)\nout == 'big'",
        &[1, 2],
    ),
    (
        "def boom(x):\n    if fetch(x) > 15:\n        raise ValueError('big')\n    return x\n\nxs = [3, 1, 2]\ntry:\n    xs.sort(key=boom)\n    out = 'no'\nexcept ValueError as e:\n    out = str(e)\n(out, xs) == ('big', [3, 1, 2])",
        &[3],
    ),
    (
        "xs = [3, 1, 2]\ntry:\n    xs.sort(key=lambda x: xs.append(0) or fetch(x))\n    out = 'no'\nexcept ValueError as e:\n    out = str(e)\n(out, xs) == ('list modified during sort', [1, 2, 3])",
        &[3, 1, 2],
    ),
    (
        "def boom(x):\n    if fetch(x) > 15:\n        raise ValueError('big')\n    return x\n\ntry:\n    min([1, 2, 3], key=boom)\n    out = 'no'\nexcept ValueError as e:\n    out = str(e)\nout == 'big'",
        &[1, 2],
    ),
    (
        "def boom(x):\n    if fetch(x) > 15:\n        raise ValueError('big')\n    return x\n\ntry:\n    max(1, 2, 3, key=boom)\n    out = 'no'\nexcept ValueError as e:\n    out = str(e)\nout == 'big'",
        &[1, 2],
    ),
    (
        "def boom(x):\n    if fetch(x) > 15:\n        raise ValueError('big')\n    return x\n\ntry:\n    list(zip((boom(x) for x in [1, 2, 3]), 'abc'))\n    out = 'no'\nexcept ValueError as e:\n    out = str(e)\nout == 'big'",
        &[1, 2],
    ),
    (
        "def boom(x):\n    if fetch(x) > 15:\n        raise ValueError('big')\n    return x\n\ntry:\n    list(enumerate(boom(x) for x in [1, 2]))\n    out = 'no'\nexcept ValueError as e:\n    out = str(e)\nout == 'big'",
        &[1, 2],
    ),
    (
        "def boom(x):\n    if fetch(x) > 15:\n        raise ValueError('big')\n    return x\n\ntry:\n    list(zip((fetch(x) for x in [1]), 'ab', strict=True))\n    out = 'no'\nexcept ValueError as e:\n    out = str(e)\nout == 'zip() argument 2 is longer than argument 1'",
        &[1],
    ),
    (
        "def boom(x):\n    if fetch(x) > 15:\n        raise ValueError('big')\n    return x\n\ntry:\n    list(zip('abc', 'xyz', (fetch(x) for x in [1]), strict=True))\n    out = 'no'\nexcept ValueError as e:\n    out = str(e)\nout == 'zip() argument 3 is shorter than arguments 1-2'",
        &[1],
    ),
];

/// A callback of a builtin calls an external function and an OS function,
/// and the builtin gives the value that CPython gives.
#[test]
fn a_callback_calls_the_host() {
    for (code, asked) in CALLS {
        assert_eq!(answer(code, false), (MontyObject::bool(true), asked.to_vec()), "{code}");
    }
}

/// A callback that waits on the host is a frame on the VM's own loop, so a
/// dump taken while it waits holds the builtin whole, and a load goes on
/// where it stood.
#[test]
fn a_callback_survives_dump_and_load() {
    for (code, asked) in CALLS {
        assert_eq!(answer(code, true), (MontyObject::bool(true), asked.to_vec()), "{code}");
    }
}

/// What a callback raises after a call to the host reaches the handler
/// around the builtin, as in CPython, with or without a dump on the way.
#[test]
fn a_raise_in_a_callback_reaches_the_handler_around_the_builtin() {
    for dumping in [false, true] {
        for (code, asked) in RAISES {
            assert_eq!(
                answer(code, dumping),
                (MontyObject::bool(true), asked.to_vec()),
                "{code}"
            );
        }
    }
}
