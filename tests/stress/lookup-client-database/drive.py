#!/usr/bin/env python3
# drive.py PATH...: asks the server at :8090 for each PATH, and meanwhile for /ping every 50 ms;
# says what each answered, how long it took, and the longest a /ping took meanwhile.
import sys, time, threading, urllib.request, urllib.parse
PORT = 8090
import os
def ask(path, limit=float(os.environ.get('LIMIT', '120'))):
    t = time.time()
    try:
        with urllib.request.urlopen('http://127.0.0.1:%d%s' % (PORT, path), timeout=limit) as r:
            return r.read().decode().strip(), time.time() - t
    except Exception as e:
        return 'ERR %s' % e, time.time() - t
for path in sys.argv[1:]:
    done = threading.Event(); worst = [0.0, 0]
    def pinger():
        while not done.is_set():
            body, took = ask('/ping', 60)
            if body != 'pong': worst[1] += 1
            worst[0] = max(worst[0], took)
            time.sleep(0.05)
    p = threading.Thread(target=pinger); p.start()
    time.sleep(0.2)
    body, took = ask(path)
    done.set(); p.join()
    print('%s\n   -> %s  [%.2f s; slowest ping %.0f ms; pings failed %d]' % (path, body, took, worst[0] * 1000, worst[1]), flush=True)
