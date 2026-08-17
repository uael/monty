# An exception escaping the `try` body is re-raised by the implicit cleanup the
# compiler emits when the handler does not match it. The traceback reports the
# raise on line 6, not the cleanup that carried it out of the handler.
def check(value):
    try:
        raise ValueError('boom')
        value = 2
    except TypeError:
        value = 3
    return value


check(1)


"""
TRACEBACK:
Traceback (most recent call last):
  File "traceback__reraise_in_handler.py", line 13, in <module>
    check(1)
    ~~~~~~~~
  File "traceback__reraise_in_handler.py", line 6, in check
    raise ValueError('boom')
ValueError: boom
"""
