# The server is told to stop at different moments. term.py WORKERS
import asyncio, sys, time, os, signal, subprocess
sys.path.insert(0, '/x')
import L
W = sys.argv[1]
def start():
    out = open('/tmp/out.log', 'w'); err = open('/tmp/err.log', 'w')
    p = subprocess.Popen(['/x/s'], env=dict(os.environ, WEB_CONCURRENCY=W), stdout=out, stderr=err)
    for _ in range(100):
        if subprocess.run(['curl', '-s', '-m', '1', '-o', '/dev/null', 'http://127.0.0.1:8080/ping']).returncode == 0: break
        time.sleep(0.05)
    return p
async def scenario(name, reqs, after, sig=signal.SIGTERM, expect=None, twice=False):
    p = start()
    tasks = [asyncio.ensure_future(L.one(m, path, body, 60)) for (m, path, body) in reqs]
    await asyncio.sleep(after)
    t = time.time(); p.send_signal(sig)
    if twice:
        await asyncio.sleep(0.2); p.send_signal(sig)
    res = await asyncio.gather(*tasks)
    loop = asyncio.get_event_loop()
    try: code = await asyncio.wait_for(loop.run_in_executor(None, p.wait), 50)
    except asyncio.TimeoutError: code = 'STILL RUNNING after 50 s'; p.kill()
    took = time.time() - t
    statuses = {}
    for (s, b), rq in zip(res, reqs):
        key = s if s > 0 else '%d %s' % (s, b[:30].decode('latin1'))
        statuses[key] = statuses.get(key, 0) + 1
    out = open('/tmp/out.log').read(); err = open('/tmp/err.log').read()
    left = subprocess.run(['pgrep', '-x', 's'], capture_output=True, text=True).stdout.split()
    print('%-44s exit=%s in %.1f s; answers %s; printed %d lines, serve returned: %s; stderr %d lines; processes left: %s' % (
        name, code, took, statuses, len(out.splitlines()), 'serve returned' in out, len(err.splitlines()), left), flush=True)
    if expect: expect(res, out, err)
    for q in left: os.kill(int(q), 9)
    await asyncio.sleep(0.3)
async def main():
    G = lambda path: ('GET', path, b'')
    await scenario('idle', [], 0.3)
    await scenario('20 handlers wait for 20 slow actors (2 s)', [G('/nap/%d/2000' % i) for i in range(20)], 0.5)
    await scenario('10 handlers wait in line for one actor', [G('/nap/1/400') for i in range(10)], 0.5)
    await scenario('300 handlers wait for slow actors', [G('/nap/%d/1500' % (i % 60)) for i in range(300)], 0.5)
    await scenario('told messages are waiting (10 x 300 ms)', [G('/slowsay/2/300/told%d' % i) for i in range(10)], 0.5,
                   expect=lambda res, out, err: print('     told and printed before the end: %d of 10' % sum(1 for l in out.splitlines() if l.startswith('told'))))
    await scenario('told to a small mailbox that is full', [G('/small/300') for i in range(12)], 0.5)
    await scenario('handlers asleep (time.sleep 2 s)', [G('/sleep/2000') for i in range(50)], 0.5)
    await scenario('a handler computes for some seconds', [G('/spin/2000000000')] * 2 + [G('/next/1')] * 20, 0.5)
    await scenario('TERM twice', [G('/nap/%d/2000' % i) for i in range(20)], 0.5, twice=True)
    await scenario('INT', [G('/nap/%d/2000' % i) for i in range(20)], 0.5, sig=signal.SIGINT)
    await scenario('actors started and stopped by handlers', [G('/life/2000') for i in range(40)], 0.2)
    await scenario('echo of 1 MB through actors', [('POST', '/echo/%d' % i, b'e' * 1000000) for i in range(30)], 0.05)
    await scenario('an actor that takes 40 s (longer than is waited)', [G('/nap/1/40000')], 0.5)
asyncio.run(main())
