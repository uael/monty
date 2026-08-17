# Tests for the os.path submodule and the package-binding import form.
# The sandbox is POSIX on every host, so these are posixpath results and the
# whole file is guarded on Windows CPython, where os.path is ntpath.
import os
import os.path
import os.path as ospath
import sys
from os.path import normpath
from pathlib import Path

if sys.platform != 'win32':
    # === import forms ===
    # `import os.path` binds the package, so `os` is the name in scope and the
    # submodule is reached through it. The aliased form names the submodule.
    # Identity is not asserted: Monty builds a fresh module object per import
    # (see limitations/modules.md), so `os.path is ospath` diverges.
    assert type(os.path).__name__ == 'module'
    assert os.path.normpath('a//b') == 'a/b'
    assert ospath.normpath('a//b') == 'a/b'

    # === relative paths ===
    assert normpath('a/b/../c') == 'a/c'
    assert normpath('a//b') == 'a/b'
    assert normpath('a/./b') == 'a/b'
    assert normpath('a/..') == '.'
    assert normpath('./') == '.'
    assert normpath('././.') == '.'
    assert normpath('') == '.'
    assert normpath('.') == '.'

    # === `..` that cannot be collapsed stays ===
    assert normpath('..') == '..'
    assert normpath('../../x') == '../../x'
    assert normpath('..//../x') == '../../x'
    assert normpath('a/../../b') == '../b'

    # === roots: POSIX reserves exactly two leading slashes ===
    assert normpath('/') == '/'
    assert normpath('//') == '//'
    assert normpath('///') == '/'
    assert normpath('////') == '/'
    assert normpath('///a//b') == '/a/b'
    assert normpath('//a/b') == '//a/b'

    # === a `..` under a root is dropped, never climbed past ===
    assert normpath('/..') == '/'
    assert normpath('/../a') == '/a'
    assert normpath('//a/b/../..') == '//'
    assert normpath('/a/./b//c/..') == '/a/b'

    # === PathLike arguments, and the str result ===
    assert normpath(Path('x/./y/..')) == 'x'
    assert type(normpath(Path('x'))) is str

    # === argument errors ===
    # CPython 3.14 resolves normpath to posix._path_normpath, whose errors name
    # that C function rather than `normpath`.
    try:
        normpath(1)  # pyright: ignore[reportArgumentType]
        raise AssertionError('expected TypeError')
    except TypeError as e:
        assert str(e) == '_path_normpath: path should be string, bytes or os.PathLike, not int'

    try:
        normpath(None)  # pyright: ignore[reportArgumentType]
        raise AssertionError('expected TypeError')
    except TypeError as e:
        assert str(e) == '_path_normpath: path should be string, bytes or os.PathLike, not NoneType'

    try:
        normpath()  # pyright: ignore[reportCallIssue]
        raise AssertionError('expected TypeError')
    except TypeError as e:
        assert str(e) == "_path_normpath() missing required argument 'path' (pos 1)"

    try:
        normpath('a', 'b')  # pyright: ignore[reportCallIssue]
        raise AssertionError('expected TypeError')
    except TypeError as e:
        assert str(e) == '_path_normpath() takes at most 1 argument (2 given)'
