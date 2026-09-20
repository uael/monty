import sys

# === a module is built once per session ===
import sys as again

assert sys is again
assert sys.modules['sys'] is sys

# === importing twice gives the same object ===
import json
import json as json_again

assert json is json_again

# === every import is remembered ===
assert 'json' in sys.modules
assert sys.modules['json'] is json


# === a module a program puts there is what import finds ===
class Held:
    pass


mine = Held()
mine.answer = 42
sys.modules['mine'] = mine

import mine as bound

assert bound is mine
assert bound.answer == 42

from mine import answer

assert answer == 42

# === built from source and registered ===
made = Held()
namespace = {}
exec('VALUE = 7\n\ndef twice(x):\n    return x * 2\n', namespace)
made.VALUE = namespace['VALUE']
made.twice = namespace['twice']
sys.modules['helper'] = made

from helper import VALUE, twice

assert VALUE == 7
assert twice(5) == 10

# === a name nothing bound ===
try:
    import nosuchmodule

    raise AssertionError('expected ModuleNotFoundError')
except ModuleNotFoundError as exc:
    assert str(exc) == "No module named 'nosuchmodule'"
