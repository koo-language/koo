#!/usr/bin/env python3
# A name server that answers every question with a stream of answers that have the wrong id, from several processes.
import socket, os, sys, time
ADDR = sys.argv[1]; PROCS = int(sys.argv[2]) if len(sys.argv) > 2 else 6; SECONDS = float(sys.argv[3]) if len(sys.argv) > 3 else 20
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEPORT, 1)
s.bind((ADDR, 53))
while True:
    q, who = s.recvfrom(4096)
    print('[flood] asked by', who, flush=True)
    bad = bytes([q[0] ^ 0xff, q[1]]) + b'\x81\x80' + q[4:]
    for _ in range(PROCS):
        if os.fork() == 0:
            end = time.time() + SECONDS
            out = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            out.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEPORT, 1)
            out.bind((ADDR, 53))
            n = 0
            while True:
                for _ in range(2000):
                    try: out.sendto(bad, who)
                    except OSError: pass
                n += 2000
                if time.time() > end: break
            os._exit(0)
