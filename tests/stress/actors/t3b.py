import asyncio, sys, hc, time
port = int(sys.argv[1]); n = int(sys.argv[2]); ms = int(sys.argv[3])
async def main():
    tasks = [asyncio.create_task(hc.req(port, "GET", f"/slow/0/{ms}", timeout=120)) for i in range(n)]
    await asyncio.sleep(0.3)
    quick = await hc.many(port, [("GET", "/count")] * 30, timeout=60)
    lat = sorted(q[2] for q in quick)
    print("30 concurrent /count (a quick actor) while %d handlers wait for one slow actor: min %.3f median %.3f max %.3f s" % (n, lat[0], lat[15], lat[-1]), hc.tally(quick))
    res = await asyncio.gather(*tasks)
    print("slow:", hc.tally(res), "max %.2f" % max(r[2] for r in res))
asyncio.run(main())
