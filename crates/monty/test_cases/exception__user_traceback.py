class Halt(Exception):
    pass


def stop():
    raise Halt('nothing left to do')


stop()
"""
TRACEBACK:
Traceback (most recent call last):
  File "exception__user_traceback.py", line 9, in <module>
    stop()
    ~~~~~~
  File "exception__user_traceback.py", line 6, in stop
    raise Halt('nothing left to do')
Halt: nothing left to do
"""
