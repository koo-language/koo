# Probe: long answers to busy workers, quick asks, tells and killed workers, all at once,
# with KOO_CHECK set. usage: chaos.py <binary> <port> <seconds> <kill 0|1>
import http.client, os, random, signal, subprocess, sys, threading, time

binary, PORT, SECONDS, KILL = sys.argv[1], int(sys.argv[2]), float(sys.argv[3]), sys.argv[4] == "1"
threading.stack_size(256 * 1024)
errors = sys.argv[5] if len(sys.argv) > 5 else "/tmp/chaos-err.txt"


def ask(path, timeout=30):
    began = time.time()
    try:
        c = http.client.HTTPConnection("127.0.0.1", PORT, timeout=timeout)
        c.request("GET", path)
        r = c.getresponse()
        body = r.read()
        c.close()
        return r.status, body, time.time() - began
    except Exception as e:
        return -1, repr(e).encode(), time.time() - began


env = dict(os.environ, PORT=str(PORT), WORKERS="4", KOO_CHECK="1")
server = subprocess.Popen([binary], env=env, stdout=subprocess.DEVNULL, stderr=open(errors, "w"))
for _ in range(100):
    if ask("/ping", 1)[0] == 200:
        break
    time.sleep(0.1)
stop_at = time.time() + SECONDS
lock = threading.Lock()
ok_next = [0] * 16
ok_tell = [0] * 16
bad, slow, hung = [], [], []


def bigs(seed):
    rng = random.Random(seed)
    while time.time() < stop_at:
        n = rng.choice([200000, 1000000, 3000000, 6000000])
        i = 32 + rng.randrange(8)
        status, body, took = ask(f"/big/{i}/{n}", 40)
        if status != 200 or len(body) != n:
            with lock:
                (hung if status == -1 else bad).append(("big", n, status, len(body), round(took, 2), body[:60] if status != 200 else b""))


def spins():
    while time.time() < stop_at:
        ask("/spin/150000000", 40)


def nexts(i):
    while time.time() < stop_at:
        status, body, took = ask(f"/next/{i}", 40)
        with lock:
            if status == 200:
                ok_next[i] += 1
                if took > 0.9:
                    slow.append(("next", i, round(took, 2)))
            else:
                (hung if status == -1 else bad).append(("next", i, status, round(took, 2), body[:60]))


def tells(i):
    while time.time() < stop_at:
        status, body, took = ask(f"/tell/{i}", 40)
        with lock:
            if status == 200:
                ok_tell[i] += 1
                if took > 0.9:
                    slow.append(("tell", i, round(took, 2)))
            else:
                (hung if status == -1 else bad).append(("tell", i, status, round(took, 2), body[:60]))


def killer():
    rng = random.Random(3)
    while time.time() < stop_at - 3:
        time.sleep(1.5)
        kids = subprocess.run(["pgrep", "-P", str(server.pid)], capture_output=True, text=True).stdout.split()
        if kids:
            try:
                os.kill(int(rng.choice(kids)), signal.SIGKILL)
            except OSError:
                pass


threads = [threading.Thread(target=bigs, args=(s,), daemon=True) for s in range(10)]
threads += [threading.Thread(target=spins, daemon=True) for _ in range(6)]
threads += [threading.Thread(target=nexts, args=(i,), daemon=True) for i in range(16)]
threads += [threading.Thread(target=tells, args=(i,), daemon=True) for i in range(16)]
if KILL:
    threads.append(threading.Thread(target=killer, daemon=True))
for t in threads:
    t.start()
for t in threads:
    t.join()
time.sleep(1.5)
print("server", "alive" if server.poll() is None else f"ENDED with {server.returncode}")
problems = []
for i in range(16):
    seen = ask(f"/seen/{i}")[1].strip()
    told = ask(f"/told/{i}")[1].strip()
    if not KILL and (seen != str(ok_next[i]).encode() or told != str(ok_tell[i]).encode()):
        problems.append((i, "next ok", ok_next[i], "seen", seen, "tell ok", ok_tell[i], "told", told))
    if KILL and (int(seen or b"-1") < ok_next[i] or int(told or b"-1") < ok_tell[i]):
        problems.append((i, "next ok", ok_next[i], "seen", seen, "tell ok", ok_tell[i], "told", told))
print("asks", sum(ok_next), "tells", sum(ok_tell))
print("count problems", problems[:8])
print("bad", len(bad), bad[:8])
print("hung", len(hung), hung[:8])
print("slow (>0.9s)", len(slow), sorted(slow, key=lambda s: -s[2])[:12])
server.terminate()
try:
    print("exit", server.wait(15))
except Exception:
    server.kill()
    print("killed: did not end")
seen = {}
for line in open(errors).read().splitlines():
    seen[line] = seen.get(line, 0) + 1
for line, n in sorted(seen.items(), key=lambda kv: -kv[1])[:12]:
    print("  %5d  %s" % (n, line[:160]))
