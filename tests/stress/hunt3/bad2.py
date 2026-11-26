# One kind of misbehaving client at a time, beside a light ordinary load. bad2.py KIND SECONDS
import asyncio, sys, time, collections, os, random
sys.path.insert(0, '/x')
import L
KIND = sys.argv[1]; SECONDS = float(sys.argv[2]); N = int(sys.argv[3]) if len(sys.argv) > 3 else 5
ok = collections.Counter(); bad = collections.Counter(); slow = collections.Counter(); did = collections.Counter()
t0 = time.time()
async def ordinary(until):
    while time.time() < until:
        t = time.time(); slot = int((t - t0) // 5)
        s, b = await L.one('GET', '/next/1', b'', 40)
        if s == 200: ok[slot] += 1
        else: bad[slot] += 1; bad[(s, b[:20])] += 1
        slow[slot] = max(slow[slot], round(time.time() - t, 2))
        await asyncio.sleep(0.01)
async def misbehave(until):
    idle = []
    while time.time() < until:
        try:
            r, w = await asyncio.open_connection('127.0.0.1', 8080)
            if KIND == 'idle':
                idle.append(w); did['idle'] += 1
                if len(idle) >= N * 100: await asyncio.sleep(1)
                continue
            if KIND == 'partial-post': w.write(b'POST /echo/61 HTTP/1.1\r\nHost: x\r\nContent-Length: 100000\r\n\r\n' + b'z' * 5000)
            elif KIND == 'nap-abort' or KIND == 'nap-close': w.write(b'GET /nap/61/300 HTTP/1.1\r\nHost: x\r\n\r\n')
            elif KIND == 'big-abort' or KIND == 'big-close': w.write(b'GET /big/61/2000000 HTTP/1.1\r\nHost: x\r\n\r\n')
            elif KIND == 'next-abort' or KIND == 'next-close': w.write(b'GET /next/61 HTTP/1.1\r\nHost: x\r\n\r\n')
            elif KIND == 'sleep-abort': w.write(b'GET /sleep/300 HTTP/1.1\r\nHost: x\r\n\r\n')
            elif KIND == 'partial-head': w.write(b'GET /ne')
            elif KIND == 'pipelined':
                w.write(b'GET /next/62 HTTP/1.1\r\nHost: x\r\n\r\n' * 19 + b'GET /next/62 HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n')
                await w.drain()
                d = await asyncio.wait_for(r.read(-1), 30)
                did['pipelined-%d' % d.count(b'HTTP/1.1 200')] += 1
                w.close(); continue
            await w.drain(); await asyncio.sleep(random.uniform(0, 0.1))
            if KIND.endswith('abort'): w.transport.abort()
            else: w.close()
            did[KIND] += 1
        except Exception as e: did['exc ' + type(e).__name__] += 1
    for w in idle: w.close()
async def main():
    until = time.time() + SECONDS
    await asyncio.gather(*[ordinary(until) for _ in range(20)], *[misbehave(until) for _ in range(N)])
    print('%-13s misbehaved: %s' % (KIND, dict(did)))
    print('   ordinary requests answered per 5 s:', [ok[i] for i in range(int(SECONDS // 5) + 1)])
    print('   slowest per 5 s:', [slow[i] for i in range(int(SECONDS // 5) + 1)], 'failed:', {k: v for k, v in bad.items() if not isinstance(k, int)})
asyncio.run(main())
