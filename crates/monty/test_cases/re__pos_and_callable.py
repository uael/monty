# Tests for the re module: a callable replacement, and the pos and endpos of re.Pattern

import re

# === re.sub() with a callable replacement ===
assert re.sub('a', lambda m: m.group().upper(), 'banana') == 'bAnAnA', 'callable repl'
assert re.sub('(a)(n)?', lambda m: m.group(2) or '-', 'banana') == 'bnn-', 'callable repl reads groups'
assert re.sub('a', lambda m: None, 'banana') == 'bnn', 'None removes the match'
assert re.sub('a', lambda m: str(m.start()), 'banana', count=2) == 'b1n3na', 'callable repl with count'
assert re.sub('a', lambda m: 'x', 'banana', count=-1) == 'banana', 'negative count calls nothing'
assert re.sub('x*', lambda m: '-', 'abc') == '-a-b-c-', 'callable repl on empty matches'
assert re.sub('é', lambda m: m.group() + m.string[:1], 'aéb') == 'aéab', 'callable repl on non-ASCII text'
assert re.sub('a', lambda m: m.group(0) * 2, 'ab') == 'aab', 'repl doubles each match'


def shout(m):
    return m.group(1).upper() + '!'


assert re.sub(r'(\w+)', shout, 'hi you') == 'HI! YOU!', 'a def as repl'

calls = []


def wrong(m):
    calls.append(m.start())
    return 1


try:
    re.sub('a', wrong, 'banana')
    assert False, 'an int from repl should raise'
except TypeError as e:
    assert str(e) == 'sequence item 1: expected str instance, int found', str(e)
assert calls == [1, 3, 5], 'each call runs before the error'

try:
    re.sub('a', lambda m: 1, 'a')
    assert False, 'an int from repl should raise'
except TypeError as e:
    assert str(e) == 'sequence item 0: expected str instance, int found', str(e)


def boom(m):
    raise ValueError('boom')


try:
    re.sub('a', boom, 'banana')
    assert False, 'an error of repl should propagate'
except ValueError as e:
    assert str(e) == 'boom', str(e)

try:
    re.sub('a', 1, 'a')
    assert False, 'an int repl should raise'
except TypeError as e:
    assert str(e) == 'decoding to str: need a bytes-like object, int found', str(e)

# === Pattern.sub() with a callable replacement ===
p = re.compile(r'\d+')
assert p.sub(lambda m: str(int(m.group()) * 2), 'a1 b22 c333') == 'a2 b44 c666', 'Pattern.sub callable'
assert p.sub(lambda m: '#', 'a1 b22', 1) == 'a# b22', 'Pattern.sub callable with count'
assert p.sub(lambda m: '#', 'a1', -1) == 'a1', 'Pattern.sub callable with negative count'
try:
    p.sub(lambda m: 2.5, 'x1y')
    assert False, 'a float from repl should raise'
except TypeError as e:
    assert str(e) == 'sequence item 1: expected str instance, float found', str(e)

# === re.split() with groups ===
assert re.split(r'(-)(x)?', 'a-b-c') == ['a', '-', None, 'b', '-', None, 'c'], 'split keeps groups'

# === Pattern.search() with pos and endpos ===
p = re.compile('a')
assert p.search('banana', 2).span() == (3, 4), 'search from pos'
assert p.search('banana', 2, 3) is None, 'search to endpos'
assert p.search('banana', pos=4, endpos=100).span() == (5, 6), 'search with keywords past the end'
assert p.search(string='ba', pos=1).span() == (1, 2), 'search with string as keyword'
assert p.search('banana', -5).span() == (1, 2), 'negative pos counts as 0'
assert p.search('banana', 5, 2) is None, 'endpos before pos'
assert re.compile('').search('abc', 2, 1) is None, 'empty pattern with endpos before pos'
assert re.compile('^a').search('banana', 1) is None, '^ does not match at pos'
assert re.compile('(?<=n)a').search('banana', 3).span() == (3, 4), 'lookbehind sees before pos'
assert re.compile('an$').search('banana', 0, 5).span() == (3, 5), '$ matches at endpos'
assert re.compile('na(?=n)').search('banana', 0, 4) is None, 'lookahead stops at endpos'
assert re.compile('é').search('ééa', 1).span() == (1, 2), 'pos counts characters'
assert re.compile('a').search('ééa', 0, 2) is None, 'endpos counts characters'
m = p.search('banana', 2)
assert m.string == 'banana' and m.group() == 'a', 'the match holds the whole subject'

# === Pattern.match() and Pattern.fullmatch() with pos and endpos ===
assert p.match('banana', 1).span() == (1, 2), 'match at pos'
assert p.match('banana', 2) is None, 'match fails at pos'
assert re.compile('b|ab').match('xab', 1).span() == (1, 3), 'match at pos tries each branch'
assert re.compile('(?<=n)a').match('banana', 3).span() == (3, 4), 'match lookbehind sees before pos'
assert re.compile(r'\Aa').match('banana', 1) is None, r'\A does not match at pos'
assert re.compile('').match('abc', 2, 1).span() == (2, 2), 'empty match with endpos before pos'
assert re.compile('a|an').fullmatch('banana', 3, 5).span() == (3, 5), 'fullmatch to endpos'
assert re.compile('an').fullmatch('banana', 1) is None, 'fullmatch fails before the end'
assert re.compile('ana').fullmatch('banana', 3).span() == (3, 6), 'fullmatch from pos'
assert re.compile('a').fullmatch('ba', 1, 2).span() == (1, 2), 'fullmatch from pos to endpos'

# === Pattern.findall() and Pattern.finditer() with pos and endpos ===
assert p.findall('banana', 2, 5) == ['a'], 'findall in bounds'
assert p.findall('banana', 2) == ['a', 'a'], 'findall from pos'
assert p.findall('banana', 5, 2) == [], 'findall with endpos before pos'
assert re.compile(r'\b').findall('ab cd', 1) == ['', '', ''], 'findall of empty matches from pos'
assert re.compile('(a)(n)').findall('banana', 2) == [('a', 'n')], 'findall of groups from pos'
assert re.compile('(a)n').findall('banana', 2) == ['a'], 'findall of one group from pos'
assert [m.span() for m in p.finditer('banana', 2)] == [(3, 4), (5, 6)], 'finditer from pos'
assert [m.span() for m in p.finditer('banana', 0, 4)] == [(1, 2), (3, 4)], 'finditer to endpos'
assert list(p.finditer('banana', 5, 2)) == [], 'finditer with endpos before pos'

# === pos and endpos errors ===
try:
    p.search('banana', 'x')
    assert False, 'a str pos should raise'
except TypeError as e:
    assert str(e) == "'str' object cannot be interpreted as an integer", str(e)
try:
    p.search('banana', 1.5)
    assert False, 'a float pos should raise'
except TypeError as e:
    assert str(e) == "'float' object cannot be interpreted as an integer", str(e)
try:
    p.search('banana', 0, 1, 2)
    assert False, 'four arguments should raise'
except TypeError as e:
    assert str(e) == 'search() takes at most 3 arguments (4 given)', str(e)
try:
    p.match()
    assert False, 'no argument should raise'
except TypeError as e:
    assert str(e) == "match() missing required argument 'string' (pos 1)", str(e)
