# Talks to the supervising process's socket as a worker would.
import socket, struct, sys, time, os
def name():
    for line in open('/proc/net/unix'):
        p = line.split()
        if p and p[-1].startswith('@koo-'): return p[-1][1:]
def conn(n=None):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect('\0' + (n or name()))
    return s
def msg(kind, method, body):
    m = method.encode() if isinstance(method, str) else method
    b = body.encode() if isinstance(body, str) else body
    inner = bytes([kind]) + struct.pack('<H', len(m)) + m + b
    return struct.pack('<I', len(inner)) + inner
def readn(s, n):
    out = b''
    while len(out) < n:
        d = s.recv(n - len(out))
        if not d: raise EOFError('closed after %d of %d' % (len(out), n))
        out += d
    return out
def reply(s):
    a = readn(s, 1)
    size = struct.unpack('<I', readn(s, 4))[0]
    data = readn(s, size)
    k = struct.unpack('<H', data[:2])[0]
    return a, data[2:2 + k], data[2 + k:]
def ask(s, method, body='[]'):
    s.sendall(msg(0, method, body))
    return reply(s)
