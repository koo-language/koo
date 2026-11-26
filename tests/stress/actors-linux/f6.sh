#!/bin/sh
# F6: while one worker computes with a long message (or a long answer) half way, the
# other worker's asks wait: of every actor (WHAT=message) or of that actor (WHAT=answer).
cd /x/min; . ./env.sh
WORKERS=2 start ./m
echo "supervisor $SUP, workers $(workers)"
WHAT=${WHAT:-message} python3 - <<'PY'
import L, time, asyncio, os
what = os.environ["WHAT"]
async def probe(seconds, path):
    worst = 0; n = 0; t_end = time.time() + seconds
    while time.time() < t_end:
        t = time.time()
        s, b = await L.one("GET", path, timeout=60)
        worst = max(worst, time.time() - t); n += 1
        await asyncio.sleep(0.02)
    return "%s: %d answered in 4 s, the slowest took %.2fs" % (path, n, worst)
async def scene(long):
    t = time.time()
    jobs = []
    if long and what == "message":
        jobs += [asyncio.ensure_future(L.one("POST", "/echo/%d" % (20 + i), b"z" * 1000000, timeout=60)) for i in range(4)]
    if long and what == "answer":
        jobs += [asyncio.ensure_future(L.one("GET", "/big/%d/1000000" % (20 + i), timeout=60)) for i in range(4)]
    jobs += [asyncio.ensure_future(L.one("GET", "/spin/4000000000", timeout=60))]
    await asyncio.sleep(0.3)
    p = await asyncio.gather(probe(4, "/ping"), probe(4, "/next/9"), probe(4, "/next/20"))
    r = await asyncio.gather(*jobs)
    print(("one worker computes for %.0f s, with four 1 MB %ss half way:" % (time.time() - t, what)) if long else ("one worker computes for %.0f s:" % (time.time() - t)))
    for line in p: print("    " + line)
for k in range(3):
    asyncio.run(scene(False))
    asyncio.run(scene(True))
PY
