#!/usr/bin/env python3
# The first request on a connection is answered and the connection kept; what the second meets is in its path.
import socket, threading, sys, time, struct, re
PORT = int(sys.argv[1]); LOG = open(sys.argv[2], 'a')
count = [0]
def log(*a): print(*a, file=LOG, flush=True)
def handle(c, cid):
    buf = b''; n = 0
    while True:
        while b'\r\n\r\n' not in buf:
            d = c.recv(65536)
            if not d: log('conn %d closed by client after %d requests' % (cid, n)); return
            buf += d
        head, buf = buf.split(b'\r\n\r\n', 1)
        m = re.search(rb'content-length: *(\d+)', head, re.I); need = int(m.group(1)) if m else 0
        while len(buf) < need:
            d = c.recv(65536)
            if not d: return
            buf += d
        body, buf = buf[:need], buf[need:]
        n += 1
        line = head.split(b'\r\n')[0].decode()
        log('conn %d request %d: %s body=%d' % (cid, n, line, len(body)))
        path = line.split(' ')[1]
        if n >= 2:
            if path == '/drop': return
            if path == '/rst':
                c.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack('ii', 1, 0)); return
            if path == '/partial':
                c.sendall(b'HTTP/1.1 200 OK\r\nContent-Le'); return
            if path == '/slow':
                time.sleep(3)
        out = b'answer %d on conn %d' % (n, cid)
        c.sendall(b'HTTP/1.1 200 OK\r\nContent-Length: %d\r\n\r\n' % len(out) + out)
s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(('127.0.0.1', PORT)); s.listen(64)
while True:
    c, _ = s.accept(); count[0] += 1
    def one(c=c, cid=count[0]):
        try: handle(c, cid)
        except Exception as e: log('ERR', repr(e))
        finally: c.close()
    threading.Thread(target=one, daemon=True).start()
