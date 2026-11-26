# A client asks for a long answer and reads it slowly (as a client on a slow line does);
# meanwhile the server is told to stop (TERM), or a handler of the same worker runs out of stack (deep).
# trunc.py HOW N [RATE bytes/s]
import socket, sys, time, os, signal, subprocess
how = sys.argv[1]; n = int(sys.argv[2]); rate = int(sys.argv[3]) if len(sys.argv) > 3 else 1000000
SUP = int(os.environ.get('SUP', '1'))
def get_slowly(results, i):
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    if os.environ.get('RCVBUF'): s.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, int(os.environ['RCVBUF']))
    s.connect(('127.0.0.1', 8080))
    s.sendall(b'GET /long/%d HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n' % n)
    got = b''; t = time.time()
    while True:
        d = s.recv(min(65536, rate // 10))
        if not d: break
        got += d
        time.sleep(0.1)
    head, _, body = got.partition(b'\r\n\r\n')
    results[i] = (head.split(b'\r\n')[0].decode(), len(body), round(time.time() - t, 2))
import threading
results = {}
ts = [threading.Thread(target=get_slowly, args=(results, i)) for i in range(8)]
[t.start() for t in ts]
time.sleep(0.5)
if how == 'TERM': os.kill(SUP, signal.SIGTERM)
elif how == 'deep':
    for _ in range(8): subprocess.run(['curl', '-s', '-m', '5', '-o', '/dev/null', 'http://127.0.0.1:8080/deep'])
[t.join() for t in ts]
short = [r for r in results.values() if r[1] != n]
print('%s: asked for %d bytes eight times; answers cut short: %d' % (how, n, len(short)))
for i in sorted(results): print('   ', results[i])
