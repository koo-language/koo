import socket, ssl, sys, threading
PORT = int(sys.argv[1])
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.load_cert_chain("tls/srv.pem", "tls/srv.key")
seen = []
ctx.sni_callback = lambda sock, name, c: print("SNI:", name, flush=True)
def h(c):
    try:
        t = ctx.wrap_socket(c, server_side=True)
    except Exception as e:
        print("handshake failed:", e, flush=True); return
    n = 0
    try:
        while True:
            d = t.recv(65536)
            if not d: break
            n += 1
            host = [l for l in d.split(b"\r\n") if l.lower().startswith(b"host:")]
            path = d.split(b" ")[1]
            auth = [l for l in d.split(b"\r\n") if l.lower().startswith(b"authorization:")]
            if path == b"/to-http":
                t.sendall(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:39601/echo\r\nContent-Length: 0\r\n\r\n"); continue
            if path == b"/to-other":
                t.sendall(b"HTTP/1.1 302 Found\r\nLocation: https://localhost:39632/a\r\nContent-Length: 0\r\n\r\n"); continue
            if path == b"/to-same":
                t.sendall(b"HTTP/1.1 302 Found\r\nLocation: /a\r\nContent-Length: 0\r\n\r\n"); continue
            body = b"secure ok, request %d on this connection, " % n + (host[0] if host else b"") + b", " + (auth[0] if auth else b"no authorization")
            t.sendall(b"HTTP/1.1 200 OK\r\nContent-Length: %d\r\n\r\n" % len(body) + body)
    except Exception as e:
        print("err", e, flush=True)
def serve(fam, addr):
    s = socket.socket(fam); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    if fam == socket.AF_INET6: s.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 1)
    s.bind((addr, PORT)); s.listen(16)
    while True:
        c, _ = s.accept(); threading.Thread(target=h, args=(c,), daemon=True).start()
threading.Thread(target=serve, args=(socket.AF_INET, "127.0.0.1"), daemon=True).start()
serve(socket.AF_INET6, "::1")
