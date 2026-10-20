//! Tests for `CompileOptions::os`: a session with no operating system lacks
//! `os`, `open` and the disk methods of `pathlib.Path`, and a session with the
//! host's keeps them.
//!
//! Each failure is raised inside the sandbox, so the code can catch it and the
//! host never sees an `OsCall`.

use monty::{Dump, MontyRepl, MontyRun, RunProgress, Session, SessionRef, dump};
use monty_types::{CompileOptions, ExcType, MontyException, MontyObject, PrintWriter, ResourceTracker, SandboxOs};

/// The options of a session with no operating system.
fn no_os() -> CompileOptions {
    CompileOptions {
        os: SandboxOs::Absent,
        ..CompileOptions::default()
    }
}

/// Runs `code` to completion in a session with no operating system.
fn run_without_os(code: &str) -> Result<MontyObject, MontyException> {
    MontyRun::new(code.to_owned(), "test.py", vec![], no_os())
        .unwrap()
        .run_no_limits(vec![])
}

/// Starts `code` in a session with no operating system and returns the value it
/// completes with, failing if the run suspends for the host instead.
fn complete_without_os(code: &str) -> MontyObject {
    let runner = MontyRun::new(code.to_owned(), "test.py", vec![], no_os()).unwrap();
    match runner
        .start(vec![], ResourceTracker::default(), PrintWriter::Disabled)
        .unwrap()
    {
        RunProgress::Complete(value) => value,
        progress => panic!("expected the run to complete in the sandbox, got {progress:?}"),
    }
}

/// The name of the OS call that `code` suspends with in a session with `options`.
fn os_call_name(code: &str, options: CompileOptions) -> &'static str {
    let runner = MontyRun::new(code.to_owned(), "test.py", vec![], options).unwrap();
    match runner
        .start(vec![], ResourceTracker::default(), PrintWriter::Disabled)
        .unwrap()
    {
        RunProgress::OsCall(call) => call.function_call.name(),
        progress => panic!("expected an OS call, got {progress:?}"),
    }
}

/// The type and message of the exception `code` raises with no operating system.
fn raised_without_os(code: &str) -> (ExcType, String) {
    let err = run_without_os(code).expect_err("expected the code to raise");
    (err.exc_type(), err.message().unwrap_or_default().to_owned())
}

#[test]
fn default_session_has_the_host_os() {
    assert_eq!(CompileOptions::default().os, SandboxOs::Host);
    let host = CompileOptions::default();
    assert_eq!(os_call_name("import os\nos.getenv('HOME')", host), "os.getenv");
    assert_eq!(os_call_name("from os import environ\nenviron", host), "os.environ");
    assert_eq!(os_call_name("open('/data.txt')", host), "open");
    assert_eq!(
        os_call_name("import builtins\nbuiltins.open('/data.txt')", host),
        "open"
    );
    assert_eq!(
        os_call_name("from pathlib import Path\nPath('/data.txt').read_text()", host),
        "Path.read_text"
    );
    assert_eq!(
        os_call_name("from pathlib import Path\nPath('/data.txt').open()", host),
        "open"
    );
}

#[test]
fn import_os_raises_module_not_found() {
    let missing = (ExcType::ModuleNotFoundError, "No module named 'os'".to_owned());
    assert_eq!(raised_without_os("import os"), missing);
    assert_eq!(raised_without_os("import os as system"), missing);
    assert_eq!(raised_without_os("from os import getenv"), missing);
    assert_eq!(raised_without_os("import os.path"), missing);
    assert_eq!(raised_without_os("import os.path as osp"), missing);
    assert_eq!(raised_without_os("from os.path import join"), missing);
    assert_eq!(raised_without_os("def f():\n    import os\nf()"), missing);
    assert_eq!(raised_without_os("exec('import os')"), missing);
}

#[test]
fn import_os_raises_inside_the_sandbox() {
    let code = "\
try:
    import os
except ModuleNotFoundError as error:
    caught = str(error)
caught";
    assert_eq!(
        complete_without_os(code),
        MontyObject::string("No module named 'os'".to_owned())
    );
}

#[test]
fn other_modules_stay() {
    let code = "\
import pathlib, time, json
from datetime import datetime
type(time.time()).__name__, json.dumps([1]), pathlib.Path('/a').name";
    assert_eq!(
        complete_without_os(code),
        MontyObject::tuple([
            MontyObject::string("float".to_owned()),
            MontyObject::string("[1]".to_owned()),
            MontyObject::string("a".to_owned()),
        ])
    );
}

#[test]
fn open_is_not_defined() {
    let undefined = (ExcType::NameError, "name 'open' is not defined".to_owned());
    assert_eq!(raised_without_os("open('/data.txt')"), undefined);
    assert_eq!(raised_without_os("f = open"), undefined);
    assert_eq!(
        raised_without_os("def f():\n    return open('/data.txt')\nf()"),
        undefined
    );
    assert_eq!(raised_without_os("eval('open')"), undefined);
    assert_eq!(
        raised_without_os("import builtins\nbuiltins.open"),
        (
            ExcType::AttributeError,
            "'module' object has no attribute 'open'".to_owned()
        )
    );
}

#[test]
fn a_name_bound_open_stays() {
    let code = "\
def open(name):
    return 'mine ' + name
open('a')";
    assert_eq!(complete_without_os(code), MontyObject::string("mine a".to_owned()));
}

#[test]
fn path_disk_methods_raise_attribute_error() {
    let methods = [
        "exists()",
        "is_file()",
        "is_dir()",
        "is_symlink()",
        "read_text()",
        "read_bytes()",
        "stat()",
        "iterdir()",
        "resolve()",
        "absolute()",
        "unlink()",
        "rmdir()",
        "write_text('x')",
        "append_text('x')",
        "write_bytes(b'x')",
        "append_bytes(b'x')",
        "mkdir()",
        "rename('/b')",
        "open()",
    ];
    for call in methods {
        let name = call.split_once('(').unwrap().0;
        assert_eq!(
            raised_without_os(&format!("from pathlib import Path\nPath('/a').{call}")),
            (
                ExcType::AttributeError,
                format!("'PosixPath' object has no attribute '{name}'")
            ),
            "Path.{call}"
        );
    }
}

#[test]
fn path_disk_methods_raise_inside_the_sandbox() {
    let code = "\
from pathlib import Path
try:
    Path('/data.txt').write_text('x')
except AttributeError as error:
    caught = str(error)
caught";
    assert_eq!(
        complete_without_os(code),
        MontyObject::string("'PosixPath' object has no attribute 'write_text'".to_owned())
    );
}

#[test]
fn path_pure_methods_stay() {
    let code = "\
from pathlib import Path
p = Path('/data') / 'notes.txt'
(
    p.as_posix(),
    str(p.parent),
    p.name,
    p.stem,
    p.suffix,
    p.parts,
    p.with_name('a.md').as_posix(),
    p.with_suffix('.md').as_posix(),
    p.joinpath('x').as_posix(),
    p.is_absolute(),
    str(Path.cwd()),
)";
    let text = |value: &str| MontyObject::string(value.to_owned());
    assert_eq!(
        complete_without_os(code),
        MontyObject::tuple([
            text("/data/notes.txt"),
            text("/data"),
            text("notes.txt"),
            text("notes"),
            text(".txt"),
            MontyObject::tuple([text("/"), text("data"), text("notes.txt")]),
            text("/data/a.md"),
            text("/data/notes.md"),
            text("/data/notes.txt/x"),
            MontyObject::bool(true),
            text("/"),
        ])
    );
}

#[test]
fn open_call_asks_the_host_as_an_undefined_name() {
    let runner = MontyRun::new("open('/data.txt')".to_owned(), "test.py", vec![], no_os()).unwrap();
    let progress = runner
        .start(vec![], ResourceTracker::default(), PrintWriter::Disabled)
        .unwrap();
    let RunProgress::FunctionCall(call) = progress else {
        panic!("expected an external function call, got {progress:?}");
    };
    assert_eq!(call.function_name, "open");
}

#[test]
fn repl_session_without_os_keeps_the_setting() {
    let mut repl = MontyRepl::new("repl.py", ResourceTracker::default(), no_os());
    let err = repl.feed_run("import os", vec![], PrintWriter::Disabled).unwrap_err();
    assert_eq!(err.exc_type(), ExcType::ModuleNotFoundError);
    repl.feed_run("x = 1", vec![], PrintWriter::Disabled).unwrap();

    let bytes = dump("repl.py", None, SessionRef::Idle(&repl)).unwrap();
    let Session::Idle(mut repl) = Dump::load(&bytes).unwrap().state else {
        panic!("dumped an idle session, loaded something else");
    };
    let err = repl.feed_run("import os", vec![], PrintWriter::Disabled).unwrap_err();
    assert_eq!(err.exc_type(), ExcType::ModuleNotFoundError);
    assert_eq!(
        repl.feed_run("x", vec![], PrintWriter::Disabled).unwrap(),
        MontyObject::int(1)
    );
}
