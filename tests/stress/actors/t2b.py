import asyncio, sys, hc, time
port = int(sys.argv[1]); n = int(sys.argv[2]); size = int(sys.argv[3]); nact = int(sys.argv[4]); conc = int(sys.argv[5])
async def main():
    t0 = time.monotonic()
    specs = [("GET", f"/big/{i % nact}/{size}") for i in range(n)]
    res = await hc.many(port, specs, concurrency=conc, timeout=20)
    wrong = sum(1 for i, r in enumerate(res) if r[0] == 200 and r[1] != b"x" * size)
    print("big answers only:", hc.tally(res), "wrong", wrong, "elapsed %.2f" % (time.monotonic() - t0))
    # big asks and answers, one actor
    bodies = [(b"%06d-" % i) * (size // 7) for i in range(n)]
    res = await hc.many(port, [("POST", "/echo/0", b) for b in bodies], concurrency=conc, timeout=20)
    wrong = sum(1 for i, r in enumerate(res) if r[0] == 200 and r[1] != bodies[i])
    print("big both ways, one actor:", hc.tally(res), "wrong", wrong, "elapsed %.2f" % (time.monotonic() - t0))
    for mb in (1, 8, 32, 60):
        body = b"0123456789abcdef" * (mb * 65536)
        r = await hc.req(port, "POST", "/echo/1", body, timeout=60)
        print(f"  {mb} MB echo:", r[0], "same" if r[1] == body else f"DIFFERENT len {len(r[1])}", "%.2fs" % r[2])
asyncio.run(main())
