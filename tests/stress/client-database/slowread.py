import socket, sys, time, threading
PORT = int(sys.argv[1]); RATE = int(sys.argv[2]); EVERY = float(sys.argv[3])
s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 65536)
s.bind(("127.0.0.1", PORT)); s.listen(64)
def h(c):
    total = 0
    try:
        while True:
            d = c.recv(RATE)
            if not d: break
            total += len(d)
            time.sleep(EVERY)
    except Exception as e:
        pass
    print("conn done, read", total, flush=True)
    c.close()
while True:
    c, a = s.accept()
    c.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 65536)
    threading.Thread(target=h, args=(c,), daemon=True).start()
