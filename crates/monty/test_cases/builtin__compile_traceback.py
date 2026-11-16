exec(compile('def f():\n    raise ValueError("boom")\nf()', 'rung3', 'exec'))
"""
TRACEBACK:
Traceback (most recent call last):
  File "builtin__compile_traceback.py", line 1, in <module>
    exec(compile('def f():\n    raise ValueError("boom")\nf()', 'rung3', 'exec'))
    ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
  File "rung3", line 3, in <module>
  File "rung3", line 2, in f
ValueError: boom
"""
