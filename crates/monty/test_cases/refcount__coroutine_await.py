# An await holds the coroutine it awaits on the awaiter's stack until the body is
# over, so nothing of it is left behind when the body returns or when it raises.
import asyncio


async def gives(value):
    return [value]


async def raises():
    raise ValueError('from the body')


async def main():
    kept = await gives('one')
    try:
        await raises()
    except ValueError:
        pass
    return kept


held = asyncio.run(main())
# ref-counts={'held': 1}
