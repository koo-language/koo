import socket, sys, threading
PORT = int(sys.argv[1])
s = socket.socket(socket.AF_INET6); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 1)
s.bind(("::1", PORT)); s.listen(64)
def pump(a, b):
    try:
        while True:
            d = a.recv(65536)
            if not d: break
            b.sendall(d)
    except Exception: pass
    for x in (a, b):
        try: x.shutdown(socket.SHUT_RDWR)
        except Exception: pass
        try: x.close()
        except Exception: pass
while True:
    c, _ = s.accept()
    u = socket.create_connection(("127.0.0.1", 54329))
    threading.Thread(target=pump, args=(c, u), daemon=True).start()
    threading.Thread(target=pump, args=(u, c), daemon=True).start()
