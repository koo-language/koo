# Probe: told messages while the supervising process has no room for a connection (Linux).
import http.client, os, socket, subprocess, sys, threading, time

PORT = 8080
threading.stack_size(256 * 1024)


def ask(path, timeout=30):
    try:
        c = http.client.HTTPConnection("127.0.0.1", PORT, timeout=timeout)
        c.request("GET", path)
        r = c.getresponse()
        body = r.read()
        c.close()
        return r.status, body
    except Exception as e:
        return -1, repr(e).encode()


env = dict(os.environ, PORT=str(PORT), WORKERS="4", KOO_CHECK="1")
server = subprocess.Popen(["/x/check-linux"], env=env, stdout=subprocess.DEVNULL, stderr=open("/tmp/err.txt", "w"))
for _ in range(100):
    if ask("/ping", 1)[0] == 200:
        break
    time.sleep(0.1)
name = None
for line in open("/proc/net/unix"):
    if f"@koo-{server.pid}-" in line:
        name = line.split()[-1][1:]
print("socket", name)
before = int(ask("/told/47")[1])
naps = [threading.Thread(target=lambda a=a: ask(f"/nap/{a}/5000", 60), daemon=True) for a in range(100, 240)]
for t in naps:
    t.start()
time.sleep(1.5)
held = []
for i in range(4300):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.setblocking(False)
    try:
        s.connect("\0" + name)
        held.append(s)
    except OSError:
        s.close()
        break
print("held", len(held))
results = [None] * 60


def tell(i):
    results[i] = ask("/tell/47", 30)


began = time.time()
tellers = [threading.Thread(target=tell, args=(i,), daemon=True) for i in range(60)]
for t in tellers:
    t.start()
for t in tellers:
    t.join()
print("tells answered", sorted(set(r[0] for r in results)), "in %.2fs" % (time.time() - began))
for s in held:
    s.close()
for t in naps:
    t.join()
time.sleep(2)
after = int(ask("/told/47")[1])
print("told arrived", after - before, "of 60")
err = open("/tmp/err.txt").read()
print("stderr lines:")
seen = {}
for line in err.splitlines():
    seen[line] = seen.get(line, 0) + 1
for line, n in seen.items():
    print("  %4d  %s" % (n, line[:160]))
server.terminate()
try:
    print("exit", server.wait(15))
except Exception:
    server.kill()
    print("killed")
