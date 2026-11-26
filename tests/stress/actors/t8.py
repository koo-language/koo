import asyncio, sys, hc, time
port = int(sys.argv[1]); which = sys.argv[2]; nq = int(sys.argv[3])
async def main():
    slow = [asyncio.create_task(hc.req(port, "GET", f"/{which}/4000", timeout=200)) for i in range(40)]
    await asyncio.sleep(0.3)
    tasks = []
    for i in range(nq):
        tasks.append(asyncio.create_task(hc.req(port, "GET", "/quick", timeout=100)))
        await asyncio.sleep(0.002)
    quick = await asyncio.gather(*tasks)
    lat = sorted(q[2] for q in quick)
    print(f"{nq} spaced asks of the quick actor: min %.3f max %.3f" % (lat[0], lat[-1]), hc.tally(quick))
    for t in slow: t.cancel()
asyncio.run(main())
