import asyncio, sys, hc, time
port = int(sys.argv[1]); mode = sys.argv[2]; n = int(sys.argv[3]); ms = int(sys.argv[4])
async def main():
    if mode == "distinct":
        specs = [("GET", f"/slow/{i}/{ms}") for i in range(n)]
    else:
        specs = [("GET", f"/slow/0/{ms}") for i in range(n)]
    tasks = [asyncio.create_task(hc.req(port, s[0], s[1], timeout=120)) for s in specs]
    await asyncio.sleep(0.3)
    quick = []
    for i in range(5):
        quick.append(await hc.req(port, "GET", "/count", timeout=60))
        quick.append(await hc.req(port, "GET", "/ping", timeout=60))
    print(mode, "quick /count latencies:", ["%.3f" % q[2] for q in quick[0::2]], [q[0] for q in quick[0::2]])
    print(mode, "/ping latencies:", ["%.3f" % q[2] for q in quick[1::2]])
    res = await asyncio.gather(*tasks)
    print(mode, "slow:", hc.tally(res), "max %.2f" % max(r[2] for r in res))
asyncio.run(main())
