# usage: f1_client.py PORT REQUESTS MS [TIMEOUT]
import asyncio, sys, hc, time
port = int(sys.argv[1]); n = int(sys.argv[2]); ms = int(sys.argv[3])
tmo = float(sys.argv[4]) if len(sys.argv) > 4 else 30
async def main():
    t0 = time.monotonic()
    res = await hc.many(port, [("GET", f"/slow/{ms}")] * n, timeout=tmo)
    print(f"{n} concurrent GET /slow/{ms}:", hc.tally(res), "elapsed %.2f s" % (time.monotonic() - t0),
          "(expected about %.1f s)" % (n * ms / 1000))
    await asyncio.sleep(2)   # every client is gone by now
    res2 = await hc.many(port, [("GET", "/ping")] * 40, concurrency=1, timeout=3)
    print("40 GET /ping afterwards, one at a time:", hc.tally(res2))
    r = await hc.req(port, "GET", "/total", timeout=5)
    print("the actor ran", r[1].decode() if r[0] == 200 else r[0], "asks;", sum(1 for x in res if x[0] == 200), "were answered")
asyncio.run(main())
