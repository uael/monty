use std::{fs, path::Path};

fn main() {
    pyo3_build_config::add_libpython_rpath_link_args();
    check_interpreter_version();
}

/// Fails the build when pyo3 resolved an interpreter older than the workspace
/// pins, while there is still a build log to read it in.
///
/// The harness embeds libpython and diffs Monty against the CPython it links,
/// so the pinned minor is part of the expectations. An older one is not a
/// weaker run but a broken one, and on macOS it is not even a run: the
/// `python3` first on `PATH` there is Xcode's 3.9, and the binary that links
/// against it aborts in dyld before `main` with a message about a missing
/// `Python3.framework` that names neither pyo3 nor this crate.
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
    assert!(
        (found.major, found.minor) >= (major, minor),
        "monty-datatest runs every test case against the CPython it links, so it needs the \
         interpreter this workspace pins.\n\
         \n\
         pinned:   {major}.{minor} (from .python-version)\n\
         resolved: {}.{} at {executable}\n\
         \n\
         Point pyo3 at a {major}.{minor} interpreter:\n\
         \n    PYO3_PYTHON=$(uv python find {major}.{minor}) cargo run -p monty-datatest\n\
         \n\
         `make test-cases` (and every other datatest recipe) already does this.",
        found.major,
        found.minor,
    );
    if (found.major, found.minor) > (major, minor) {
        println!(
            "cargo::warning=monty-datatest resolved CPython {}.{} at {executable}, newer than the \
             pinned {major}.{minor}; test-case expectations track the pinned version, so \
             divergences may be CPython's rather than Monty's",
            found.major, found.minor,
        );
    }
}
