#!/usr/bin/env python3
# An HTTP server that answers by path, with exactly the bytes written here, and logs each request line.
import socket, threading, sys, time, urllib.parse, re
PORT = int(sys.argv[1]); OTHER = PORT + 1
LOG = open(sys.argv[2], 'a') if len(sys.argv) > 2 else sys.stdout
def log(*a):
    print(*a, file=LOG, flush=True)
def ok(body, extra=b''):
    return b'HTTP/1.1 200 OK\r\n' + extra + b'Content-Length: %d\r\n\r\n' % len(body) + body
def handle(c, port):
    buf = b''
    while True:
        while b'\r\n\r\n' not in buf:
            d = c.recv(65536)
            if not d: return
            buf += d
        head, buf = buf.split(b'\r\n\r\n', 1)
        lines = head.split(b'\r\n')
        method, target, _ = lines[0].split(b' ', 2)
        hdrs = {}
        for l in lines[1:]:
            k, v = l.split(b':', 1); hdrs[k.strip().lower()] = v.strip()
        n = int(hdrs.get(b'content-length', b'0'))
        while len(buf) < n:
            d = c.recv(65536)
            if not d: return
            buf += d
        body, buf = buf[:n], buf[n:]
        log('%d %s %s body=%r auth=%r cookie=%r xk=%r ctype=%r' % (port, method.decode(), target.decode('latin1'), body[:40], hdrs.get(b'authorization'), hdrs.get(b'cookie'), hdrs.get(b'x-key'), hdrs.get(b'content-type')))
        path, _, query = target.partition(b'?')
        q = urllib.parse.parse_qs(query.decode('latin1'), keep_blank_values=True, encoding='latin1')
        head_only = method == b'HEAD'
        if 'to' in q and path != b'/raw':
            # /r?s=302&to=<location>
            status = q.get('s', ['302'])[0]; to = q.get('to', [''])[0].encode('latin1')
            out = b'HTTP/1.1 %s Moved\r\nLocation: %s\r\nContent-Length: 0\r\n\r\n' % (status.encode(), to)
            c.sendall(out); continue
        if path == b'/raw':
            # /raw?b=<percent-encoded bytes of the whole answer>&close=1
            out = urllib.parse.unquote_to_bytes(q.get('b', [''])[0])
            c.sendall(out)
            if q.get('close'): return
            if q.get('hold'): time.sleep(float(q['hold'][0])); return
            continue
        if path == b'/':
            if target == b'/' or True:
                pass
        # anything else: says what was asked
        body = b'%s %s on %d' % (method, target, port)
        out = b'HTTP/1.1 200 OK\r\nContent-Length: %d\r\n\r\n' % len(body)
        c.sendall(out if head_only else out + body)
        if path == b'/root-redirect': pass
def serve(port):
    s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind(('127.0.0.1', port)); s.listen(64)
    while True:
        c, _ = s.accept()
        def one(c=c):
            try: handle(c, port)
            except Exception as e: log('ERR', repr(e))
            finally: c.close()
        threading.Thread(target=one, daemon=True).start()
threading.Thread(target=serve, args=(PORT,), daemon=True).start()
threading.Thread(target=serve, args=(OTHER,), daemon=True).start()
while True: time.sleep(3600)
