# usage: f5_client.py PORT DEEP_REQUESTS
# Ten handlers wait (asleep for 1.5 s); then DEEP_REQUESTS handlers run out of stack, at the same time.
import asyncio, sys, hc, time
port = int(sys.argv[1]); deep_n = int(sys.argv[2])
async def main():
    waiting = [asyncio.create_task(hc.req(port, "GET", "/sleep/1500", timeout=30)) for i in range(10)]
    await asyncio.sleep(0.3)
    deep = await hc.many(port, [("GET", "/depth/100000000")] * deep_n, timeout=30)
    print(f"{deep_n} x GET /depth/100000000 ->", [d[0] for d in deep])
    res = await asyncio.gather(*waiting)
    print("the 10 handlers that were asleep meanwhile:", hc.tally(res))
asyncio.run(main())
