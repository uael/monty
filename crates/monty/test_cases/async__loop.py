# The loop a coroutine runs inside answers that it is running.
import asyncio


async def running():
    loop = asyncio.get_running_loop()
    return (loop.is_running(), loop.is_closed())


assert asyncio.run(running()) == (True, False)


# === the loop is the same object throughout one coroutine ===
async def same():
    loop = asyncio.get_running_loop()
    return loop is loop


assert asyncio.run(same()) is True
