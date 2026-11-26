# More handlers wait for slow actors than the supervising process may have connections.
import asyncio, sys, time, collections, os
sys.path.insert(0, '/x')
import L
N = int(sys.argv[1]); MS = int(sys.argv[2])
async def main():
    t = time.time()
    res = await asyncio.gather(*[L.one('GET', '/nap/%d/%d' % (i % 64, MS), b'', 300) for i in range(N)])
    c = collections.Counter((s if s > 0 else '%d %s' % (s, b[:30])) for s, b in res)
    print('%d handlers asked slow actors (%d ms each): %s in %.1f s' % (N, MS, dict(c), time.time() - t))
    print('sup: threads=%d fds=%d' % (len(os.listdir('/proc/1/task')), len(os.listdir('/proc/1/fd'))))
    s, b = await L.one('GET', '/next/1', b'', 10); print('next after:', s, b)
asyncio.run(main())
