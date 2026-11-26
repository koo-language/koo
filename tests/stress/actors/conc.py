import asyncio, sys, hc, time, collections
port = int(sys.argv[1])
which = sys.argv[2:] or ["A", "B", "C", "D", "E", "F"]
async def get(path, timeout=30):
    return await hc.req(port, "GET", path, timeout=timeout)
def show(name, res):
    print(name, hc.tally(res), "max %.2fs" % max(r[2] for r in res))
async def A():
    # panic while others wait for the same actor
    await get("/boom")  # reset to 0
    specs = [("GET", "/slow/30")] * 15 + [("GET", "/slowboom/30")] + [("GET", "/slow/30")] * 15
    tasks = []
    for s in specs:
        tasks.append(asyncio.create_task(hc.req(port, s[0], s[1], timeout=30)))
        await asyncio.sleep(0.002)
    res = await asyncio.gather(*tasks)
    show("A", res)
    vals = [int(r[1]) for r in res if r[0] == 200]
    print("  values:", vals, "| final:", (await get("/get"))[1])
async def B():
    specs = []
    for i in range(300):
        specs.append(("GET", ["/cycle", "/pingecho", "/nested_boom", "/nested/1", "/selfie", "/selfie/one", "/inner/slow/2", "/nested_slow/2"][i % 8]))
    res = await hc.many(port, specs, concurrency=40, timeout=30)
    by = collections.defaultdict(collections.Counter)
    for s, r in zip(specs, res):
        by[s[1]][r[0]] += 1
    for k, v in by.items():
        print("  B", k, dict(v))
    print("  B after:", (await get("/pingecho"))[:2], (await get("/nested/1"))[:2], (await get("/selfie/one"))[:2])
async def C():
    tasks = []
    for i in range(20):
        tasks.append(asyncio.create_task(get("/victim/slow/100")))
        await asyncio.sleep(0.002)
    await asyncio.sleep(0.25)
    t0 = time.monotonic()
    stop = await get("/victim/stop")
    print("  C stop:", stop[:2], "took %.2f" % (time.monotonic() - t0))
    res = await asyncio.gather(*tasks)
    show("C", res)
    print("  C values:", sorted(int(r[1]) for r in res if r[0] == 200))
async def D():
    before = (await get("/log"))[1].split()
    n = 3000
    res = await hc.many(port, [("GET", f"/log/{i}") for i in range(n)], concurrency=60)
    show("D log", res)
    after = (await get("/log"))[1].split()
    print("  D log count +%d (want %d) sum +%d (want %d)" % (int(after[0]) - int(before[0]), n, int(after[1]) - int(before[1]), n * (n - 1) // 2))
    b = int((await get("/printed"))[1])
    res = await hc.many(port, [("GET", "/send/3")] * 300, concurrency=60, timeout=60)
    show("D send", res)
    for _ in range(100):
        a = int((await get("/printed"))[1])
        if a - b >= 300: break
        await asyncio.sleep(0.1)
    print("  D printed +%d (want 300)" % (a - b))
    b = a
    res = await hc.many(port, [("GET", "/try/3")] * 300, concurrency=60, timeout=60)
    show("D try", res)
    yes = sum(1 for r in res if r[1] == b"true"); no = sum(1 for r in res if r[1] == b"false")
    await asyncio.sleep(1.0)
    a = int((await get("/printed"))[1])
    print("  D try true=%d false=%d printed +%d" % (yes, no, a - b))
async def E():
    n = 3000
    specs = [("GET", f"/room/e{i % 40}") for i in range(n)]
    res = await hc.many(port, specs, concurrency=60)
    show("E", res)
    seen = collections.defaultdict(list)
    for r in res:
        if r[0] == 200:
            name, k = r[1].decode().split(":"); seen[name].append(int(k))
    bad = {k: len(v) for k, v in seen.items() if sorted(v) != list(range(1, len(v) + 1))}
    print("  E rooms", len(seen), "rooms with gaps/dups:", bad, "rooms known:", (await get("/rooms"))[1])
async def F():
    n = 5000
    res = await hc.many(port, [("GET", f"/temp/{i}") for i in range(n)], concurrency=60)
    show("F", res)
    wrong = sum(1 for i, r in enumerate(res) if r[0] == 200 and r[1] != f"{i} {i+1}".encode())
    print("  F wrong:", wrong)
async def main():
    for w in which:
        t0 = time.monotonic()
        await globals()[w]()
        print("   [%s took %.1fs]" % (w, time.monotonic() - t0))
asyncio.run(main())
