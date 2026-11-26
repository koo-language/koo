#!/usr/bin/env python3
# A name server that behaves as the first label of the name it is asked for says.
# dns.py ADDRESS [PERSONALITY]   (personality: byname, refuse, servfail, formerr, lie, nx, silent, slowlie)
import socket, struct, sys, threading, time, os, re

ADDR = sys.argv[1]
WHO = sys.argv[2] if len(sys.argv) > 2 else 'byname'
V6 = ':' in ADDR
FAM = socket.AF_INET6 if V6 else socket.AF_INET
T0 = time.time()

def log(*a):
    print('[dns %s %s %.2f]' % (ADDR, WHO, time.time() - T0), *a, flush=True)

def qname(pkt, at=12):
    labels = []
    while True:
        n = pkt[at]
        if n == 0: return b'.'.join(labels), at + 1
        labels.append(pkt[at + 1:at + 1 + n]); at += 1 + n

def enc(name):
    out = b''
    for l in name.split(b'.'):
        if l: out += bytes([len(l)]) + l
    return out + b'\0'

def head(q, flags, an, qd=1, ns=0, ar=0):
    return q[:2] + struct.pack('>HHHHH', flags, qd, an, ns, ar)

def rr(name_bytes, kind, rdata, ttl=60):
    return name_bytes + struct.pack('>HHIH', kind, 1, ttl, len(rdata)) + rdata

PTR = b'\xc0\x0c'
A1 = bytes([127, 0, 0, 1])
V6_1 = b'\0' * 15 + b'\1'

def answer(q, proto):
    """returns a list of (delay, bytes, how) to send; how: None|'other'"""
    name, end = qname(q)
    kind = struct.unpack('>H', q[end:end + 2])[0]
    quest = q[12:end + 4]
    first = name.split(b'.')[0].lower().decode('latin1')
    log(proto, 'asked', name.decode('latin1'), 'A' if kind == 1 else 'AAAA' if kind == 28 else kind, 'id', q[:2].hex())
    ok = head(q, 0x8180, 1) + quest + (rr(PTR, 1, A1) if kind == 1 else rr(PTR, 28, V6_1))
    nodata = head(q, 0x8180, 0) + quest
    def rcode(n): return head(q, 0x8180 | n, 0) + quest
    if WHO == 'noedns':  # a server from before EDNS: a question with anything after it is malformed
        return [(0, rcode(1), None)] if q[10:12] != b'\0\0' else [(0, ok, None)]
    if WHO == 'refuse': return [(0, rcode(5), None)]
    if WHO == 'servfail': return [(0, rcode(2), None)]
    if WHO == 'formerr': return [(0, rcode(1), None)]
    if WHO == 'nx': return [(0, rcode(3), None)]
    if WHO == 'silent': return []
    if WHO == 'lie': return [(0, head(q, 0x8180, 1) + quest + rr(PTR, 1, bytes([127, 0, 0, 66])), None)] if kind == 1 else [(0, nodata, None)]
    if WHO == 'slowok': return [(1.0, ok, None)]
    if WHO == 'ok': return [(0, ok, None)]
    m = re.match(r'slow(\d+)$', first)
    if m: return [(int(m.group(1)) / 10.0, ok, None)]
    if first == 'ok': return [(0, ok, None)]
    if first == 'ok4':
        return [(0, ok if kind == 1 else nodata, None)]
    if first == 'nx': return [(0, rcode(3), None)]
    if first == 'nodata': return [(0, nodata, None)]
    if first == 'refused': return [(0, rcode(5), None)]
    if first == 'servfail': return [(0, rcode(2), None)]
    if first == 'formerr': return [(0, rcode(1), None)]
    if first == 'notimp': return [(0, rcode(4), None)]
    if first == 'silent': return []
    if first == 'aaaaonly':
        return [(0, nodata if kind == 1 else ok, None)]
    if first == 'slowa':   # the old kind never answers, the new kind does
        return [] if kind == 1 else [(0, ok, None)]
    if first in ('tc', 'tcnotcp', 'tcstall', 'tcwronglen', 'tcbig', 'tcshortlen', 'tcslow', 'tcrefused', 'tcdrip'):
        if proto == 'udp': return [(0, head(q, 0x8380, 0) + quest, None)]
        if first == 'tc': return [(0, ok, None)]
        if first == 'tcbig':
            n = 900 if kind == 1 else 400
            junk = b''.join(rr(PTR, 16, bytes([10]) + b'x' * 10) for _ in range(n))
            return [(0, head(q, 0x8180, n + 1) + quest + junk + (rr(PTR, 1, A1) if kind == 1 else rr(PTR, 28, V6_1)), None)]
        if first == 'tcrefused': return [(0, rcode(5), None)]
        return [(0, ok, first)]
    if first == 'tcwithanswer':  # truncated, but an address is in what did fit
        if proto == 'udp': return [(0, head(q, 0x8380, 1) + quest + rr(PTR, 1, bytes([127, 0, 0, 77])), None)]
        return [(0, ok, None)]
    if first == 'wrongid':
        bad = bytes([q[0] ^ 0xff, q[1]]) + ok[2:]
        return [(0, bad, None)]
    if first == 'wrongidthenok':
        bad = bytes([q[0] ^ 0xff, q[1]]) + head(q, 0x8180, 1)[2:] + quest + rr(PTR, 1, bytes([127, 0, 0, 66]))
        return [(0, bad, None), (0.3, ok, None)]
    if first == 'wrongname':
        other = enc(b'other.example') + struct.pack('>HH', kind, 1)
        return [(0, head(q, 0x8180, 1) + other + rr(PTR, 1, bytes([127, 0, 0, 66])), None)]
    if first == 'wrongrrname':
        return [(0, head(q, 0x8180, 1) + quest + rr(enc(b'other.example'), 1, bytes([127, 0, 0, 66])), None)]
    if first == 'wrongaddr':
        return [(0, head(q, 0x8180, 1) + quest + rr(PTR, 1, bytes([127, 0, 0, 66])), 'other')]
    if first == 'wrongaddrthenok':
        return [(0, head(q, 0x8180, 1) + quest + rr(PTR, 1, bytes([127, 0, 0, 66])), 'other'), (0.3, ok, None)]
    if first == 'query':   # an answer that is itself a question (QR not set) with an address in it
        return [(0, head(q, 0x0100, 1) + quest + rr(PTR, 1, bytes([127, 0, 0, 66])), None)]
    if first == 'echo': return [(0, q, None)]
    if first == 'short': return [(0, q[:2] + b'\x81\x80\0', None)]
    if first == 'header': return [(0, head(q, 0x8180, 5), None)]
    if first == 'counts': return [(0, head(q, 0x8180, 65535, qd=65535) + quest, None)]
    if first == 'countsthenok': return [(0, head(q, 0x8180, 65535, qd=65535) + quest, None), (0.3, ok, None)]
    if first == 'ptrloop':
        # the name of the question is a pointer to itself, as is the name of the answer
        return [(0, head(q, 0x8180, 1) + b'\xc0\x0c' + struct.pack('>HH', kind, 1) + rr(b'\xc0\x10', 1, A1), None)]
    if first == 'ptrfar': return [(0, head(q, 0x8180, 1) + quest + rr(b'\xff\xff', 1, A1), None)]
    if first == 'cname':
        chain = rr(PTR, 5, enc(b'one.example')) + rr(enc(b'one.example'), 5, enc(b'two.example'))
        return [(0, head(q, 0x8180, 3) + quest + chain + (rr(enc(b'two.example'), 1, A1) if kind == 1 else rr(enc(b'two.example'), 28, V6_1)), None)]
    if first == 'cnameonly': return [(0, head(q, 0x8180, 1) + quest + rr(PTR, 5, enc(b'one.example')), None)]
    if first == 'cnameloop': return [(0, head(q, 0x8180, 2) + quest + rr(PTR, 5, enc(b'b.example')) + rr(enc(b'b.example'), 5, name and enc(name)), None)]
    if first == 'garbage': return [(0, q[:2] + os.urandom(700), None)]
    if first == 'garbageflood': return [(i * 0.0005, q[:2] + os.urandom(300), None) for i in range(20000)]
    if first == 'idflood': return [(i * 0.0002, os.urandom(2) + ok[2:], None) for i in range(60000)]
    if first == 'huge':
        n = 85
        return [(0, head(q, 0x8180, n + 1) + quest + b''.join(rr(PTR, 16, b'\x03abc') for _ in range(n)) + rr(PTR, 1, A1), None)]
    if first == 'toolong':  # longer than the 1232 that was asked for, and than a frame
        n = 180
        return [(0, head(q, 0x8180, n + 1) + quest + b''.join(rr(PTR, 16, b'\x03abc') for _ in range(n)) + rr(PTR, 1, A1), None)]
    if first == 'kind':   # an address of the other kind than was asked for
        return [(0, head(q, 0x8180, 1) + quest + (rr(PTR, 28, V6_1) if kind == 1 else rr(PTR, 1, A1)), None)]
    if first == 'rdlen': return [(0, head(q, 0x8180, 2) + quest + rr(PTR, 1, b'\x7f\0\0') + rr(PTR, 1, A1 + b'\0'), None)]
    if first == 'rdlenlie':  # says four bytes and has two
        return [(0, head(q, 0x8180, 1) + quest + PTR + struct.pack('>HHIH', 1, 1, 60, 4) + b'\x7f\0', None)]
    if first == 'authority':  # the address is not in the answers but in what follows them
        return [(0, head(q, 0x8180, 0, ns=1) + quest + rr(PTR, 1, bytes([127, 0, 0, 66])), None)]
    if first == 'class':  # an address, but not of the Internet
        return [(0, head(q, 0x8180, 1) + quest + PTR + struct.pack('>HHIH', 1, 3, 60, 4) + bytes([127, 0, 0, 66]), None)]
    if first == 'zero': return [(0, head(q, 0x8180, 1) + quest + rr(PTR, 1, bytes(4)), None)]
    if first == 'twice': return [(0, rcode(5), None), (0.0, rcode(5), None), (0.5, ok, None)]
    if first == 'refusedthenok': return [(0, rcode(5), None), (0.5, ok, None)]
    return [(0, rcode(3), None)]

def udp():
    s = socket.socket(FAM, socket.SOCK_DGRAM)
    s.bind((ADDR, 53))
    other = None
    if not V6:
        other = socket.socket(FAM, socket.SOCK_DGRAM)
        try: other.bind(('127.0.0.99', 53))
        except OSError: other = s
    while True:
        q, who = s.recvfrom(4096)
        try: plan = answer(q, 'udp')
        except Exception as e:
            log('bad question', e); continue
        def go(plan=plan, who=who):
            at = time.time()
            for delay, data, how in plan:
                wait = at + delay - time.time()
                if wait > 0: time.sleep(wait)
                (other if how == 'other' else s).sendto(data, who)
        threading.Thread(target=go, daemon=True).start()

def tcp():
    s = socket.socket(FAM, socket.SOCK_STREAM)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind((ADDR, 53)); s.listen(8)
    def one(c):
        try:
            n = struct.unpack('>H', c.recv(2))[0]
            q = b''
            while len(q) < n: q += c.recv(n - len(q))
            plan = answer(q, 'tcp')
            for delay, data, how in plan:
                time.sleep(delay)
                if how == 'tcnotcp': break
                if how == 'tcstall': time.sleep(60); break
                if how == 'tcslow': time.sleep(1.2)
                if how == 'tcwronglen':
                    c.sendall(struct.pack('>H', 5000) + data); time.sleep(60); break
                if how == 'tcshortlen':
                    c.sendall(struct.pack('>H', 20) + data); break
                if how == 'tcdrip':
                    whole = struct.pack('>H', len(data)) + data
                    for b in whole:
                        c.sendall(bytes([b])); time.sleep(0.2)
                    break
                c.sendall(struct.pack('>H', len(data)) + data)
        except Exception as e:
            log('tcp', e)
        finally:
            c.close()
    while True:
        c, _ = s.accept()
        threading.Thread(target=one, args=(c,), daemon=True).start()

threading.Thread(target=udp, daemon=True).start()
if WHO != 'notcp':
    threading.Thread(target=tcp, daemon=True).start()
while True: time.sleep(3600)
