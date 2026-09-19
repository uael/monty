use std::{fs, path::Path};

fn main() {
    pyo3_build_config::add_libpython_rpath_link_args();
    check_interpreter_version();
}

/// Warns when pyo3 resolved an interpreter other than the one the workspace
/// pins, while there is still a build log to read it in.
///
/// The harness embeds libpython and diffs Monty against the CPython it links,
/// so the pinned minor is part of the expectations: an older one invents
/// divergences that are the interpreter's rather than Monty's, and on macOS it
/// is not even a run, because the `python3` first on `PATH` there is Xcode's
/// 3.9 and the binary that links against it aborts in dyld before `main`.
/// A warning rather than a failure, so an ordinary `cargo build` of the
/// workspace still works where `python3` is old; the Makefile recipes pass the
/// right interpreter.
fn check_interpreter_version() {
    let pinned_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.python-version");
    println!("cargo::rerun-if-changed={}", pinned_path.display());
    println!("cargo::rerun-if-env-changed=PYO3_PYTHON");

    let pinned = fs::read_to_string(&pinned_path).expect("workspace .python-version should be readable");
    let mut parts = pinned.trim().split('.');
    let (Some(Ok(major)), Some(Ok(minor))) = (parts.next().map(str::parse::<u8>), parts.next().map(str::parse::<u8>))
    else {
        panic!(
            "{} should hold a `<major>.<minor>` version, got {pinned:?}",
            pinned_path.display()
        );
    };

    let resolved = pyo3_build_config::get();
    let found = resolved.version();
    let executable = resolved.executable().unwrap_or("<unknown>");
    if (found.major, found.minor) != (major, minor) {
        println!(
            "cargo::warning=monty-datatest resolved CPython {}.{} at {executable}, not the pinned \
             {major}.{minor}; test-case expectations track the pinned version, so divergences may \
             be the interpreter's rather than Monty's. `make test-cases` passes the pinned one.",
            found.major, found.minor,
        );
    }
}
