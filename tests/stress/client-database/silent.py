import socket, sys, threading, time
PORT = int(sys.argv[1]); MODE = sys.argv[2]
s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", PORT)); s.listen(64)
keep = []
def h(c):
    try:
        if MODE == "silent":
            time.sleep(3600)
        elif MODE == "N-then-silent":
            c.recv(8); c.sendall(b"N"); time.sleep(3600)
        elif MODE == "garbage":
            c.recv(8); c.sendall(b"N"); c.recv(4096); c.sendall(b"HTTP/1.1 400 Bad Request\r\n\r\n"); time.sleep(1); c.close()
        elif MODE == "hugelen":
            c.recv(8); c.sendall(b"N"); c.recv(4096); c.sendall(b"R\x3f\xff\xff\xff" + b"\0" * 100); time.sleep(3600)
        elif MODE == "auth-ok-then-silent":
            c.recv(8); c.sendall(b"N"); c.recv(4096); c.sendall(b"R\0\0\0\x08\0\0\0\0" + b"Z\0\0\0\x05I")
            c.recv(65536); time.sleep(3600)
    except Exception as e:
        print(e, flush=True)
while True:
    c, a = s.accept(); keep.append(c)
    threading.Thread(target=h, args=(c,), daemon=True).start()
