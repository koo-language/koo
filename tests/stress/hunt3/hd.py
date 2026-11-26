#!/usr/bin/env python3
# A name server that misbehaves as the first label of the name says. hd.py ADDRESS [PERSONALITY]
import socket, struct, sys, threading, time, os, re
ADDR = sys.argv[1]; WHO = sys.argv[2] if len(sys.argv) > 2 else 'byname'
V6 = ':' in ADDR; FAM = socket.AF_INET6 if V6 else socket.AF_INET
T0 = time.time()
def log(*a):
    if os.environ.get('QUIET'): return
    print('[dns %s %s %.2f]' % (ADDR, WHO, time.time() - T0), *a, flush=True)
def qname(pkt, at=12):
    labels = []
    while True:
        n = pkt[at]
        if n == 0: return b'.'.join(labels), at + 1
        labels.append(pkt[at + 1:at + 1 + n]); at += 1 + n
def enc(name):
    return b''.join(bytes([len(l)]) + l for l in name.split(b'.') if l) + b'\0'
def head(q, flags, an, qd=1, ns=0, ar=0):
    return q[:2] + struct.pack('>HHHHH', flags, qd, an, ns, ar)
def rr(name_bytes, kind, rdata, ttl=60):
    return name_bytes + struct.pack('>HHIH', kind, 1, ttl, len(rdata)) + rdata
PTR = b'\xc0\x0c'; A1 = bytes([127, 0, 0, 1]); V6_1 = b'\0' * 15 + b'\1'
seen = {}
def flood(sock, who, bad, seconds, procs):
    # As fast as the system sends: many datagrams with each call (sendmmsg).
    import ctypes
    class iovec(ctypes.Structure): _fields_ = [('base', ctypes.c_void_p), ('len', ctypes.c_size_t)]
    class msghdr(ctypes.Structure): _fields_ = [('name', ctypes.c_void_p), ('namelen', ctypes.c_uint32), ('iov', ctypes.POINTER(iovec)), ('iovlen', ctypes.c_size_t), ('control', ctypes.c_void_p), ('controllen', ctypes.c_size_t), ('flags', ctypes.c_int)]
    class mmsghdr(ctypes.Structure): _fields_ = [('hdr', msghdr), ('len', ctypes.c_uint)]
    for _ in range(procs):
        if os.fork() == 0:
            libc = ctypes.CDLL(None, use_errno=True)
            n = 512
            data = ctypes.create_string_buffer(bad, len(bad))
            if V6:
                addr = struct.pack('=HH', socket.AF_INET6, socket.htons(who[1])) + struct.pack('>I', 0) + socket.inet_pton(socket.AF_INET6, who[0]) + struct.pack('=I', 0)
            else:
                addr = struct.pack('=HH', socket.AF_INET, socket.htons(who[1])) + socket.inet_aton(who[0]) + bytes(8)
            name = ctypes.create_string_buffer(addr, len(addr))
            iov = iovec(ctypes.cast(data, ctypes.c_void_p), len(bad))
            msgs = (mmsghdr * n)()
            for m in msgs:
                m.hdr.name = ctypes.cast(name, ctypes.c_void_p); m.hdr.namelen = len(addr)
                m.hdr.iov = ctypes.pointer(iov); m.hdr.iovlen = 1
            end = time.time() + seconds
            sent = 0
            while time.time() < end:
                for _ in range(50):
                    r = libc.sendmmsg(sock.fileno(), msgs, n, 0)
                    if r > 0: sent += r
            print('[flood] sent', sent, 'in', seconds, 's', flush=True)
            os._exit(0)
def answer(q, proto, sock=None, who=None):
    name, end = qname(q)
    kind = struct.unpack('>H', q[end:end + 2])[0]
    quest = q[12:end + 4]
    first = name.split(b'.')[0].lower().decode('latin1')
    edns = q[10:12] != b'\0\0'
    log(proto, 'asked', name.decode('latin1'), 'A' if kind == 1 else 'AAAA' if kind == 28 else kind, 'id', q[:2].hex(), 'edns' if edns else 'plain')
    good = rr(PTR, 1, A1) if kind == 1 else rr(PTR, 28, V6_1)
    ok = head(q, 0x8180, 1) + quest + good
    nodata = head(q, 0x8180, 0) + quest
    def rcode(n): return head(q, 0x8180 | n, 0) + quest
    if WHO == 'silent': return []
    if WHO == 'ok': return [(0, ok)]
    if WHO == 'nx': return [(0, rcode(3))]
    if WHO == 'servfail': return [(0, rcode(2))]
    if WHO == 'noedns': return [(0, rcode(1))] if edns else [(0, ok)]
    if WHO == 'ednsdrop': return [] if edns else [(0, ok)]          # an old server that drops what it does not understand
    m = re.match(r'slow(\d+)$', first)
    if m: return [(int(m.group(1)) / 10.0, ok)]
    if first == 'ok': return [(0, ok)]
    if first == 'nx': return [(0, rcode(3))]
    if first == 'silent': return []
    if first == 'aaaaonly': return [(0, nodata if kind == 1 else ok)]
    m = re.match(r'flood(\d+)$', first)   # floodN: answers with wrong ids for N seconds
    if m:
        bad = bytes([q[0] ^ 0xff, q[1]]) + ok[2:]
        flood(sock, who, bad, int(m.group(1)), int(os.environ.get("PROCS", "4")))
        return []
    m = re.match(r'sfflood(\d+)$', first)   # says it failed, over and over, for N seconds
    if m:
        flood(sock, who, rcode(2), int(m.group(1)), 4)
        return []
    if first == 'caseflip':   # the question comes back in other letters
        flipped = bytes(c ^ 0x20 if (65 <= c <= 90 or 97 <= c <= 122) else c for c in quest[:-4]) + quest[-4:]
        return [(0, head(q, 0x8180, 1) + flipped + good)]
    if first == 'dup': return [(0, ok), (0, ok), (0.01, ok)]
    if first == 'exact': return [(0, head(q, 0x8180, 1) + quest)]   # says one answer, has none
    if first == 'qonly': return [(0, head(q, 0x8180, 0) + quest[:-1])]   # a byte short of the question
    if first == 'optback': return [(0, head(q, 0x8180, 1, ar=1) + quest + good + b'\0' + struct.pack('>HHIH', 41, 1232, 0, 0))]
    if first == 'nxthenok': return [(0, rcode(3)), (0.05, ok)]
    if first == 'late':     # answers the first question it is asked 2.3 s late, and never the second
        k = (name, kind)
        seen[k] = seen.get(k, 0) + 1
        return [(2.3, ok)] if seen[k] == 1 else []
    if first == 'lateformerr':   # says, 2.2 s late, that it did not understand the first question; answers the second after 0.5 s
        k = (name, kind)
        seen[k] = seen.get(k, 0) + 1
        return [(2.2, rcode(1))] if seen[k] == 1 else [(0.5, ok)]
    if first == 'formerrtwice':  # does not understand the long form, twice; answers the plain one
        return [(0, rcode(1)), (0.01, rcode(1))] if edns else [(0.1, ok)]
    if first in ('tcwrongq', 'tctc', 'tcqr', 'tcnx', 'tcother', 'tchuge', 'tczero', 'tcdrip1', 'tcclose'):
        if proto == 'udp': return [(0, head(q, 0x8380, 0) + quest)]
        if first == 'tcwrongq':
            other = enc(b'other.example') + struct.pack('>HH', kind, 1)
            return [(0, head(q, 0x8180, 1) + other + rr(PTR, 1, bytes([127, 0, 0, 66])))]
        if first == 'tctc': return [(0, head(q, 0x8380, 0) + quest)]
        if first == 'tcqr': return [(0, head(q, 0x0100, 1) + quest + rr(PTR, 1, bytes([127, 0, 0, 66])))]
        if first == 'tcnx': return [(0, rcode(3))]
        if first == 'tchuge': return [(0, ok, 'huge')]
        if first == 'tczero': return [(0, ok, 'zero')]
        if first == 'tcdrip1': return [(0, ok, 'drip')]
        if first == 'tcclose': return [(0, ok, 'close')]
    return [(0, rcode(3))]
def udp():
    s = socket.socket(FAM, socket.SOCK_DGRAM)
    s.bind((ADDR, 53))
    while True:
        q, who = s.recvfrom(4096)
        try: plan = answer(q, 'udp', s, who)
        except Exception as e:
            log('bad question', repr(e)); continue
        def go(plan=plan, who=who):
            at = time.time()
            for step in plan:
                wait = at + step[0] - time.time()
                if wait > 0: time.sleep(wait)
                s.sendto(step[1], who)
        threading.Thread(target=go, daemon=True).start()
def tcp():
    s = socket.socket(FAM, socket.SOCK_STREAM)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind((ADDR, 53)); s.listen(64)
    def one(c):
        try:
            n = struct.unpack('>H', c.recv(2))[0]
            q = b''
            while len(q) < n: q += c.recv(n - len(q))
            for step in answer(q, 'tcp'):
                time.sleep(step[0]); data = step[1]; how = step[2] if len(step) > 2 else None
                if how == 'huge': c.sendall(struct.pack('>H', 65535) + data + b'\0' * (65535 - len(data)))
                elif how == 'zero': c.sendall(struct.pack('>H', 0)); time.sleep(30)
                elif how == 'close': c.sendall(struct.pack('>H', len(data)) + data[:5])
                elif how == 'drip':
                    for b in struct.pack('>H', len(data)) + data:
                        c.sendall(bytes([b])); time.sleep(0.5)
                else: c.sendall(struct.pack('>H', len(data)) + data)
        except Exception as e: log('tcp', repr(e))
        finally: c.close()
    while True:
        c, _ = s.accept()
        threading.Thread(target=one, args=(c,), daemon=True).start()
threading.Thread(target=udp, daemon=True).start()
if WHO != 'notcp': threading.Thread(target=tcp, daemon=True).start()
while True: time.sleep(3600)
