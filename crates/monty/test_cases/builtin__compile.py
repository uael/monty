# === A code object runs where its source would ===
c = compile('1 + 1', 'f.py', 'eval')
assert type(c).__name__ == 'code'
assert eval(c) == 2

ns = {}
assert exec(compile('x = 5', 'f.py', 'exec'), ns) is None
assert ns['x'] == 5

# === The mode is the code object's, not the caller's ===
# `eval()` of an 'exec' body runs it and answers None; `exec()` of an 'eval'
# expression evaluates it and throws the value away.
ns = {}
assert eval(compile('y = 7', 'f.py', 'exec'), ns) is None
assert ns['y'] == 7
assert exec(compile('1 + 1', 'f.py', 'eval')) is None

# === A code object runs more than once, in whichever namespace it is given ===
doubler = compile('out = n * 2', 'f.py', 'exec')
first = {'n': 3}
second = {'n': 10}
exec(doubler, first)
exec(doubler, second)
assert first['out'] == 6
assert second['out'] == 20

# === The source is parsed by compile(), so a syntax error lands there ===
try:
    compile('1 +', 'f.py', 'eval')
    raise AssertionError
except SyntaxError:
    pass

# === Equality and truth ===
assert compile('1 + 1', 'f.py', 'eval') == compile('1 + 1', 'g.py', 'eval')
assert compile('1 + 1', 'f.py', 'eval') != compile('1 + 2', 'f.py', 'eval')
assert compile('1 + 1', 'f.py', 'eval') != compile('1 + 1', 'f.py', 'exec')
assert bool(compile('1', 'f.py', 'eval'))
assert compile('1', 'f.py', 'eval') != 1

# === Arguments ===
assert eval(compile(source='2 + 2', filename='f.py', mode='eval')) == 4
assert eval(compile('2 + 2', 'f.py', 'eval', 0, False, -1)) == 4
assert eval(compile(b'3 + 3', b'f.py', 'eval')) == 6
assert eval(compile('4 + 4', 'f.py', 'eval', dont_inherit=True)) == 8

try:
    compile('1', 'f.py', 'nope')
    raise AssertionError
except ValueError as exc:
    assert str(exc) == "compile() mode must be 'exec', 'eval' or 'single'"

try:
    compile(1, 'f.py', 'eval')
    raise AssertionError
except TypeError as exc:
    assert str(exc) == 'compile() arg 1 must be a string, bytes or AST object'

try:
    compile('1', 1, 'eval')
    raise AssertionError
except TypeError as exc:
    assert str(exc) == 'expected str, bytes or os.PathLike object, not int'
