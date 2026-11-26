# A long mixed run against s.koo. soak.py SECONDS CONCURRENCY [kill] [bad] [deep]
import asyncio, sys, os, random, time, collections, signal, subprocess
sys.path.insert(0, '/x')
import L
SECONDS = float(sys.argv[1]); CONC = int(sys.argv[2]); FLAGS = set(sys.argv[3:])
SUP = int(os.environ.get('SUP', '1'))
KILL = 'kill' in FLAGS; BAD = 'bad' in FLAGS; DEEP = 'deep' in FLAGS; BIGMSG = 'nobig' not in FLAGS
rnd = random.Random(int(os.environ.get('SEED', '1')))
ok = collections.Counter(); bad = collections.Counter(); notes = []
answered_next = collections.Counter(); unsure_next = collections.Counter()
told_ok = collections.Counter(); told_unsure = collections.Counter()
small_ok = 0; small_unsure = 0; kept_ok = 0; kept_unsure = 0
slowest = collections.defaultdict(float)
def note(kind, what):
    bad[kind] += 1
    if len(notes) < 40: notes.append((kind, what))
def workers():
    out = subprocess.run(['pgrep', '-P', str(SUP), '-x', 's'], capture_output=True, text=True).stdout.split()
    return [int(p) for p in out]
def supstat():
    try:
        st = open('/proc/%d/stat' % SUP).read().split()
        return dict(fds=len(os.listdir('/proc/%d/fd' % SUP)), threads=len(os.listdir('/proc/%d/task' % SUP)),
                    rss=int([l.split()[1] for l in open('/proc/%d/status' % SUP) if l.startswith('VmRSS')][0]), ticks=int(st[13]) + int(st[14]))
    except Exception as e:
        return dict(gone=repr(e))
async def req(kind, method, path, body=b'', timeout=90.0):
    t = time.time()
    r = await L.one(method, path, body, timeout)
    slowest[kind] = max(slowest[kind], time.time() - t)
    return r
async def worker(n, until):
    global small_ok, small_unsure, kept_ok, kept_unsure
    r = random.Random(n * 7919 + 13)
    while time.time() < until:
        p = r.random()
        if p < 0.30:
            i = r.randrange(0, 16); s, b = await req('next', 'GET', '/next/%d' % i)
            if s == 200: answered_next[i] += 1; ok['next'] += 1
            else:
                unsure_next[i] += 1
                if not KILL: note('next', (s, b[:80]))
        elif p < 0.45:
            i = r.randrange(16, 32)
            size = r.choice([0, 1, 10, 300, 5000, 70000, 300000, 1000000, 3000000]) if BIGMSG else r.choice([0, 1, 10, 300])
            body = bytes(r.choice(b'abcdefghijklmnopqrstuvwxyz') for _ in range(min(size, 64))) * (size // 64 + 1)
            body = body[:size]
            s, b = await req('echo', 'POST', '/echo/%d' % i, body)
            if s == 200 and b == body: ok['echo'] += 1
            elif s == 200: note('echo-WRONG', (len(body), len(b), b[:40]))
            elif not KILL: note('echo', (s, len(body), b[:80]))
        elif p < 0.52:
            i = r.randrange(32, 40); size = r.choice([0, 1, 100, 100000, 1000000, 2500000]) if BIGMSG else r.choice([0, 1, 100])
            s, b = await req('big', 'GET', '/big/%d/%d' % (i, size))
            if s == 200 and b == b'x' * size: ok['big'] += 1
            elif s == 200: note('big-WRONG', (size, len(b)))
            elif not KILL: note('big', (s, size, b[:80]))
        elif p < 0.58:
            i = r.randrange(40, 48); s, b = await req('nap', 'GET', '/nap/%d/%d' % (i, r.choice([1, 5, 20, 100])))
            if s == 200: ok['nap'] += 1
            elif not KILL: note('nap', (s, b[:80]))
        elif p < 0.70:
            i = r.randrange(48, 56); s, b = await req('tell', 'GET', '/tell/%d' % i)
            if s == 200 and b == b'told\n': told_ok[i] += 1; ok['tell'] += 1
            else:
                told_unsure[i] += 1
                if not KILL: note('tell', (s, b[:80]))
        elif p < 0.74:
            i = r.randrange(48, 56); s, b = await req('trytell', 'GET', '/trytell/%d' % i)
            if s == 200 and b == b'true\n': told_ok[i] += 1; ok['trytell'] += 1
            elif s == 200 and b == b'false\n': ok['trytell-full'] += 1
            else:
                told_unsure[i] += 1
                if not KILL: note('trytell', (s, b[:80]))
        elif p < 0.78:
            s, b = await req('small', 'GET', '/small/%d' % r.choice([0, 1, 3]))
            if s == 200 and b == b'told\n': small_ok += 1; ok['small'] += 1
            else:
                small_unsure += 1
                if not KILL: note('small', (s, b[:80]))
        elif p < 0.82:
            s, b = await req('trysmall', 'GET', '/trysmall/%d' % r.choice([0, 1, 3]))
            if s == 200 and b == b'true\n': small_ok += 1; ok['trysmall'] += 1
            elif s == 200 and b == b'false\n': ok['trysmall-full'] += 1
            else:
                small_unsure += 1
                if not KILL: note('trysmall', (s, b[:80]))
        elif p < 0.87:
            k = r.choice([0, 1, 5, 50]); s, b = await req('life', 'GET', '/life/%d' % k)
            if s == 200 and b == b'%d\n' % k: ok['life'] += 1
            elif s == 200: note('life-WRONG', (k, b[:40]))
            elif not KILL: note('life', (s, b[:80]))
        elif p < 0.89:
            s, b = await req('keep', 'GET', '/keep')
            if s == 200: kept_ok += 1; ok['keep'] += 1
            else:
                kept_unsure += 1
                if not KILL: note('keep', (s, b[:80]))
        elif p < 0.92:
            s, b = await req('spin', 'GET', '/spin/%d' % r.choice([1000, 3000000, 60000000]))
            if s == 200: ok['spin'] += 1
            elif not KILL: note('spin', (s, b[:80]))
        elif p < 0.95:
            i = r.randrange(56, 60); what = r.choice(['boom', 'toldboom'] + (['deep', 'tolddeep'] if DEEP else []))
            s, b = await req(what, 'GET', '/%s/%d' % (what, i))
            if s in (200, 500): ok[what + '-%d' % s] += 1
            elif not KILL: note(what, (s, b[:80]))
        elif p < 0.97:
            what = r.choice(['hboom'] + (['hdeep'] if DEEP else []))
            s, b = await req(what, 'GET', '/' + what)
            if s == 500: ok[what] += 1
            elif not (KILL or DEEP): note(what, (s, b[:80]))
        else:
            s, b = await req('ping', 'GET', '/ping')
            if s == 200 and b == b'pong\n': ok['ping'] += 1
            elif not (KILL or DEEP): note('ping', (s, b[:80]))
async def killer(until):
    while time.time() < until:
        await asyncio.sleep(rnd.uniform(1.0, 4.0))
        ws = workers()
        if ws:
            try: os.kill(rnd.choice(ws), signal.SIGKILL); ok['killed'] += 1
            except ProcessLookupError: pass
async def misbehave(until):
    # clients that do not behave, all the while
    async def slow_sender():
        while time.time() < until:
            try:
                r, w = await asyncio.open_connection('127.0.0.1', 8080)
                for ch in b'GET /next/60 HTTP/1.1\r\nHost: x\r\n':
                    w.write(bytes([ch])); await w.drain(); await asyncio.sleep(0.05)
                if rnd.random() < 0.5:
                    w.write(b'\r\n'); await w.drain()
                    d = await asyncio.wait_for(r.read(200), 10)
                    if d.startswith(b'HTTP/1.1 200'): answered_next[60] += 1; ok['slow-sender'] += 1
                    else: unsure_next[60] += 1
                w.close()
            except Exception as e: bad['slow-sender-exc'] += 1
    async def half():
        while time.time() < until:
            try:
                r, w = await asyncio.open_connection('127.0.0.1', 8080)
                which = rnd.randrange(4)
                if which == 0: w.write(b'POST /echo/61 HTTP/1.1\r\nHost: x\r\nContent-Length: 100000\r\n\r\n' + b'z' * 5000)
                elif which == 1: w.write(b'GET /nap/61/3 HTTP/1.1\r\nHost: x\r\n\r\n')
                elif which == 2: w.write(b'GET /big/61/20000 HTTP/1.1\r\nHost: x\r\n\r\n')
                else: w.write(b'GET /ne')
                await w.drain(); await asyncio.sleep(rnd.uniform(0, 0.2))
                if rnd.random() < 0.5: w.transport.abort()
                else: w.close()
                ok['half'] += 1
            except Exception as e: bad['half-exc'] += 1
    async def pipelined():
        while time.time() < until:
            try:
                r, w = await asyncio.open_connection('127.0.0.1', 8080)
                n = 20
                w.write(b''.join(b'GET /next/62 HTTP/1.1\r\nHost: x\r\n\r\n' for _ in range(n - 1)) + b'GET /next/62 HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n')
                await w.drain()
                d = await asyncio.wait_for(r.read(-1), 30)
                got = d.count(b'HTTP/1.1 200')
                answered_next[62] += got
                if got == n: ok['pipelined'] += 1
                else:
                    unsure_next[62] += n - got
                    if not (KILL or DEEP): note('pipelined', (got, d[-80:]))
                w.close()
            except Exception as e:
                unsure_next[62] += 20
                if not (KILL or DEEP): note('pipelined-exc', repr(e))
    idle = []
    for _ in range(int(os.environ.get('IDLE', '500'))):
        try:
            r, w = await asyncio.open_connection('127.0.0.1', 8080); idle.append(w)
        except Exception: break
    ok['idle-open'] = len(idle)
    await asyncio.gather(*[slow_sender() for _ in range(5)], *[half() for _ in range(5)], *[pipelined() for _ in range(3)])
    for w in idle: w.close()
async def main():
    before = supstat(); print('before', before, 'workers', workers(), flush=True)
    until = time.time() + SECONDS
    tasks = [worker(n, until) for n in range(CONC)]
    if KILL: tasks.append(killer(until))
    if BAD: tasks.append(misbehave(until))
    async def watch():
        while time.time() < until:
            await asyncio.sleep(30); print('  ...', int(until - time.time()), 's left', supstat(), 'workers', len(workers()), dict(ok=sum(ok.values()), bad=sum(bad.values())), flush=True)
    await asyncio.gather(*tasks, watch())
    print('load done; requests ok by kind:', dict(sorted(ok.items())), flush=True)
    print('BAD:', dict(bad)); [print('   ', n) for n in notes]
    print('slowest by kind (s):', {k: round(v, 2) for k, v in sorted(slowest.items())})
    await asyncio.sleep(3)
    # exactness
    wrong = 0
    for i in list(range(0, 16)) + [60, 62]:
        s, b = await L.one('GET', '/total/%d' % i)
        total = int(b) if s == 200 else None
        lo, hi = answered_next[i], answered_next[i] + unsure_next[i]
        if total is None or not (lo <= total <= hi): wrong += 1; print('  COUNTER %d: total=%s answered=%d unsure=%d' % (i, total, lo, unsure_next[i]))
    for i in range(48, 56):
        s, b = await L.one('GET', '/tolds/%d' % i)
        total = int(b) if s == 200 else None
        lo, hi = told_ok[i], told_ok[i] + told_unsure[i]
        if total is None or not (lo <= total <= hi): wrong += 1; print('  TOLD %d: tolds=%s told=%d unsure=%d' % (i, total, lo, told_unsure[i]))
    for _ in range(100):
        s, b = await L.one('GET', '/smalltolds')
        if s == 200 and int(b) >= small_ok: break
        await asyncio.sleep(0.2)
    total = int(b) if s == 200 else None
    if total is None or not (small_ok <= total <= small_ok + small_unsure): wrong += 1; print('  SMALL: tolds=%s told=%d unsure=%d' % (total, small_ok, small_unsure))
    s, b = await L.one('GET', '/kept')
    total = int(b) if s == 200 else None
    if total is None or not (kept_ok <= total <= kept_ok + kept_unsure): wrong += 1; print('  KEPT: kept=%s ok=%d unsure=%d' % (total, kept_ok, kept_unsure))
    print('exactness: %d wrong' % wrong)
    a = supstat(); print('just after', a, 'workers', workers(), flush=True)
    await asyncio.sleep(15)
    b2 = supstat(); print('15 s idle later', b2, 'workers', workers())
    if 'ticks' in a and 'ticks' in b2: print('supervisor cpu ticks while idle for 15 s:', b2['ticks'] - a['ticks'])
    for w in workers():
        try:
            st = open('/proc/%d/stat' % w).read().split(); t1 = int(st[13]) + int(st[14])
            await asyncio.sleep(1)
            st = open('/proc/%d/stat' % w).read().split(); t2 = int(st[13]) + int(st[14])
            print('  worker', w, 'fds', len(os.listdir('/proc/%d/fd' % w)), 'cpu ticks in 1 s idle:', t2 - t1)
        except Exception as e: print('  worker', w, repr(e))
    s, b = await L.one('GET', '/ping', timeout=5); print('ping after:', s, b)
asyncio.run(main())
