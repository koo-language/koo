# Many lookups at once from the handlers of a server, and what the server does for others meanwhile.
import asyncio, sys, time, os, re, subprocess, collections
sys.path.insert(0, '/x')
import L
def fds():
    out = []
    for p in subprocess.run(['pgrep', '-x', 's'], capture_output=True, text=True).stdout.split():
        try: out.append(len(os.listdir('/proc/%s/fd' % p)))
        except Exception: pass
    return out
async def pings(stop):
    worst = 0; n = 0
    while not stop.is_set():
        t = time.time(); s, b = await L.one('GET', '/ping', b'', 30); worst = max(worst, time.time() - t); n += 1
        await asyncio.sleep(0.02)
    return worst, n
async def batch(label, host, limit, count):
    stop = asyncio.Event(); p = asyncio.ensure_future(pings(stop))
    t = time.time()
    res = await asyncio.gather(*[L.one('GET', '/look/%s/%d' % (host, limit), b'', 90) for _ in range(count)])
    took = time.time() - t; stop.set(); worst, n = await p
    kinds = collections.Counter(); over = 0; longest = 0
    for s, b in res:
        m = re.search(rb'-- (\d+) ms', b)
        ms = int(m.group(1)) if m else -1
        longest = max(longest, ms)
        if ms > limit + 150: over += 1
        kinds[(s, b.split(b' --')[0][:40].decode('latin1'))] += 1
    print('%-40s %d at once, limit %d ms: all done in %.1f s; longest lookup %d ms; over the limit: %d; slowest ping meanwhile %.2f s (%d pings); worker fds %s' % (label, count, limit, took, longest, over, worst, n, fds()))
    for k, v in kinds.most_common(4): print('      %d x %s' % (v, k))
async def main():
    print('fds at the start', fds())
    await batch('answer after 2 s', 'slow20.test', 5000, 100)
    await batch('silent name server', 'silent.test', 1500, 100)
    await batch('no such name', 'nx.test', 1500, 200)
    await batch('answer cut short, connection stalls', 'tczero.test', 2000, 60)
    await batch('answer after 1 s, more than fit a worker', 'slow10.test', 5000, 400)
    await batch('silent, more than fit a worker', 'silent.test', 1000, 400)
    await asyncio.sleep(1)
    print('fds at the end', fds())
asyncio.run(main())
