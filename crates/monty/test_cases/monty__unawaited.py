# /// monty
# xfail = ['cpython']
# ///
# `monty.unawaited()`: the names of the coroutines freed with their body never
# started since the last call, each once, which CPython warns of as "coroutine
# ... was never awaited". CPython has no `monty` module, so this runs on Monty alone.
import asyncio
import gc

import monty


async def hire():
    return 1


async def fire():
    return 2


def plain():
    return 3


def counts():
    yield 1


# === a call that nothing awaits is told once, and then forgotten ===
assert monty.unawaited() == []
hire()
assert monty.unawaited() == ['hire']
assert monty.unawaited() == []

# === each function once, in the order of its first free ===
for _ in range(3):
    hire()
fire()
hire()
assert monty.unawaited() == ['hire', 'fire']

# === a coroutine that is held is not freed, and is told when it goes ===
held = hire()
assert monty.unawaited() == []
del held
assert monty.unawaited() == ['hire']

# === a coroutine that a cycle holds is told when the collector frees the cycle ===
box = [fire()]
box.append(box)
del box
assert monty.unawaited() == []
gc.collect()
assert monty.unawaited() == ['fire']


# === an awaited coroutine, a gathered one, a closed one, a plain call and a generator are not told ===
async def main():
    await hire()
    return await asyncio.gather(hire(), fire())


assert asyncio.run(main()) == [1, 2]
closed = hire()
closed.close()
del closed
plain()
counts()
assert monty.unawaited() == []
