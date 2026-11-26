import socket, sys, time, subprocess, resource
resource.setrlimit(resource.RLIMIT_NOFILE, (8192, resource.getrlimit(resource.RLIMIT_NOFILE)[1]))
path = sys.argv[1]
def hit(label):
    t = time.time()
    r = subprocess.run(["curl", "-s", "--max-time", "5", "-o", "/dev/null", "-w", "%{http_code}", "localhost:39901/hit"], capture_output=True, text=True)
    print(label, "/hit ->", r.stdout, "after %.2fs" % (time.time() - t))
idle = []
for i in range(4096):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(path)
    idle.append(s)
time.sleep(1.5)
hit("4096 idle connections held (as kept connections are):")
hit("again:")
for s in idle[:200]: s.close()
time.sleep(1.5)
hit("after 200 of them closed:")
