# Opens connections to the supervising process's socket that nobody takes, as the asks
# of other workers would be while every answering thread is busy.
import socket, sys, time, errno
name = None
for line in open("/proc/net/unix"):
    if "@koo-%s-" % sys.argv[1] in line:
        name = line.split("@", 1)[1].strip()
held, refused = [], 0
for i in range(1100):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.setblocking(False)
    try:
        s.connect("\0" + name)
        held.append(s)
    except OSError as e:
        refused += 1
        s.close()
print("filler: %d connections waiting to be taken, %d refused for want of room" % (len(held), refused), flush=True)
time.sleep(float(sys.argv[2]))
