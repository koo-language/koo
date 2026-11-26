# N clients, one after another, each ask for something that takes 20 s (a handler asleep, nobody
# leaves). Then quick requests are timed. full.py N [PATH]
import socket, time, sys, os, subprocess
N = int(sys.argv[1]); PATH = sys.argv[2] if len(sys.argv) > 2 else '/sleep/20000'
def workers():
    out = []
    for w in subprocess.run(['pgrep', '-P', '1'], capture_output=True, text=True).stdout.split():
        try:
            if open('/proc/%s/comm' % w).read().strip() in ('s', 'full'): out.append('%s:%d' % (w, len(os.listdir('/proc/%s/fd' % w))))
        except Exception: pass
    return ' '.join(out)
def ping():
    t = time.time()
    try:
        s = socket.create_connection(('127.0.0.1', 8080), timeout=8); s.settimeout(8)
        s.sendall(b'GET /ping HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n')
        d = s.recv(100); s.close()
        return '%s in %.2f s' % (d[9:12].decode() or 'closed', time.time() - t)
    except Exception as e: return 'NO ANSWER in %.2f s (%s)' % (time.time() - t, type(e).__name__)
print('files open in each worker (pid:count) at the start:', workers())
ss = []
for i in range(N):
    s = socket.create_connection(('127.0.0.1', 8080)); s.sendall(b'GET %s HTTP/1.1\r\nHost: x\r\n\r\n' % PATH.encode()); ss.append(s)
    time.sleep(0.01)
time.sleep(0.5)
print('%d requests that take 20 s are waiting; files open in each worker: %s' % (N, workers()))
res = [ping() for _ in range(10)]
print('ten quick requests, one after another:', res)
print('files open in each worker afterwards:', workers())
