import socket, sys, time, subprocess
path = sys.argv[1]
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect(path)
# a message header that says 100 bytes follow, and one byte of them: a sender stalled mid-message
s.sendall((100).to_bytes(4, 'little') + b'\x00')
time.sleep(0.3)
t = time.time()
r = subprocess.run(["curl", "-s", "--max-time", "4", "-o", "/dev/null", "-w", "%{http_code}", "localhost:39901/hit"], capture_output=True, text=True)
print("while a sender is stalled mid-message: /hit ->", r.stdout, "after %.1fs" % (time.time() - t))
s.close()
time.sleep(0.3)
t = time.time()
r = subprocess.run(["curl", "-s", "--max-time", "4", "localhost:39901/hit"], capture_output=True, text=True)
print("after the stalled sender closed: /hit ->", r.stdout, "after %.2fs" % (time.time() - t))
