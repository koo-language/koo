#!/usr/bin/env python3
# A peer that pretends, badly, to be PostgreSQL. One mode per port: base+N.
import socket, threading, sys, struct, os, time, base64, re

BASE = int(sys.argv[1]) if len(sys.argv) > 1 else 39600

def msg(kind, body=b''):
    return kind + struct.pack('>I', len(body) + 4) + body

def read_startup(c):
    # reads the start-up message (and answers N to a request for TLS)
    while True:
        head = c.recv(4)
        if len(head) < 4: return None
        n = struct.unpack('>I', head)[0]
        body = b''
        while len(body) < n - 4:
            more = c.recv(n - 4 - len(body))
            if not more: return None
            body += more
        if body[:4] == b'\x04\xd2\x16\x2f':
            c.sendall(b'N')
            continue
        return body

def read_msg(c):
    head = b''
    while len(head) < 5:
        more = c.recv(5 - len(head))
        if not more: return None
        head += more
    n = struct.unpack('>I', head[1:])[0]
    body = b''
    while len(body) < n - 4:
        more = c.recv(n - 4 - len(body))
        if not more: return None
        body += more
    return head[:1], body

def silent(c):
    time.sleep(120)

def silent_after_startup(c):
    read_startup(c); time.sleep(120)

def flood_notices(c):
    read_startup(c)
    one = msg(b'N', b'SNOTICE\0Mhello\0\0') * 200000
    try:
        while True: c.sendall(one)
    except OSError: pass

def trickle_notices(c):
    read_startup(c)
    one = msg(b'N', b'SNOTICE\0Mhello\0\0')
    try:
        while True:
            c.sendall(one); time.sleep(0.5)
    except OSError: pass

def scram_huge(c):
    read_startup(c)
    c.sendall(msg(b'R', struct.pack('>I', 10) + b'SCRAM-SHA-256\0\0'))
    m = read_msg(c)
    if not m: return
    body = m[1]
    first = body[body.index(b'\0') + 5:]  # after name, NUL, int32
    nonce = re.search(rb'r=([^,]+)', first).group(1)
    reply = b'r=' + nonce + b'serverpart,s=' + base64.b64encode(b'0123456789abcdef') + b',i=4294967295'
    c.sendall(msg(b'R', struct.pack('>I', 11) + reply))
    time.sleep(300)

def huge_length(c):
    read_startup(c)
    c.sendall(b'R' + struct.pack('>I', 0x40000000) + b'\0\0\0\0')
    time.sleep(120)

def garbage(c):
    read_startup(c)
    try:
        while True: c.sendall(os.urandom(65536))
    except OSError: pass

def close_at_once(c):
    pass

def close_after_startup(c):
    read_startup(c)

def ok_then_close(c):
    read_startup(c); c.sendall(msg(b'R', struct.pack('>I', 0)))

def ok_then_silent(c):
    read_startup(c); c.sendall(msg(b'R', struct.pack('>I', 0))); time.sleep(120)

def flood_after_ok(c):
    read_startup(c); c.sendall(msg(b'R', struct.pack('>I', 0)))
    one = msg(b'S', b'a\0b\0') * 400000
    try:
        while True: c.sendall(one)
    except OSError: pass

def http_like(c):
    read_startup(c)
    c.sendall(b'HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n')
    time.sleep(120)

def tls_silent(c):
    # never answers the request for TLS
    time.sleep(120)

def half_message(c):
    read_startup(c); c.sendall(b'R\0\0'); time.sleep(120)

def tiny_length(c):
    read_startup(c); c.sendall(b'R\0\0\0\x03' + b'\0' * 64); time.sleep(120)

def short_auth(c):
    # an authentication request whose body is shorter than it must be
    read_startup(c); c.sendall(msg(b'R', struct.pack('>I', 12))); c.sendall(msg(b'R', struct.pack('>I', 5) + b'ab')); c.sendall(msg(b'R', struct.pack('>I', 11))); time.sleep(5)

def ready_never_reads(c):
    read_startup(c); c.sendall(msg(b'R', struct.pack('>I', 0)) + msg(b'Z', b'I')); time.sleep(300)

def ready_slow_reader(c):
    read_startup(c); c.sendall(msg(b'R', struct.pack('>I', 0)) + msg(b'Z', b'I'))
    while True:
        if not c.recv(1024): break
        time.sleep(0.5)

def ready_then_garbage(c):
    # answers the first statement with rows that lie about themselves
    read_startup(c); c.sendall(msg(b'R', struct.pack('>I', 0)) + msg(b'Z', b'I'))
    while True:
        m = read_msg(c)
        if m is None: return
        if m[0] == b'S':
            row = msg(b'T', struct.pack('>H', 65535) + b'a\0' + b'\0' * 18)
            row += msg(b'D', struct.pack('>H', 3) + struct.pack('>i', 2147483647) + b'xy')
            row += msg(b'D', struct.pack('>H', 65535) + struct.pack('>i', -5))
            row += msg(b'D', b'')
            row += msg(b'T', b'')
            row += msg(b'C', b'SELECT 99999999999999999999999999\0')
            row += msg(b'E', b'')
            row += msg(b'E', b'C\0M\0\0')
            row += msg(b'E', b'Cabcdefgh\0Mlong code\0\0')
            row += msg(b'Z', b'I')
            c.sendall(row)

MODES = [silent, silent_after_startup, flood_notices, scram_huge, huge_length, garbage, close_at_once,
         close_after_startup, ok_then_close, ok_then_silent, flood_after_ok, http_like, tls_silent,
         half_message, tiny_length, short_auth, trickle_notices,
         ready_never_reads, ready_slow_reader, ready_then_garbage]

def serve(port, mode):
    s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind(('0.0.0.0', port)); s.listen(16)
    def one(c):
        try: mode(c)
        except Exception as e: pass
        finally:
            try: c.close()
            except OSError: pass
    while True:
        c, _ = s.accept()
        threading.Thread(target=one, args=(c,), daemon=True).start()

for i, mode in enumerate(MODES):
    print(BASE + i, mode.__name__, flush=True)
    threading.Thread(target=serve, args=(BASE + i, mode), daemon=True).start()
while True: time.sleep(3600)
