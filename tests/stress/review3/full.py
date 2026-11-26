import socket, sys, time, subprocess, resource
resource.setrlimit(resource.RLIMIT_NOFILE, (8192, resource.getrlimit(resource.RLIMIT_NOFILE)[1]))
path, pid = sys.argv[1], sys.argv[2]
def cpu():
    out = subprocess.run(["ps", "-o", "time=", "-p", pid], capture_output=True, text=True).stdout.strip()
    m, s = out.split(":"); return int(m) * 60 + float(s)
def burn(label, seconds=3):
    a = cpu(); time.sleep(seconds); b = cpu()
    print("%s supervisor used %.2fs of processor in %ds" % (label, b - a, seconds))
held = []
for i in range(4096):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    while True:
        try:
            s.connect(path); break
        except (ConnectionRefusedError, BlockingIOError):
            time.sleep(0.01)
    s.sendall(b"\x64")          # one byte of a message's length: a sender in the middle of a message
    held.append(s)
time.sleep(1.0)
burn("table full (4096 connections each mid-message), nothing waiting to be taken:")
extra = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
extra.connect(path)
time.sleep(0.2)
burn("one more connection waiting to be taken:")
extra.close()
time.sleep(0.5)
burn("after that one went away:")
