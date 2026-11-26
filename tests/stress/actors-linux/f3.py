# Fills every worker with waiting handlers in turn. A worker that is full is stopped
# (SIGSTOP) so that the next round goes to the others, as it would on a machine whose
# processors are all busy; they are all continued before the server is tried.
import L, os, signal, sys, time, subprocess
sup = int(sys.argv[1])
def workers():
    return [int(p) for p in subprocess.run(["pgrep", "-P", str(sup)], capture_output=True, text=True).stdout.split()]
def fds(pid):
    try: return len(os.listdir("/proc/%d/fd" % pid))
    except Exception: return -1
ws = workers(); full = set(); rounds = 0
while len(full) < len(ws) - 1 and rounds < 80:
    rounds += 1
    r = L.run([("GET", "/nap/%d/50" % (1 + i % 250)) for i in range(700)], conc=700, timeout=15)
    for w in ws:
        if w not in full and fds(w) >= 130:
            os.kill(w, signal.SIGSTOP); full.add(w)
    if rounds % 5 == 0 or any(s != 200 for s, b in r):
        print("round %d: %s; workers that have had 127 handlers waiting: %d; connections of the process that watches: %d" % (rounds, L.summary(r), len(full), fds(sup)), flush=True)
    if any(s != 200 for s, b in r):
        break
rest = [w for w in ws if w not in full]
print("%d workers are left that have not been full" % len(rest), flush=True)
for k in range(2):
    t = time.time()
    r = L.run([("GET", "/nap/%d/50" % (1 + i % 250)) for i in range(700)], conc=700, timeout=20)
    print("700 asks that take 50 ms, of the workers that are left:", L.summary(r), "(-5 is no answer in 20 s) in %.1fs" % (time.time() - t), flush=True)
for w in full:
    os.kill(w, signal.SIGCONT)
time.sleep(30)
print("every worker continued, 30 s later: files open in each worker:", sorted(fds(w) for w in ws)[-6:], "(132 is a worker at rest that has been full)")
