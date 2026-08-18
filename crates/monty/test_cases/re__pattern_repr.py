# A pattern's repr accounts for every flag it was given: the ones CPython has a
# name for by that name, in CPython's order, and the rest as themselves.

import re

# === No flags, and the one flag that is a default ===
assert repr(re.compile('a')) == "re.compile('a')"
assert repr(re.compile('a', 0)) == "re.compile('a')"
# re.UNICODE is a str pattern's default, so it is left unnamed.
assert repr(re.compile('a', 32)) == "re.compile('a')"

# === One named flag at a time ===
assert repr(re.compile('a', re.IGNORECASE)) == "re.compile('a', re.IGNORECASE)"
assert repr(re.compile('a', re.MULTILINE)) == "re.compile('a', re.MULTILINE)"
assert repr(re.compile('a', re.DOTALL)) == "re.compile('a', re.DOTALL)"
assert repr(re.compile('a', re.ASCII)) == "re.compile('a', re.ASCII)"
# Two more CPython names that Monty does not expose as attributes but still
# reports when given as a number.
assert repr(re.compile('a', 64)) == "re.compile('a', re.VERBOSE)"
assert repr(re.compile('a', 128)) == "re.compile('a', re.DEBUG)"

# The short aliases name the same bits.
assert repr(re.compile('a', re.I)) == "re.compile('a', re.IGNORECASE)"
assert repr(re.compile('a', re.M)) == "re.compile('a', re.MULTILINE)"
assert repr(re.compile('a', re.S)) == "re.compile('a', re.DOTALL)"
assert repr(re.compile('a', re.A)) == "re.compile('a', re.ASCII)"

# === Several at once, in CPython's order rather than the order given ===
assert repr(re.compile('a', re.MULTILINE | re.IGNORECASE)) == "re.compile('a', re.IGNORECASE|re.MULTILINE)"
assert repr(re.compile('a', re.DOTALL | re.IGNORECASE)) == "re.compile('a', re.IGNORECASE|re.DOTALL)"
assert repr(re.compile('a', 2 | 8 | 16)) == "re.compile('a', re.IGNORECASE|re.MULTILINE|re.DOTALL)"
assert repr(re.compile('a', 2 | 64)) == "re.compile('a', re.IGNORECASE|re.VERBOSE)"
assert repr(re.compile('a', 2 | 8 | 16 | 64 | 128 | 256)) == (
    "re.compile('a', re.IGNORECASE|re.MULTILINE|re.DOTALL|re.VERBOSE|re.DEBUG|re.ASCII)"
)

# === A bit with no name is printed as itself, after the named ones ===
assert repr(re.compile('a', 1)) == "re.compile('a', 0x1)"
assert repr(re.compile('a', 512)) == "re.compile('a', 0x200)"
assert repr(re.compile('a', 1024)) == "re.compile('a', 0x400)"
assert repr(re.compile('a', 3)) == "re.compile('a', re.IGNORECASE|0x1)"
assert repr(re.compile('a', 2 | 8 | 1)) == "re.compile('a', re.IGNORECASE|re.MULTILINE|0x1)"
assert repr(re.compile('a', 256 | 512 | 1)) == "re.compile('a', re.ASCII|0x201)"
# The unnamed default is not counted among the leftovers either.
assert repr(re.compile('a', 32 | 1)) == "re.compile('a', 0x1)"
assert repr(re.compile('a', 32 | 64)) == "re.compile('a', re.VERBOSE)"

# === The pattern string is quoted the way any string is ===
assert repr(re.compile("a'b")) == 're.compile("a\'b")'
assert repr(re.compile('a\\d')) == "re.compile('a\\\\d')"
assert repr(re.compile('a', re.IGNORECASE)) == str(re.compile('a', re.IGNORECASE))
