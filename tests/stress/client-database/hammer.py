import sys, threading, urllib.request, random, time
PORT = int(sys.argv[1]); N = int(sys.argv[2]); T = int(sys.argv[3]); kinds = sys.argv[4].split(",")
bad = []; counts = {}; BASE = (int(time.time()) % 20000) * 100000
lock = threading.Lock()
def get(path):
    try:
        with urllib.request.urlopen("http://127.0.0.1:%d%s" % (PORT, path), timeout=60) as r:
            return r.status, r.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()
    except Exception as e:
        return -1, repr(e)
def work(tid):
    rnd = random.Random(tid)
    for i in range(N):
        id = BASE + tid * 100000 + i + 1
        k = rnd.choice(kinds)
        if k == "fetch":
            st, body = get("/fetch/%d" % id); want = "id %d|%d|%d" % (id, id, 3 + len(str(id)) + id % 5000)
        elif k == "db":
            st, body = get("/db/%d" % id); want = "%d|%d" % (id, id % 5000)
        elif k == "tx":
            st, body = get("/tx/%d" % id); want = ("undone %d" % id) if id % 2 else ("kept n%d" % id)
        elif k == "each":
            n = id % 777; st, body = get("/each/%d" % n); want = "%d|%d" % (n, n * (n + 1) // 2)
        elif k == "both":
            st, body = get("/both/%d" % id); want = "%d|%d" % (id, id)
        elif k == "panic":
            st, body = get("/panic/%d" % (id + 50000)); want = None
            with lock: counts["panic"] = counts.get("panic", 0) + 1
            if st != 500:
                with lock: bad.append((k, id, st, body[:200]))
            continue
        with lock: counts[k] = counts.get(k, 0) + 1
        if st != 200 or body != want:
            with lock: bad.append((k, id, st, body[:200], want[:80]))
ts = [threading.Thread(target=work, args=(t + 1,)) for t in range(T)]
t0 = time.time()
[t.start() for t in ts]; [t.join() for t in ts]
print("done in %.1fs" % (time.time() - t0), counts, "bad:", len(bad))
for b in bad[:15]: print("  ", b)
