import socket, ssl, sys, threading
PORT = int(sys.argv[1])
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.load_cert_chain("tls/srv.pem", "tls/srv.key")
def h(c):
    try:
        c.recv(8); c.sendall(b"S")
        try:
            t = ctx.wrap_socket(c, server_side=True)
        except Exception as e:
            print("handshake failed:", e, flush=True); return
        print("handshake ok, sni callback n/a", flush=True)
        t.recv(4096)
        msg = b"SFATAL\0C28000\0Mthe handshake was fine, and this test server lets nobody in\0\0"
        t.sendall(b"E" + (len(msg) + 4).to_bytes(4, "big") + msg)
        t.close()
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
