# === What a field names ===
assert "{} and {}".format(1, "two") == "1 and two"
assert "{0}-{1}-{0}".format("a", "b") == "a-b-a"
assert "{name}!".format(name="sabre") == "sabre!"
assert "{0}{name}".format("x", name="y") == "xy"
assert "".format() == ""
assert "nothing".format() == "nothing"

# === Braces are written twice ===
assert "{{literal}} {}".format(7) == "{literal} 7"
assert "{{}}".format() == "{}"
assert "{{{}}}".format(1) == "{1}"

# === Conversions ===
assert "{!r} {!s}".format("x", "y") == "'x' y"
assert "{0!r}".format([1, 2]) == "[1, 2]"
assert "{!a}".format("café") == "'caf\\xe9'"

# === Specs, which are the f-string mini-language ===
assert "spent ${:.2f} of ${:.2f}".format(1.5, 2.0) == "spent $1.50 of $2.00"
assert "held {:.0f} of {:.0f} tokens".format(3.7, 9.0) == "held 4 of 9 tokens"
assert "{:>6}".format("z") == "     z"
assert "{:<6}|".format("z") == "z     |"
assert "{:^6}|".format("z") == "  z   |"
assert "{:06.2f}".format(3.14159) == "003.14"
assert "{:d}".format(42) == "42"
assert "{:x}".format(255) == "ff"
assert "{:+d}".format(7) == "+7"
assert "{:,}".format(1234567) == "1,234,567"

# === A spec may hold fields of its own ===
assert "{:>{w}}".format("z", w=5) == "    z"
assert "{:{f}{a}{w}}".format("z", f="*", a=">", w=5) == "****z"
assert "{0:.{1}f}".format(3.14159, 3) == "3.142"

# === Reading out of what a field names ===
assert "{0[1]}".format(["a", "b"]) == "b"
assert "{0[k]}".format({"k": "v"}) == "v"
assert "{d[k]}".format(d={"k": "v"}) == "v"
assert "{0[0][1]}".format([["a", "b"]]) == "b"


class Point:
  def __init__(self, x, y):
    self.x = x
    self.y = y


p = Point(1, 2)
assert "{0.x},{0.y}".format(p) == "1,2"
assert "{p.x}".format(p=p) == "1"

# === A template is data, which is the whole reason it exists ===
SAYS = (("usd", "spent ${:.2f} of ${:.2f}"), ("rounds", "bought {:.0f} of {:.0f} rounds"))
spent = {"usd": (1.5, 2.0), "rounds": (3.0, 10.0)}
said = [say.format(*spent[name]) for name, say in SAYS]
assert said == ["spent $1.50 of $2.00", "bought 3 of 10 rounds"]

# === Numbering cannot be mixed ===
try:
  "{} {0}".format(1, 2)
  raise AssertionError("automatic then manual is refused")
except ValueError as e:
  assert "cannot switch from automatic field numbering to manual field specification" in str(e)

try:
  "{0} {}".format(1, 2)
  raise AssertionError("manual then automatic is refused")
except ValueError as e:
  assert "cannot switch from manual field specification to automatic field numbering" in str(e)

# === What is missing ===
try:
  "{}".format()
  raise AssertionError("an index past the arguments is refused")
except IndexError as e:
  assert "Replacement index 0 out of range for positional args tuple" in str(e)

try:
  "{2}".format("a")
  raise AssertionError("a named index past the arguments is refused")
except IndexError:
  pass

try:
  "{nope}".format(name="x")
  raise AssertionError("a name nothing supplied is refused")
except KeyError as e:
  assert "nope" in str(e)

# === Malformed templates ===
try:
  "{".format()
  raise AssertionError("a lone brace is refused")
except ValueError:
  pass

try:
  "}".format()
  raise AssertionError("a lone closing brace is refused")
except ValueError as e:
  assert "Single '}' encountered in format string" in str(e)

try:
  "{!q}".format(1)
  raise AssertionError("an unknown conversion is refused")
except ValueError as e:
  assert "Unknown conversion specifier q" in str(e)

print("str.format holds")
"""
OUTPUT:
str.format holds
"""
