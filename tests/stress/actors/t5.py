import asyncio, sys, hc, time
port = int(sys.argv[1]); deep_path = sys.argv[2]
async def main():
    waiting = [asyncio.create_task(hc.req(port, "GET", "/slow/40", timeout=30)) for i in range(30)]
    waiting += [asyncio.create_task(hc.req(port, "GET", "/sleep/800", timeout=30)) for i in range(10)]
    await asyncio.sleep(0.2)
    t0 = time.monotonic()
    deep = await hc.many(port, [("GET", deep_path)] * 4, timeout=30)
    print(deep_path, "->", [(d[0], "%.2f" % d[2]) for d in deep])
    res = await asyncio.gather(*waiting)
    print("handlers that were waiting (30 on an actor, 10 asleep):", hc.tally(res[:30]), hc.tally(res[30:]), "max %.2f" % max(r[2] for r in res))
    await asyncio.sleep(0.5)
    after = await hc.many(port, [("GET", "/slow/1")] * 40, concurrency=20, timeout=10)
    print("afterwards 40 /slow/1:", hc.tally(after), "/get:", (await hc.req(port, "GET", "/get", timeout=5))[:2])
asyncio.run(main())
