# Two workers. One handler asks actor 8 for a long answer; a handler of the same worker then computes
# for seconds. Meanwhile the other worker asks actor 8, and actor 9, something quick.
import asyncio, sys, time
sys.path.insert(0, '/x')
import L
SPIN = int(sys.argv[1]) if len(sys.argv) > 1 else 3000000000
async def timed(path, timeout=60):
    t = time.time(); s, b = await L.one('GET', path, b'', timeout); return (round(time.time() - t, 2), s, len(b))
async def attempt(n):
    big = asyncio.ensure_future(timed('/big/8/6000000'))
    await asyncio.sleep(0.05)
    spin = asyncio.ensure_future(timed('/spin/%d' % SPIN))
    await asyncio.sleep(0.3)
    probes8 = [asyncio.ensure_future(timed('/next/8')) for _ in range(6)]
    probes9 = [asyncio.ensure_future(timed('/next/9')) for _ in range(6)]
    r8 = await asyncio.gather(*probes8); r9 = await asyncio.gather(*probes9)
    b = await big; s = await spin
    same = b[0] > s[0] * 0.8
    print('attempt %d: computing took %.2f s; the long answer took %.2f s (%s worker)' % (n, s[0], b[0], 'the same' if same else 'another'))
    print('    quick asks of the actor that gave the long answer: fastest %.2f s, all %s' % (min(r[0] for r in r8), sorted(r[0] for r in r8)))
    print('    quick asks of another actor:                      fastest %.2f s, all %s' % (min(r[0] for r in r9), sorted(r[0] for r in r9)))
async def main():
    for n in range(1, 7): await attempt(n)
asyncio.run(main())
