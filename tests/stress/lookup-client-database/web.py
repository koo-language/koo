#!/usr/bin/env python3
# Answers every request with the address it was reached at.
import socket, threading, sys
def serve(fam, addr, port):
    s = socket.socket(fam, socket.SOCK_STREAM); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    if fam == socket.AF_INET6: s.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 1)
    s.bind((addr, port)); s.listen(64)
    while True:
        c, _ = s.accept()
        def one(c=c):
            try:
                c.recv(65536)
                body = ('reached at %s' % c.getsockname()[0]).encode()
                c.sendall(b'HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: %d\r\n\r\n' % len(body) + body)
            finally: c.close()
        threading.Thread(target=one, daemon=True).start()
threading.Thread(target=serve, args=(socket.AF_INET, '0.0.0.0', 8080), daemon=True).start()
try: threading.Thread(target=serve, args=(socket.AF_INET6, '::', 8080), daemon=True).start()
except Exception as e: print('no v6', e)
import time
while True: time.sleep(3600)
