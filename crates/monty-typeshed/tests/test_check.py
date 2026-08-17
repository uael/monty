"""Regression tests for `check.py`, the vendored-typeshed drift check.

The check exists to fail CI when `custom/` and `vendor/` disagree. It walked
`custom/*.pyi` non-recursively while `update.py` copies with `rglob`, so a stub
inside a package directory was invisible to it: `custom/collections/__init__.pyi`
could differ from the vendored copy that actually ships and the check still
reported the tree in sync.
"""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path
from types import ModuleType
from typing import Any

import pytest

CRATE_DIR = Path(__file__).resolve().parent.parent


def load(name: str) -> ModuleType:
    """Imports one of the crate's sibling scripts by path.

    They import each other as top-level modules (`import update`), which only
    resolves when their own directory is on `sys.path` — true when run as a
    script, not under pytest.
    """
    if str(CRATE_DIR) not in sys.path:
        sys.path.insert(0, str(CRATE_DIR))
    spec = importlib.util.spec_from_file_location(name, CRATE_DIR / f'{name}.py')
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


update = load('update')
check = load('check')


@pytest.fixture
def tree(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """A minimal custom/vendor pair that `check.py` reports as in sync.

    One flat override and one inside a package, which is the shape the bug
    turned on.
    """
    custom = tmp_path / 'custom'
    vendor = tmp_path / 'vendor' / 'typeshed'
    stdlib = vendor / 'stdlib'
    versions = 'sys: 3.0-\nos: 3.0-\nos.path: 3.0-\n'

    (custom / 'os').mkdir(parents=True)
    (custom / 'sys.pyi').write_text('version: str\n')
    (custom / 'os' / '__init__.pyi').write_text('sep: str\n')
    (custom / 'os' / 'path.pyi').write_text('def normpath(path: str) -> str: ...\n')

    stdlib.mkdir(parents=True)
    (vendor / 'source_commit.txt').write_text(f'{update.COMMIT}\n')
    (stdlib / 'VERSIONS').write_text(versions)
    (stdlib / 'builtins.pyi').write_text('class int: ...\n')
    for relative in ('sys.pyi', 'os/__init__.pyi', 'os/path.pyi'):
        destination = stdlib / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes((custom / relative).read_bytes())

    monkeypatch.setattr(update, 'CUSTOM_DIR', custom)
    monkeypatch.setattr(update, 'VENDOR_DIR', vendor)
    monkeypatch.setattr(update, 'STDLIB_DIR', stdlib)
    monkeypatch.setattr(update, 'COPY_FILES', [])
    monkeypatch.setattr(update, 'VERSIONS', versions)
    return tmp_path


def problems() -> list[str]:
    return [*check.check_tree_contents(), *check.check_custom_stubs(), *check.check_versions()]


def test_in_sync_tree_reports_nothing(tree: Path) -> None:
    assert problems() == []


def test_drift_in_a_package_stub_is_reported(tree: Path) -> None:
    (tree / 'custom' / 'os' / 'path.pyi').write_text('def normpath(path: str) -> bytes: ...\n')
    assert problems() == ['custom/os/path.pyi differs from stdlib/os/path.pyi']


def test_drift_in_a_flat_stub_is_reported(tree: Path) -> None:
    (tree / 'custom' / 'sys.pyi').write_text('version: bytes\n')
    assert problems() == ['custom/sys.pyi differs from stdlib/sys.pyi']


def test_package_stub_missing_from_versions_is_reported(tree: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(update, 'VERSIONS', 'sys: 3.0-\nos: 3.0-\n')
    assert problems() == ['custom/os/path.pyi is missing from VERSIONS, so the type checker ignores it']


def test_package_init_maps_to_the_package_itself(tree: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """`os/__init__.pyi` must satisfy the `os` entry, not an `os.__init__` one."""
    monkeypatch.setattr(update, 'VERSIONS', 'sys: 3.0-\nos.path: 3.0-\n')
    assert problems() == ['custom/os/__init__.pyi is missing from VERSIONS, so the type checker ignores it']


def test_unvendored_package_stub_is_reported(tree: Path) -> None:
    (tree / 'vendor' / 'typeshed' / 'stdlib' / 'os' / 'path.pyi').unlink()
    assert problems() == [
        'missing, update.py would write it: stdlib/os/path.pyi',
        'os.path is listed in VERSIONS but has no vendored stub',
    ]


def test_module_name_of(tree: Path) -> None:
    cases: dict[str, str] = {
        'sys.pyi': 'sys',
        'os/path.pyi': 'os.path',
        'collections/__init__.pyi': 'collections',
    }
    got: dict[str, Any] = {stub: check.module_name(stub) for stub in cases}
    assert got == cases
