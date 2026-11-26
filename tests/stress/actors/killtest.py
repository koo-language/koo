# usage: killtest.py PORT SUPERVISOR_PID SECONDS KILL_INTERVAL [CONC]
import asyncio, sys, hc, time, os, signal, subprocess, random, collections
port = int(sys.argv[1]); sup = int(sys.argv[2]); secs = float(sys.argv[3]); every = float(sys.argv[4])
conc = int(sys.argv[5]) if len(sys.argv) > 5 else 40
def workers():
    out = subprocess.run(["pgrep", "-P", str(sup)], capture_output=True, text=True).stdout.split()
    return [int(x) for x in out]
def supstat():
    fds = len(subprocess.run(["lsof", "-p", str(sup)], capture_output=True, text=True).stdout.splitlines())
    rss = subprocess.run(["ps", "-o", "rss=", "-p", str(sup)], capture_output=True, text=True).stdout.strip()
    thr = len(subprocess.run(["ps", "-M", "-p", str(sup)], capture_output=True, text=True).stdout.splitlines()) - 1
    return f"fds={fds} rss={rss}KB threads={thr}"
stop = False
results = collections.defaultdict(collections.Counter)
counts = []
attempts = 0
echo_wrong = 0
async def client(k):
    global attempts, echo_wrong
    rnd = random.Random(k)
    while not stop:
        kind = rnd.choice(["count", "count", "slow", "echo", "ping"])
        if kind == "count":
            attempts += 1
            r = await hc.req(port, "GET", "/count", timeout=20)
            if r[0] == 200: counts.append(int(r[1]))
        elif kind == "slow":
            r = await hc.req(port, "GET", f"/slow/{rnd.randrange(16)}/{rnd.randrange(8)}", timeout=20)
        elif kind == "echo":
            body = (b"%06d-" % rnd.randrange(10**6)) * rnd.randrange(1, 500)
            r = await hc.req(port, "POST", f"/echo/{rnd.randrange(16)}", body, timeout=20)
            if r[0] == 200 and r[1] != body: echo_wrong += 1
        else:
            r = await hc.req(port, "GET", "/ping", timeout=20)
        results[kind][r[0]] += 1
async def killer():
    kills = 0
    while not stop:
        await asyncio.sleep(every)
        ws = workers()
        if ws:
            try:
                os.kill(random.choice(ws), signal.SIGKILL); kills += 1
            except ProcessLookupError: pass
    return kills
async def main():
    global stop
    print("before:", supstat(), "workers", len(workers()))
    tasks = [asyncio.create_task(client(k)) for k in range(conc)]
    kt = asyncio.create_task(killer())
    await asyncio.sleep(secs)
    stop = True
    kills = await kt
    await asyncio.gather(*tasks)
    print("kills:", kills)
    for k, v in results.items(): print("  ", k, dict(v))
    total = int((await hc.req(port, "GET", "/total", timeout=10))[1] or -1)
    dups = len(counts) - len(set(counts))
    print(f"  /count: attempts {attempts} successes {len(counts)} duplicates {dups} actor total {total} ok={len(counts) <= total <= attempts}; echo wrong {echo_wrong}")
    await asyncio.sleep(1.0)
    print("after:", supstat(), "workers", len(workers()))
    # every worker answers? many concurrent requests that touch an actor
    res = await hc.many(port, [("GET", "/slow/1/20")] * 60, timeout=15)
    print("  60 concurrent /slow afterwards:", hc.tally(res))
    res = await hc.many(port, [("GET", "/count")] * 200, concurrency=50, timeout=10)
    print("  200 /count afterwards:", hc.tally(res))
asyncio.run(main())
