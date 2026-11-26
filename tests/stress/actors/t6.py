import asyncio, sys, hc, collections, subprocess
port = int(sys.argv[1]); sup = sys.argv[2]
def workers(): return subprocess.run(["pgrep", "-P", sup], capture_output=True, text=True).stdout.split()
async def main():
    w0 = workers()
    specs = []
    for i in range(30000):
        specs.append(("GET", ["/boom", "/other/add/1", "/ping"][i % 3]))
    res = await hc.many(port, specs, concurrency=30, timeout=20)
    by = collections.defaultdict(collections.Counter)
    for s, r in zip(specs, res): by[s[1]][r[0]] += 1
    for k, v in by.items(): print(" ", k, dict(v))
    ok = sum(1 for s, r in zip(specs, res) if s[1] == "/other/add/1" and r[0] == 200)
    vals = [int(r[1]) for s, r in zip(specs, res) if s[1] == "/other/add/1" and r[0] == 200]
    print("  other counter:", (await hc.req(port, "GET", "/other/get"))[1], "successes", ok, "dups", len(vals) - len(set(vals)))
    w1 = workers()
    print("  workers before", w0, "after", w1)
    res = await hc.many(port, [("GET", "/other/add/1")] * 100, concurrency=30, timeout=10)
    print("  afterwards:", hc.tally(res))
asyncio.run(main())
