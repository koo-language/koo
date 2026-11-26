import asyncio, sys, hc, time
port = int(sys.argv[1]); n = int(sys.argv[2]); ms = int(sys.argv[3]); nact = int(sys.argv[4]) if len(sys.argv) > 4 else 1
tmo = float(sys.argv[5]) if len(sys.argv) > 5 else 60
async def main():
    t0 = time.monotonic()
    specs = [("GET", f"/slow/{i % nact}/{ms}") for i in range(n)]
    res = await hc.many(port, specs, timeout=tmo)
    print("slow:", hc.tally(res), "elapsed %.2f" % (time.monotonic() - t0), "p50 %.2f p99 %.2f" % (hc.pct(res, .5), hc.pct(res, .99)))
    vals = sorted(int(r[1]) for r in res if r[0] == 200)
    # afterwards, are workers alive?
    res2 = await hc.many(port, [("GET", "/ping")] * 20, timeout=5)
    print("ping after:", hc.tally(res2))
    res3 = await hc.many(port, [("GET", "/count")] * 20, timeout=5)
    print("count after:", hc.tally(res3))
    tot = 0
    for i in range(nact):
        r = await hc.req(port, "GET", f"/slowtotal/{i}", timeout=5)
        tot += int(r[1]) if r[0] == 200 else 0
    print("ok answers", len(vals), "actor total", tot)
asyncio.run(main())
