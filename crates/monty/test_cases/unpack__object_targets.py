class Box:
    def __init__(self):
        self.a = 0
        self.b = 0
        self.c = 0


# === Attribute targets in tuple unpacking ===
box = Box()
box.a, box.b = 1, 2
assert (box.a, box.b) == (1, 2)

box.a, box.b, box.c = [10, 20, 30]
assert (box.a, box.b, box.c) == (10, 20, 30)

# === Subscript targets in tuple unpacking ===
d = {}
d['x'], d['y'] = 'first', 'second'
assert d == {'x': 'first', 'y': 'second'}

lst = [0, 0, 0]
lst[0], lst[2] = 'a', 'b'
assert lst == ['a', 0, 'b']

# === Mixed name / attribute / subscript ===
plain, box.a, d['z'] = 1, 2, 3
assert plain == 1
assert box.a == 2
assert d['z'] == 3

# === Nested patterns ===
p, (box.b, lst[1]) = 'p', (5, 6)
assert p == 'p'
assert box.b == 5
assert lst[1] == 6

[box.c], d['w'] = [7], 8
assert box.c == 7
assert d['w'] == 8

# === Starred targets still work alongside object targets ===
box.a, *rest = [1, 2, 3, 4]
assert box.a == 1
assert rest == [2, 3, 4]

first, *middle, d['last'] = [1, 2, 3, 4]
assert first == 1
assert middle == [2, 3]
assert d['last'] == 4

# === Evaluation order: the right-hand side runs before any store ===
order = []


def note(tag, value):
    order.append(tag)
    return value


holder = Box()
holder.a, holder.b = note('rhs1', 1), note('rhs2', 2)
assert order == ['rhs1', 'rhs2']
assert (holder.a, holder.b) == (1, 2)

# === `for` loop targets ===
seen = []
counter = Box()
for counter.a in range(3):
    seen.append(counter.a)
assert seen == [0, 1, 2]
assert counter.a == 2

slots = {}
for slots['k'] in 'ab':
    pass
assert slots == {'k': 'b'}

pairs = Box()
for pairs.a, pairs.b in [(1, 2), (3, 4)]:
    pass
assert (pairs.a, pairs.b) == (3, 4)


# === `with` statement targets ===
class Ctx:
    def __enter__(self):
        return 7

    def __exit__(self, exc_type, exc, tb):
        return False


held = Box()
with Ctx() as held.a:
    assert held.a == 7
assert held.a == 7

# === Chained assignment with object targets ===
left = Box()
right = {}
name = left.a = right['k'] = 'shared'
assert name == 'shared'
assert left.a == 'shared'
assert right['k'] == 'shared'

# === Augmented forms are unchanged ===
acc = Box()
acc.a = 1
acc.a += 4
assert acc.a == 5

# === Object targets inside a closure still capture correctly ===
outer = Box()


def store():
    outer.a, outer.b = 'x', 'y'


store()
assert (outer.a, outer.b) == ('x', 'y')
