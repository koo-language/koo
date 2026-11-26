#!/usr/bin/env python3
# Reads a request and then resets the connection, as a server that dies does. With "half", after half an answer.
import socket, struct, sys, threading
s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(('127.0.0.1', int(sys.argv[1]))); s.listen(16)
while True:
    c, _ = s.accept()
    d = c.recv(65536)
    if b'/half' in d: c.sendall(b'HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabc')
    c.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack('ii', 1, 0))
    c.close()
