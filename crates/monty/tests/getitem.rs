//! `__getitem__` dispatch on user instances.

use monty::MontyRun;
use monty_types::{CompileOptions, MontyObject, ResourceTracker};

fn run(code: &str) -> Result<MontyObject, String> {
    let runner = MontyRun::new(code.to_owned(), "getitem.py", vec![], CompileOptions::default()).unwrap();
    runner
        .run(vec![], ResourceTracker::default(), monty_types::PrintWriter::Disabled)
        .map_err(|e| e.to_string())
}

#[test]
fn int_key_dispatches() {
    let got = run("class C:\n    def __getitem__(self, k):\n        return k * 10\nC()[4]\n").unwrap();
    assert_eq!(got, MontyObject::Int(40));
}

#[test]
fn slice_key_reaches_the_dunder() {
    let got =
        run("class C:\n    def __getitem__(self, k):\n        return (k.start, k.stop, k.step)\nC()[2:5]\n").unwrap();
    assert_eq!(
        got,
        MontyObject::Tuple(vec![MontyObject::Int(2), MontyObject::Int(5), MontyObject::None])
    );
}

#[test]
fn chunk_style_slicing_works() {
    let got = run(concat!(
        "class Chunk:\n",
        "    def __init__(self, content, lo, hi):\n",
        "        self.content = content\n",
        "        self.lo = lo\n",
        "        self.hi = hi\n",
        "    def __getitem__(self, key):\n",
        "        start = self.lo if key.start is None else key.start\n",
        "        end = self.hi if key.stop is None else key.stop\n",
        "        if start < self.lo or end > self.hi or end < start:\n",
        "            raise ValueError(f'slice {start}-{end} escapes {self.lo}-{self.hi}')\n",
        "        lines = self.content.split('\\n')[start - self.lo : end - self.lo + 1]\n",
        "        return Chunk('\\n'.join(lines), start, end)\n",
        "c = Chunk('a\\nb\\nc\\nd', 1, 4)\n",
        "cut = c[2:3]\n",
        "(cut.content, cut.lo, cut.hi)\n",
    ))
    .unwrap();
    assert_eq!(
        got,
        MontyObject::Tuple(vec![
            MontyObject::String("b\nc".to_owned()),
            MontyObject::Int(2),
            MontyObject::Int(3)
        ])
    );
}

#[test]
fn a_dunder_raising_propagates() {
    let err = run("class C:\n    def __getitem__(self, k):\n        raise KeyError(str(k))\nC()[7]\n").unwrap_err();
    assert!(err.contains("KeyError"), "got: {err}");
}

#[test]
fn no_dunder_keeps_the_existing_error() {
    let err = run("class C:\n    pass\nC()[1]\n").unwrap_err();
    assert!(err.contains("'C' object is not subscriptable"), "got: {err}");
}
