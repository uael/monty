# A traceback names the sandbox class an exception was raised from, not the
# builtin it descends from.


class Refused(Exception):
    pass


def refuse():
    raise Refused('why')


refuse()
"""
TRACEBACK:
Traceback (most recent call last):
  File "exception__user_traceback.py", line 13, in <module>
    refuse()
    ~~~~~~~~
  File "exception__user_traceback.py", line 10, in refuse
    raise Refused('why')
Refused: why
"""
