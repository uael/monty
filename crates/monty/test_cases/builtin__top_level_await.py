# `ast.PyCF_ALLOW_TOP_LEVEL_AWAIT` makes a body that awaits at its top level
# compile as one that may, so running it hands back a coroutine to drive.
import ast


class Act(str):
    def __await__(self):
        return (yield self)


def act():
    return Act('act://one')


# === the code object runs in the dict it is given, and gives a coroutine ===
module = {'act': act}
code = compile('x = 1\nanswer = await act()\nresult = x + 1', '<rung>', 'exec', flags=ast.PyCF_ALLOW_TOP_LEVEL_AWAIT)
assert type(code).__name__ == 'code'
frame = eval(code, module)
assert type(frame).__name__ == 'coroutine'

# === the await travels out, and what is sent in is what it gives ===
said = frame.send(None)
assert said == 'act://one'
try:
    frame.send('answered')
    assert False, 'expected StopIteration'
except StopIteration:
    pass

# === what the body bound is in the dict, not in a frame of its own ===
assert module['x'] == 1
assert module['answer'] == 'answered'
assert module['result'] == 2

# === a body that awaits nothing is over where it began ===
# The flag lets a body await; only one that does becomes a coroutine.
module = {}
code = compile('done = 3', '<rung>', 'exec', flags=ast.PyCF_ALLOW_TOP_LEVEL_AWAIT)
assert eval(code, module) is None
assert module['done'] == 3

# === without the flag an await at the top level is a syntax error ===
try:
    eval(compile('await act()', '<rung>', 'exec'), {'act': act})
    assert False, 'expected SyntaxError'
except SyntaxError:
    pass
