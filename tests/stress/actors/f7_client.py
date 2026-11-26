# usage: f7_client.py PORT first|second
# 40 handlers ask one slow actor (2 s a message); meanwhile 40 others ask a quick actor.
import asyncio, sys, hc, time
port = int(sys.argv[1]); which = sys.argv[2]
async def main():
    slow = [asyncio.create_task(hc.req(port, "GET", f"/{which}/2000", timeout=200)) for i in range(40)]
    await asyncio.sleep(0.3)
    quick = await hc.many(port, [("GET", "/quick")] * 40, timeout=100)
    lat = sorted(q[2] for q in quick)
    print(f"while 40 handlers wait for the `{which}` actor: 40 asks of the quick actor took min %.3f s, median %.3f s, max %.3f s" % (lat[0], lat[20], lat[-1]), hc.tally(quick))
    for t in slow: t.cancel()
asyncio.run(main())
