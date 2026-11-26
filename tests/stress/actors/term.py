import asyncio, sys, hc, time, os, signal
port = int(sys.argv[1]); sup = int(sys.argv[2]); n = int(sys.argv[3]); ms = int(sys.argv[4]); nact = int(sys.argv[5])
async def main():
    tasks = [asyncio.create_task(hc.req(port, "GET", f"/slow/{i % nact}/{ms}", timeout=60)) for i in range(n)]
    await asyncio.sleep(0.3)
    t0 = time.monotonic()
    os.kill(sup, signal.SIGTERM)
    res = await asyncio.gather(*tasks)
    print("in flight at SIGTERM:", hc.tally(res), "last answer %.2fs after SIGTERM" % (time.monotonic() - t0))
asyncio.run(main())
