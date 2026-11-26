#!/usr/bin/env python3
"""A hostile HTTP server. Behaviour is chosen by the path. Logs each request to stderr-like file."""
import socket, sys, threading, time, os

PORT = int(sys.argv[1])
FAMILY = sys.argv[2] if len(sys.argv) > 2 else "4"
LOG = open(sys.argv[3], "a") if len(sys.argv) > 3 else sys.stderr

def log(*a):
    print(*a, file=LOG, flush=True)

def read_request(conn):
    data = b""
    while b"\r\n\r\n" not in data:
        chunk = conn.recv(65536)
        if not chunk:
            return None
        data += chunk
    head, rest = data.split(b"\r\n\r\n", 1)
    lines = head.split(b"\r\n")
    headers = {}
    for l in lines[1:]:
        k, _, v = l.partition(b":")
        headers[k.strip().lower()] = v.strip()
    n = int(headers.get(b"content-length", b"0"))
    while len(rest) < n:
        chunk = conn.recv(65536)
        if not chunk:
            break
        rest += chunk
    return lines[0], headers, rest, head

def handle(conn, addr):
    served = 0
    try:
        while True:
            req = read_request(conn)
            if req is None:
                return
            first, headers, body, head = req
            served += 1
            method, path, _ = first.split(b" ", 2)
            log("REQ", served, first.decode("latin1"), "| auth=", headers.get(b"authorization"), "cookie=", headers.get(b"cookie"), "host=", headers.get(b"host"), "bodylen=", len(body), "ctype=", headers.get(b"content-type"))
            p = path.split(b"?")[0]
            def send(b):
                conn.sendall(b)
            def ok(body=b"ok", extra=b"", status=b"200 OK"):
                send(b"HTTP/1.1 " + status + b"\r\nContent-Length: " + str(len(body)).encode() + b"\r\n" + extra + b"\r\n" + body)
            if p == b"/ok":
                ok(b"ok %d" % served)
            elif p == b"/echo":
                ok(head + b"\r\n\r\n" + body)
            elif p == b"/truncated":
                send(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nshort"); return
            elif p == b"/noclen":
                send(b"HTTP/1.1 200 OK\r\n\r\nbody until close"); return
            elif p == b"/noclen-keep":
                send(b"HTTP/1.1 200 OK\r\nConnection: keep-alive\r\n\r\nbody until close"); time.sleep(30); return
            elif p == b"/chunked":
                send(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6;ext=1\r\n world\r\n0\r\nTrailer: x\r\n\r\n")
            elif p == b"/chunked-bad":
                send(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhelloXX\r\n0\r\n\r\n"); return
            elif p == b"/chunked-trunc":
                send(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n10\r\nabc"); return
            elif p == b"/chunked-neg":
                send(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n-5\r\nhello\r\n0\r\n\r\n"); return
            elif p == b"/chunked-huge":
                send(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nffffffff\r\nhello\r\n0\r\n\r\n"); return
            elif p == b"/chunked-and-clen":
                send(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n")
            elif p == b"/chunked-upper":
                send(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: Chunked\r\n\r\nA\r\n0123456789\r\n0\r\n\r\n")
            elif p == b"/two-clen":
                send(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Length: 5\r\n\r\nhello")
            elif p == b"/neg-clen":
                send(b"HTTP/1.1 200 OK\r\nContent-Length: -1\r\n\r\nhello"); return
            elif p == b"/garbage":
                send(b"SSH-2.0-OpenSSH\r\n\r\n"); return
            elif p == b"/empty":
                return
            elif p == b"/http10":
                send(b"HTTP/1.0 200 OK\r\n\r\nold style"); return
            elif p == b"/204":
                send(b"HTTP/1.1 204 No Content\r\n\r\n")
            elif p == b"/204-body":
                send(b"HTTP/1.1 204 No Content\r\nContent-Length: 5\r\n\r\nhello")
            elif p == b"/304":
                send(b"HTTP/1.1 304 Not Modified\r\nContent-Length: 5\r\n\r\n")
            elif p == b"/head":
                send(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n" + (b"" if method == b"HEAD" else b"hello"))
            elif p == b"/100":
                send(b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nafter")
            elif p == b"/fold":
                send(b"HTTP/1.1 200 OK\r\nX-Fold: a\r\n b\r\nContent-Length: 2\r\n\r\nok")
            elif p == b"/dup":
                send(b"HTTP/1.1 200 OK\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2\r\nX-Empty:\r\nX-Colon: a: b\r\nContent-Length: 2\r\n\r\nok")
            elif p == b"/lf":
                send(b"HTTP/1.1 200 OK\nContent-Length: 2\n\nok")
            elif p == b"/bighead":
                send(b"HTTP/1.1 200 OK\r\nX-Big: " + b"a" * 100000 + b"\r\nContent-Length: 2\r\n\r\nok"); return
            elif p == b"/manyheads":
                send(b"HTTP/1.1 200 OK\r\n" + b"".join(b"X-%d: v\r\n" % i for i in range(20000)) + b"Content-Length: 2\r\n\r\nok"); return
            elif p == b"/nostatus":
                send(b"HTTP/1.1 OK\r\nContent-Length: 2\r\n\r\nok"); return
            elif p == b"/status999":
                send(b"HTTP/1.1 999 Weird\r\nContent-Length: 2\r\n\r\nok")
            elif p == b"/status000":
                send(b"HTTP/1.1 000 Weird\r\nContent-Length: 2\r\n\r\nok")
            elif p == b"/binary":
                ok(bytes(range(256)))
            elif p == b"/nul":
                ok(b"a\x00b\x00c")
            elif p == b"/badutf8":
                ok(b"abc\xff\xfe")
            elif p == b"/big":
                n = int(path.split(b"=")[1])
                send(b"HTTP/1.1 200 OK\r\nContent-Length: %d\r\n\r\n" % n)
                blk = b"x" * 65536
                left = n
                while left > 0:
                    send(blk[:min(left, 65536)]); left -= 65536
            elif p == b"/drip":
                # one byte every 0.2s, for ever
                send(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n")
                for i in range(1000):
                    send(b"x"); time.sleep(0.2)
            elif p == b"/drip-head":
                send(b"HTTP/1.1 200 OK\r\n")
                for i in range(1000):
                    send(b"X-%d: y\r\n" % i); time.sleep(0.2)
            elif p == b"/stall":
                time.sleep(60); return
            elif p == b"/close-after":
                ok(b"bye %d" % served); return
            elif p == b"/close-said":
                ok(b"bye", b"Connection: close\r\n")
                time.sleep(5); return
            elif p == b"/extra":
                ok(b"first"); send(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\npoison")
            elif p == b"/late-extra":
                ok(b"first"); time.sleep(0.3); send(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\npoison")
            elif p.startswith(b"/r/"):
                # /r/<status>?to=<location> (location is hex)
                status = p.split(b"/")[2]
                to = bytes.fromhex(path.split(b"to=")[1].decode())
                send(b"HTTP/1.1 " + status + b" Moved\r\nLocation: " + to + b"\r\nContent-Length: 0\r\n\r\n")
            elif p == b"/" and b"to=" in path:
                try:
                    to = bytes.fromhex(path.split(b"to=")[1].split(b"&")[0].decode())
                    send(b"HTTP/1.1 302 Moved\r\nLocation: " + to + b"\r\nContent-Length: 0\r\n\r\n")
                except ValueError:
                    ok(b"the request was for " + path)
            elif p.startswith(b"/id/"):
                import random
                n = p.split(b"/")[2]
                time.sleep(random.random() * 0.05)
                pad = b"." * (int(n) % 5000)
                if int(n) % 3 == 0:
                    send(b"HTTP/1.1 200 OK\r\nX-Id: " + n + b"\r\nTransfer-Encoding: chunked\r\n\r\n" + hex(len(n) + 3)[2:].encode() + b"\r\nid " + n + b"\r\n")
                    time.sleep(random.random() * 0.02)
                    send(hex(len(pad))[2:].encode() + b"\r\n" + pad + b"\r\n0\r\n\r\n" if pad else b"0\r\n\r\n")
                elif int(n) % 7 == 0:
                    ok(b"id " + n + pad, b"X-Id: " + n + b"\r\n"); return
                else:
                    ok(b"id " + n + pad, b"X-Id: " + n + b"\r\n")
            elif p == b"/loop":
                send(b"HTTP/1.1 302 Found\r\nLocation: /loop\r\nContent-Length: 0\r\n\r\n")
            elif p == b"/slowloop":
                time.sleep(0.4)
                send(b"HTTP/1.1 302 Found\r\nLocation: /slowloop\r\nContent-Length: 0\r\n\r\n")
            elif p == b"/slowread":
                ok(b"got %d" % len(body))
            else:
                ok(b"default " + path, status=b"404 Not Found")
    except Exception as e:
        log("ERR", repr(e))
    finally:
        try: conn.close()
        except: pass

def main():
    if FAMILY == "6":
        s = socket.socket(socket.AF_INET6, socket.SOCK_STREAM)
        s.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 1)
        s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        s.bind(("::1", PORT))
    else:
        s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        s.bind(("127.0.0.1", PORT))
    s.listen(256)
    while True:
        c, a = s.accept()
        threading.Thread(target=handle, args=(c, a), daemon=True).start()

main()
