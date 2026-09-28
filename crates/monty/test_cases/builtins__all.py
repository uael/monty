import builtins

# === a builtin function reached as an attribute ===
assert builtins.len([1, 2]) == 2
assert builtins.abs(-3) == 3
assert builtins.sorted([2, 1]) == [1, 2]

# === a builtin type reached as an attribute ===
assert builtins.int('7') == 7
assert builtins.list((1, 2)) == [1, 2]
assert builtins.str(7) == '7'

# === a builtin exception reached as an attribute ===
assert str(builtins.ValueError('x')) == 'x'
assert builtins.TypeError is TypeError

# === it is the same object a bare name resolves to ===
assert builtins.len is len
assert builtins.int is int
assert builtins.ValueError is ValueError

# === looked up by name, which is what the module is for ===
names = vars(builtins)
assert names['abs'](-3) == 3
assert names['TypeError'] is TypeError
assert names['list']((1,)) == [1]

# === the three constants ===
assert names['None'] is None
assert names['True'] is True
assert names['False'] is False

# === a name that is no builtin ===
try:
    builtins.nosuchname
    raise AssertionError('expected AttributeError')
except AttributeError:
    pass
