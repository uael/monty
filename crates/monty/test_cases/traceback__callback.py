def key(x):
    raise ValueError(f'no key for {x}')


sorted([1, 2], key=key)
"""
TRACEBACK:
Traceback (most recent call last):
  File "traceback__callback.py", line 5, in <module>
    sorted([1, 2], key=key)
    ~~~~~~~~~~~~~~~~~~~~~~~
  File "traceback__callback.py", line 2, in key
    raise ValueError(f'no key for {x}')
ValueError: no key for 1
"""
