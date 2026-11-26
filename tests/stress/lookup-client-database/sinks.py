#!/usr/bin/env python3
# Servers that are slow in different ways. BASE+1: takes the connection and never reads.
# +2: reads a kilobyte a second and then answers. +3: reads the request, never answers.
# +4: answers the head at once and the body a byte each 300 ms. +5: answers a byte each 300 ms from the start.
# +6: reads all (by Content-Length) quickly and answers 200 with how much it read.
import socket, threading, sys, time, re
BASE = int(sys.argv[1]) if len(sys.argv) > 1 else 8080
HOST = sys.argv[2] if len(sys.argv) > 2 else '0.0.0.0'
def never_reads(c): time.sleep(300)
def slow_reader(c):
    c.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 4096)
    got = 0
    while True:
        d = c.recv(1024)
        if not d: break
        got += len(d); time.sleep(1)
def never_answers(c):
    while c.recv(65536): pass
def read_request(c):
    data = b''
    while b'\r\n\r\n' not in data:
        d = c.recv(65536)
        if not d: return None
        data += d
    head, rest = data.split(b'\r\n\r\n', 1)
    m = re.search(rb'content-length: *(\d+)', head, re.I)
    n = int(m.group(1)) if m else 0
    while len(rest) < n:
        d = c.recv(1 << 20)
        if not d: break
        rest += d
    return head, rest
def drip_body(c):
    read_request(c)
    c.sendall(b'HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n')
    for i in range(100):
        c.sendall(b'x'); time.sleep(0.3)
def drip_all(c):
    read_request(c)
    for b in b'HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok':
        c.sendall(bytes([b])); time.sleep(0.3)
def reads_all(c):
    r = read_request(c)
    body = b'read %d' % len(r[1])
    c.sendall(b'HTTP/1.1 200 OK\r\nContent-Length: %d\r\n\r\n' % len(body) + body)
MODES = [never_reads, slow_reader, never_answers, drip_body, drip_all, reads_all]
def serve(port, mode):
    s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    if mode in (never_reads, slow_reader): s.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 4096)
    s.bind((HOST, port)); s.listen(64)
    def one(c):
        try: mode(c)
        except Exception: pass
        finally: c.close()
    while True:
        c, _ = s.accept()
        threading.Thread(target=one, args=(c,), daemon=True).start()
for i, mode in enumerate(MODES):
    threading.Thread(target=serve, args=(BASE + 1 + i, mode), daemon=True).start()
while True: time.sleep(3600)
