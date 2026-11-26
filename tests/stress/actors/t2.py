import asyncio, sys, hc, time, os, hashlib
port = int(sys.argv[1]); n = int(sys.argv[2]); size = int(sys.argv[3]); nact = int(sys.argv[4]); conc = int(sys.argv[5])
tmo = float(sys.argv[6]) if len(sys.argv) > 6 else 60
async def main():
    t0 = time.monotonic()
    bodies = []
    for i in range(n):
        tok = f"tok{i:06d}-".encode()
        bodies.append((tok * (size // len(tok) + 1))[:size])
    specs = [("POST", f"/echo/{i % nact}", bodies[i]) for i in range(n)]
    res = await hc.many(port, specs, concurrency=conc, timeout=tmo)
    wrong = sum(1 for i, r in enumerate(res) if r[0] == 200 and r[1] != bodies[i])
    print("echo:", hc.tally(res), "wrong", wrong, "elapsed %.2f" % (time.monotonic() - t0), "p50 %.2f p99 %.2f" % (hc.pct(res, .5), hc.pct(res, .99)))
    specs = [("GET", f"/big/{i % nact}/{size}") for i in range(n)]
    res = await hc.many(port, specs, concurrency=conc, timeout=tmo)
    wrong = sum(1 for i, r in enumerate(res) if r[0] == 200 and r[1] != b"x" * size)
    print("big:", hc.tally(res), "wrong", wrong, "elapsed %.2f" % (time.monotonic() - t0))
    res2 = await hc.many(port, [("GET", "/count")] * 20, timeout=5)
    print("count after:", hc.tally(res2))
asyncio.run(main())
