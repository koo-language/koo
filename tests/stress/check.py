"""A check of a Koo server under the kinds of load that have broken it before.

    python3 tests/stress/check.py <path to koo> [--target linux-x86-64] [--port 39800]
    python3 tests/stress/check.py --binary <check.koo, built already> [--port 39800]
                                  [--stderr <a file for what the server writes>]

Builds `check.koo`, starts it, and makes a series of checks against it, each of which
passes or fails: every request answered exactly once, counters exact, nothing left
spinning, open or running afterwards, and a prompt end when the server is told to
stop. Ends with status 1 if any check failed. Takes about two minutes. Needs only
Python 3 and, off Linux, `ps`, `pgrep` and `lsof`."""
import atexit, glob, http.client, shutil, struct, os, random, resource, signal, socket, subprocess, sys, tempfile, threading, time

here = os.path.dirname(os.path.abspath(__file__))
given = sys.argv[sys.argv.index("--binary") + 1] if "--binary" in sys.argv else None
koo = None if given else os.path.abspath(sys.argv[1])
target = sys.argv[sys.argv.index("--target") + 1] if "--target" in sys.argv else None
PORT = int(sys.argv[sys.argv.index("--port") + 1]) if "--port" in sys.argv else 39800
LINUX = sys.platform.startswith("linux")
soft, hard = resource.getrlimit(resource.RLIMIT_NOFILE)
want = 8192 if hard == resource.RLIM_INFINITY else min(8192, hard)
resource.setrlimit(resource.RLIMIT_NOFILE, (max(soft, want), hard))
threading.stack_size(256 * 1024)

failures = []


def ask(path, body=None, timeout=30):
    """One request on a connection of its own: (status or the error's name, body, seconds)."""
    began = time.time()
    try:
        conn = http.client.HTTPConnection("127.0.0.1", PORT, timeout=timeout)
        conn.request("POST" if body is not None else "GET", path, body=body)
        reply = conn.getresponse()
        text = reply.read()
        conn.close()
        return reply.status, text, time.time() - began
    except Exception as error:
        return f"{type(error).__name__} ({error})", b"", time.time() - began


class Client:
    """Requests one after another on a connection that is kept, as a real client's are:
    a machine has only so many ports for connections that are made and dropped."""

    def __init__(self, timeout=30):
        self.conn, self.timeout = None, timeout

    def ask(self, path, body=None):
        began = time.time()
        try:
            if self.conn is None:
                self.conn = http.client.HTTPConnection("127.0.0.1", PORT, timeout=self.timeout)
            self.conn.request("POST" if body is not None else "GET", path, body=body)
            reply = self.conn.getresponse()
            text = reply.read()
            return reply.status, text, time.time() - began
        except Exception as error:
            if self.conn is not None:
                self.conn.close()
            self.conn = None
            return f"{type(error).__name__} ({error})", b"", time.time() - began


steady = Client()


def fan(jobs):
    """Runs every job at once, each on a thread, and returns what they returned, in order."""
    out = [None] * len(jobs)

    def run(index):
        out[index] = jobs[index]()

    threads = [threading.Thread(target=run, args=(i,), daemon=True) for i in range(len(jobs))]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    return out


def children(pid):
    if LINUX:
        try:
            with open(f"/proc/{pid}/task/{pid}/children") as f:
                return [int(x) for x in f.read().split()]
        except OSError:
            return []
    out = subprocess.run(["pgrep", "-P", str(pid)], capture_output=True, text=True).stdout
    return [int(x) for x in out.split()]


def files(pid):
    if LINUX:
        try:
            return len(os.listdir(f"/proc/{pid}/fd"))
        except OSError:
            return 0
    out = subprocess.run(["lsof", "-p", str(pid)], capture_output=True, text=True).stdout
    return max(0, len(out.splitlines()) - 1)


def threads_of(pid):
    if LINUX:
        try:
            return len(os.listdir(f"/proc/{pid}/task"))
        except OSError:
            return 0
    out = subprocess.run(["ps", "-M", "-p", str(pid)], capture_output=True, text=True).stdout
    return max(0, len(out.splitlines()) - 1)


def processor(pid):
    """Seconds of processor the process has used."""
    if LINUX:
        try:
            with open(f"/proc/{pid}/stat") as f:
                fields = f.read().rsplit(")", 1)[1].split()
            return (int(fields[11]) + int(fields[12])) / os.sysconf("SC_CLK_TCK")
        except OSError:
            return 0.0
    out = subprocess.run(["ps", "-o", "time=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
    if not out:
        return 0.0
    parts = out.split(":")
    return sum(float(part) * 60 ** power for power, part in enumerate(reversed(parts)))


def socket_of(pid):
    """Where the supervising process is reached by its workers."""
    if LINUX:
        with open("/proc/net/unix") as f:
            for line in f:
                if f"@koo-{pid}-" in line:
                    return "\0" + line.split("@", 1)[1].strip()
        return None
    found = glob.glob(os.path.join(os.environ.get("TMPDIR", "/tmp"), f"koo-{pid}-*.sock"))
    return found[0] if found else None


def check(name, problems):
    """Reports one check: it passed if it has no problems to tell of."""
    problems = [p for p in problems if p]
    if problems:
        failures.append(name)
        print(f"FAIL  {name}")
        for problem in problems[:6]:
            print(f"        {problem}")
    else:
        print(f"pass  {name}")
    sys.stdout.flush()


def exactly_once(results, before, what):
    """What is wrong with the answers of `/next` to one actor: each number once, none left out."""
    bad = [r for r in results if r[0] != 200]
    if bad:
        return f"{what}: {len(bad)} of {len(results)} not answered with 200, the first {bad[0][0]} after {bad[0][2]:.1f}s"
    got = sorted(int(r[1]) for r in results)
    if got != list(range(before + 1, before + len(results) + 1)):
        return f"{what}: answers are not {before + 1}..{before + len(results)}, each once: {got[:5]}..{got[-3:]}"
    return None


def count_of(actor, path="seen"):
    """What an actor has counted. Asked again if the connection it was asked on had gone."""
    for _ in range(3):
        status, text, _ = steady.ask(f"/{path}/{actor}")
        if status == 200:
            return int(text)
    return -1


def start(workers):
    env = dict(os.environ, PORT=str(PORT), WORKERS=str(workers))
    env.pop("KOO_TARGET", None)
    if ask("/ping", timeout=1)[0] == 200:
        print(f"FAIL  something already answers on port {PORT}")
        sys.exit(1)
    server = subprocess.Popen([binary], env=env, stdout=subprocess.DEVNULL, stderr=open(errors, "a"))
    atexit.register(lambda: server.poll() is None and server.kill())
    for _ in range(100):
        if server.poll() is None and ask("/ping", timeout=1)[0] == 200:
            return server
        time.sleep(0.1)
    print("FAIL  the server did not start")
    sys.exit(1)


def stop(server, name):
    began = time.time()
    server.send_signal(signal.SIGTERM)
    try:
        status = server.wait(timeout=10)
        took = time.time() - began
        check(name, [status != 0 and f"exit status {status}", took > 3 and f"took {took:.1f}s to end"])
    except subprocess.TimeoutExpired:
        server.kill()
        check(name, ["still running ten seconds after it was told to stop"])


def stop_while_sending(server, name):
    """Told to stop while clients are still taking long answers, slowly: they get them whole."""
    size, got = 6000000, [0] * 4

    def reader(index):
        try:
            s = socket.socket()
            s.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 65536)
            s.settimeout(20)
            s.connect(("127.0.0.1", PORT))
            s.sendall(f"GET /big/{90 + index}/{size} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n".encode())
            data = b""
            while True:
                piece = s.recv(65536)
                if not piece:
                    break
                data += piece
                time.sleep(0.03)
            got[index] = len(data.split(b"\r\n\r\n", 1)[1]) if b"\r\n\r\n" in data else -1
        except OSError:
            got[index] = -2

    readers = [threading.Thread(target=reader, args=(i,), daemon=True) for i in range(4)]
    for t in readers:
        t.start()
    time.sleep(0.7)
    began = time.time()
    server.send_signal(signal.SIGTERM)
    for t in readers:
        t.join()
    problems = [any(n != size for n in got) and f"answers of {size} bytes arrived as {got}"]
    try:
        status = server.wait(timeout=15)
        problems.append(status != 0 and f"exit status {status}")
    except subprocess.TimeoutExpired:
        server.kill()
        problems.append("still running long after it was told to stop")
    check(name, problems)


def shared(name):
    """Many requests that wait, arriving one after another: they are shared out among the
    workers, and a quick request is answered at once by one that has room."""
    waits = []
    for i in range(300):
        t = threading.Thread(target=lambda: waits.append(ask("/wait/5000", timeout=30)[0]), daemon=True)
        t.start()
        waits.append(t)
        time.sleep(0.004)
    threads = [w for w in waits if isinstance(w, threading.Thread)]
    time.sleep(0.5)
    quick = [ask("/ping", timeout=4) for _ in range(10)]
    slowest = max(r[2] for r in quick)
    problems = [any(r[0] != 200 for r in quick) and f"quick requests were answered {sorted(set(str(r[0]) for r in quick))} beside 300 that wait"]
    problems.append(slowest > 2 and f"a quick request took {slowest:.1f}s beside 300 that wait")
    for t in threads:
        t.join()
    done = [w for w in waits if not isinstance(w, threading.Thread)]
    problems.append(any(status != 200 for status in done) and f"{sum(1 for status in done if status != 200)} of the 300 that waited were not answered")
    check(name, problems)


def message(method, body):
    """A message as a worker sends it to the supervising process: an ask of `Kind id method`."""
    inner = bytes([0]) + struct.pack("<H", len(method)) + method.encode() + body.encode()
    return struct.pack("<I", len(inner)) + inner


def forgers(name, server):
    """A local peer that writes well-formed messages no worker would: an actor named by a
    kind it is not of, and more on a connection than its answer has been waited for."""
    path = socket_of(server.pid)
    if path is None:
        check(name, ["the supervising process's socket was not found"])
        return
    problems = []
    for method in ("Names 0 all", "Names 3 add", "Counter 256 next"):
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(3)
        try:
            s.connect(path)
            s.sendall(message(method, '["x"]' if method.endswith("add") else "[]"))
            s.recv(200)
        except OSError:
            pass
        s.close()
    time.sleep(0.3)
    problems.append(server.poll() is not None and f"the server ended with status {server.returncode} when an actor was named by another kind")
    problems.append(ask("/names")[0] != 200 and "the actor of the other kind does not answer afterwards")
    problems.append(ask("/next/0")[0] != 200 and "the first actor does not answer afterwards")
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        s.connect(path)
        s.sendall(message("Counter 250 nap", "[3000]"))
        time.sleep(0.3)
        s.sendall(b"\x00")
    except OSError:
        problems.append("the supervising process could not be reached")
    time.sleep(0.3)
    used = processor(server.pid)
    time.sleep(2)
    used = processor(server.pid) - used
    problems.append(used > 0.5 and f"the supervising process used {used:.2f}s of processor in 2s over one byte too many")
    s.close()
    time.sleep(1)
    check(name, problems)


def burst(name, actors, each, rounds):
    problems = []
    for _ in range(rounds):
        before = {a: count_of(a) for a in actors}
        results = fan([(lambda a=a: (a, ask(f"/next/{a}"))) for a in actors for _ in range(each)])
        for a in actors:
            problems.append(exactly_once([r for who, r in results if who == a], before[a], f"actor {a}"))
    check(name, problems)


def prompt(name):
    """Asks one after another from forty clients, each the moment the last is answered,
    beside an actor that takes its time: a handler's ask goes out on the connection the
    one before it has just done with, and none waits for anything but its own actor."""
    def client(index):
        mine, slowest = Client(), 0.0
        for k in range(50):
            status, _, took = mine.ask(f"/next/{96 + (index + k) % 4}")
            slowest = max(slowest, took if status == 200 else 99.0)
        return slowest

    # Four ask one actor that takes its time, so that three messages are held up for
    # it; the asks of other actors beside them are answered as fast as ever.
    napping = [threading.Thread(target=lambda: ask("/nap/95/400"), daemon=True) for _ in range(4)]
    for t in napping:
        t.start()
    time.sleep(0.1)
    beside = max(ask(f"/next/{96 + k % 4}")[2] for k in range(40))
    slowest = max(fan([(lambda i=i: client(i)) for i in range(40)]))
    slowest = max(slowest, beside)
    for t in napping:
        t.join()
    check(name, [slowest > 0.5 and f"an ask took {slowest:.2f}s with nothing in its way"])


def fresh(name):
    """Actors of a second kind started, asked and stopped by handlers."""
    results = fan([(lambda: ask("/fresh")) for _ in range(40)])
    bad = [r for r in results if r[0] != 200 or r[1].strip() != b"2"]
    check(name, [bad and f"{len(bad)} of 40 were answered otherwise, the first {bad[0][0]} {bad[0][1][:40]!r}"])


def large(name):
    rng = random.Random(7)
    bodies = [bytes(rng.choice(b"abcdefghijklmnopqrstuvwxyz0123456789") for _ in range(4096)) * rng.randint(60, 220) for _ in range(24)]
    results = fan([(lambda i=i: ask(f"/echo/{80 + i % 8}", bodies[i], timeout=60)) for i in range(24)])
    problems = [f"echo {i}: {r[0]} after {r[2]:.1f}s, {len(r[1])} of {len(bodies[i])} bytes" for i, r in enumerate(results) if r[0] != 200 or r[1] != bodies[i]]
    results = fan([(lambda i=i: ask(f"/big/{88 + i}/3000000", timeout=60)) for i in range(8)])
    problems += [f"big {i}: {r[0]}, {len(r[1])} bytes" for i, r in enumerate(results) if r[0] != 200 or len(r[1]) != 3000000]
    check(name, problems)


def slow_and_quick(name):
    """Slow actors do not keep the quick ones waiting."""
    before = {a: count_of(a) for a in range(16, 32)}
    jobs = [(lambda a=a: ("nap", a, ask(f"/nap/{a}/400"))) for a in range(100, 200)]
    jobs += [(lambda a=a: ("next", a, ask(f"/next/{a}"))) for a in range(16, 32) for _ in range(12)]
    results = fan(jobs)
    naps = [r for kind, _, r in results if kind == "nap"]
    quick = [(a, r) for kind, a, r in results if kind == "next"]
    problems = [f"{sum(1 for r in naps if r[0] != 200)} slow asks were not answered" if any(r[0] != 200 for r in naps) else None]
    slowest = max(r[2] for _, r in quick)
    problems.append(slowest > 5 and f"a quick ask took {slowest:.1f}s beside the slow ones")
    for a in range(16, 32):
        problems.append(exactly_once([r for who, r in quick if who == a], before[a], f"actor {a}"))
    check(name, problems)


def tells(name):
    actors = range(32, 48)
    before = sum(count_of(a, "told") for a in actors)
    def teller(t):
        client = Client()
        return [client.ask(f"/tell/{32 + (t + k) % 16}")[0] for k in range(60)]

    results = fan([(lambda t=t: teller(t)) for t in range(50)])
    sent = sum(1 for batch in results for status in batch if status == 200)
    told = 0
    for _ in range(100):
        told = sum(count_of(a, "told") for a in actors) - before
        if told == sent:
            break
        time.sleep(0.1)
    check(name, [sent != 3000 and f"{sent} of 3000 tells were answered", told != sent and f"{told} of {sent} told messages arrived"])


def panics(name):
    """Each actor once: one that panics more than five times in a minute is left stopped.
    And no stack runs out in a server of one worker, which ends when one does."""
    results = fan([(lambda i=i: ask(f"/slip/{200 + i}")) for i in range(16)])
    problems = [any(r[0] != 500 for r in results) and f"a panic in an actor was answered with {sorted(set(str(r[0]) for r in results))}, not 500"]
    # (Nor where the server is being checked by a sanitizer, which does not know what to
    # make of a stack that runs out: `CHECK_NO_DIVE` says so.)
    if WORKERS > 1 and not os.environ.get("CHECK_NO_DIVE"):
        results = fan([(lambda i=i: ask(f"/dive/{216 + i}", timeout=60)) for i in range(8)])
        problems.append(any(r[0] != 500 for r in results) and f"a stack that ran out in an actor was answered with {sorted(set(str(r[0]) for r in results))}, not 500")
    results = fan([(lambda a=a: ask(f"/next/{a}")) for a in range(200, 224)])
    bad = [(200 + i, r[0]) for i, r in enumerate(results) if r[0] != 200]
    problems.append(bad and f"actors did not answer after they had panicked: {bad[:4]}")
    problems.append(ask("/ping")[0] != 200 and "the server does not answer after the panics")
    check(name, problems)


def bad_clients(name):
    """Clients that say nothing, or half of something, beside those that ask."""
    held = []
    for i in range(300):
        s = socket.socket()
        s.settimeout(5)
        try:
            s.connect(("127.0.0.1", PORT))
            if i % 3 == 1:
                s.sendall(b"GET /next/1 HT")
            elif i % 3 == 2:
                s.sendall(b"POST /echo/1 HTTP/1.1\r\nHost: x\r\nContent-Length: 1000\r\n\r\n0123456789")
        except OSError:
            pass
        held.append(s)
    actors = range(48, 64)
    before = {a: count_of(a) for a in actors}
    results = fan([(lambda a=a: (a, ask(f"/next/{a}", timeout=15))) for a in actors for _ in range(12)])
    problems = [exactly_once([r for who, r in results if who == a], before[a], f"actor {a}") for a in actors]
    for s in held:
        s.close()
    check(name, problems)


# How many rounds of ninety clients that leave, for each worker: fewer where the server
# is many times slower than it is, under a sanitizer, and is looked at for other things.
MANY = 1 if os.environ.get("CHECK_NO_DIVE") else 14


def left(name, server):
    """Clients that leave before they are answered: their handlers end, and do not keep
    places from requests that somebody waits for; an actor that was asked finishes what
    it began, and its next answer goes to whoever asks next; and thousands of them
    beside requests that are waited for cost those requests nothing."""

    def abandon(paths, after):
        socks = []
        for path in paths:
            try:
                sock = socket.create_connection(("127.0.0.1", PORT), timeout=10)
                sock.sendall(f"GET {path} HTTP/1.1\r\nHost: check\r\n\r\n".encode())
                socks.append(sock)
            except OSError:
                pass
        time.sleep(after)
        for sock in socks:
            sock.close()

    problems = []
    # Handlers that read of their requests after their clients have left: the path, a
    # header and the body are still there to read, though the connection is not.
    body = b"x" * 3000
    socks = []
    for _ in range(60):
        sock = socket.create_connection(("127.0.0.1", PORT), timeout=10)
        sock.sendall(b"POST /after/20000 HTTP/1.1\r\nHost: check\r\nX-Check: yes\r\nContent-Length: 3000\r\n\r\n" + body)
        socks.append(sock)
    time.sleep(0.2)
    for sock in socks:
        sock.close()
    time.sleep(0.3)
    status, text, took = ask("/after/5", body=b"abc", timeout=10)
    problems.append((status != 200 or text.strip() != b"5 /after/5 - 3") and f"after 60 clients left handlers that read their requests, a request was answered {status} {text[:60]!r}")
    problems.append(server.poll() is not None and f"the server ended, with status {server.poll()}")
    # Handlers that work between their waits, as many as there are workers and more,
    # left by their clients: with nothing to wait for they have two seconds of work
    # each, and the requests of others are answered between its pieces, not after it.
    abandon(["/grind"] * (2 * WORKERS), 0.3)
    worst, until = 0.0, time.time() + 2
    while time.time() < until:
        status, _, took = ask("/ping", timeout=10)
        worst = max(worst, took if status == 200 else 10.0)
        time.sleep(0.01)
    problems.append(worst > 0.5 and f"beside handlers that work on after their clients left, /ping waited {worst:.1f}s")
    # More handlers that would wait twenty seconds than the server has places for,
    # each left by its client a moment after it began.
    for _ in range(2 * WORKERS):
        abandon(["/wait/20000"] * 90, 0.1)
    time.sleep(0.3)
    status, _, took = ask("/wait/10", timeout=25)
    problems.append((status != 200 or took > 2) and f"after {180 * WORKERS} clients left, a request that waits 10 ms was answered {status} in {took:.1f}s")
    if WORKERS > 1:
        # Asks of actors in another process: the asker stops waiting, the actor does not stop.
        actors = list(range(100, 140))
        before = [count_of(a) for a in actors]
        abandon([f"/nap/{a}/1500" for a in actors], 0.2)
        status, _, took = ask("/ping", timeout=10)
        problems.append((status != 200 or took > 1) and f"after 40 askers left, /ping was answered {status} in {took:.1f}s")
        time.sleep(2)
        after = [count_of(a) for a in actors]
        ran = sum(1 for b, a in zip(before, after) if a == b + 1)
        problems.append(ran != len(actors) and f"{ran} of {len(actors)} methods whose askers left ran to their end, once")
        mixed = 0
        for a, counted in zip(actors, after):
            status, text, _ = ask(f"/next/{a}")
            if status != 200 or int(text) != counted + 1:
                mixed += 1
        problems.append(mixed and f"{mixed} asks after that got an answer that was not their own")
    # Several thousand more, while others are answered.
    stop_asking, refused, asked = threading.Event(), [], [0]

    def asker():
        while not stop_asking.is_set():
            status, text, took = ask("/ping", timeout=10)
            asked[0] += 1
            if status != 200 or took > 3:
                refused.append((status, round(took, 1)))
            time.sleep(0.02)

    beside = threading.Thread(target=asker, daemon=True)
    beside.start()
    for _ in range(MANY * WORKERS):
        abandon(["/wait/20000"] * 90, 0.03)
        time.sleep(0.05)
    stop_asking.set()
    beside.join()
    problems.append(refused and f"{len(refused)} of {asked[0]} requests beside {90 * MANY * WORKERS} clients that left were not answered well: {refused[:3]}")
    status, _, took = ask("/wait/10", timeout=25)
    problems.append((status != 200 or took > 2) and f"after {90 * MANY * WORKERS} more clients left, a request that waits 10 ms was answered {status} in {took:.1f}s")
    if WORKERS > 1:
        for _ in range(100):
            if len(children(server.pid)) == WORKERS:
                break
            time.sleep(0.1)
        problems.append(len(children(server.pid)) != WORKERS and f"{len(children(server.pid))} workers afterwards, not {WORKERS}")
    check(name, problems)


def kills(name, server):
    """Workers killed outright under load: nothing hangs, nothing is answered twice."""
    actors = list(range(64, 80))
    before = {a: count_of(a) for a in actors}
    until = time.time() + 8
    answers = {a: [] for a in actors}
    errors, slow, lock = [0], [0.0], threading.Lock()

    def client(seed):
        rng, mine = random.Random(seed), Client(timeout=15)
        while time.time() < until:
            a = rng.choice(actors)
            status, text, took = mine.ask(f"/next/{a}")
            with lock:
                slow[0] = max(slow[0], took)
                if status == 200:
                    answers[a].append(int(text))
                else:
                    errors[0] += 1

    clients = [threading.Thread(target=client, args=(i,), daemon=True) for i in range(64)]
    for t in clients:
        t.start()
    killed = 0
    while time.time() < until - 1:
        time.sleep(1)
        workers = children(server.pid)
        if workers:
            try:
                os.kill(random.choice(workers), signal.SIGKILL)
                killed += 1
            except OSError:
                pass
    for t in clients:
        t.join()
    problems = [killed == 0 and "no worker was there to kill"]
    problems.append(slow[0] > 12 and f"a request waited {slow[0]:.1f}s")
    total = sum(len(v) for v in answers.values())
    problems.append(total < 200 and f"only {total} requests were answered in eight seconds")
    lost = 0
    for a in actors:
        got = answers[a]
        if len(set(got)) != len(got):
            problems.append(f"actor {a} gave the same number twice")
        after = count_of(a)
        if after < before[a] + len(got):
            problems.append(f"actor {a} counted {after - before[a]} for {len(got)} answers: it lost what it had")
        lost += after - before[a] - len(got)
    problems.append(lost > errors[0] and f"{lost} asks were run whose answers nobody got, with only {errors[0]} requests cut off")
    for _ in range(50):
        if len(children(server.pid)) == WORKERS:
            break
        time.sleep(0.1)
    problems.append(len(children(server.pid)) != WORKERS and f"{len(children(server.pid))} workers afterwards, not {WORKERS}")
    check(name, problems)


def strangers(name, server):
    """Peers on the supervising process's own socket that are no workers of its."""
    path = socket_of(server.pid)
    if path is None:
        check(name, ["the supervising process's socket was not found"])
        return
    held = []
    rng = random.Random(3)
    for i in range(300):
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(2)
        try:
            s.connect(path)
            if i % 3 == 0:
                s.sendall(b"\x64")
            elif i % 3 == 1:
                s.sendall(b"\xff\xff\xff\x3f\x01\x05\x00ab")
            else:
                s.sendall(bytes(rng.randrange(256) for _ in range(64)))
        except OSError:
            pass
        held.append(s)
    time.sleep(0.5)
    actors = range(240, 256)
    before = {a: count_of(a) for a in actors}
    results = fan([(lambda a=a: (a, ask(f"/next/{a}", timeout=15))) for a in actors for _ in range(12)])
    problems = [exactly_once([r for who, r in results if who == a], before[a], f"actor {a}") for a in actors]
    used = processor(server.pid)
    time.sleep(2)
    used = processor(server.pid) - used
    problems.append(used > 0.5 and f"the supervising process used {used:.2f}s of processor in 2s with strangers connected")
    for s in held:
        s.close()
    problems.append(server.poll() is not None and f"the server ended with status {server.returncode}")
    check(name, problems)


def crowded(name, server):
    """More connections than the supervising process has room for, none of which it can
    let go of: it waits for one to end, and does not spin meanwhile."""
    path = socket_of(server.pid)
    if path is None:
        check(name, ["the supervising process's socket was not found"])
        return
    held, problems = [], []
    for i in range(4200):
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(2)
        # One that is refused for the moment, the others not yet taken, is tried again.
        for attempt in range(200):
            try:
                s.connect(path)
                s.sendall(b"\x64")
                held.append(s)
                break
            except OSError:
                if len(held) >= 4097:
                    break
                time.sleep(0.01)
        else:
            s.close()
    problems.append(len(held) < 4097 and f"only {len(held)} connections could be made, too few to fill the table")
    time.sleep(1)
    used = processor(server.pid)
    time.sleep(2)
    used = processor(server.pid) - used
    problems.append(used > 0.5 and f"the supervising process used {used:.2f}s of processor in 2s with its table full")
    for s in held:
        s.close()
    time.sleep(0.5)
    actors = range(224, 240)
    before = {a: count_of(a) for a in actors}
    results = fan([(lambda a=a: (a, ask(f"/next/{a}", timeout=15))) for a in actors for _ in range(12)])
    problems += [exactly_once([r for who, r in results if who == a], before[a], f"actor {a}") for a in actors]
    check(name, problems)


def no_room(name, server):
    """Every answering thread busy and no room for another connection to wait: a handler
    that asks then waits for room, and is not told that the process is gone. (Only Linux
    says at once that there is no room; elsewhere the one who connects is kept waiting.)"""
    path = socket_of(server.pid)
    if path is None:
        check(name, ["the supervising process's socket was not found"])
        return
    naps = [threading.Thread(target=lambda a=a: ask(f"/nap/{a}/4000", timeout=60), daemon=True) for a in range(100, 240)]
    for t in naps:
        t.start()
    time.sleep(1.5)
    held = []
    for i in range(4300):
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.setblocking(False)
        try:
            s.connect(path)
            held.append(s)
        except OSError:
            s.close()
            break
    # More at once than a worker has connections by, so that some must make one.
    results = fan([(lambda: ask("/tell/47", timeout=30)) for _ in range(60)])
    problems = [any(r[0] != 200 for r in results) and f"tells were answered with {sorted(set(str(r[0]) for r in results))} while the supervising process had no room"]
    for s in held:
        s.close()
    for t in naps:
        t.join()
    problems.append(ask("/ping")[0] != 200 and "the server does not answer afterwards")
    check(name, problems)


def settled(name, server, files_before):
    """Some time after the load: nothing spins, and nothing was left open or running."""
    time.sleep(13)
    pids = [server.pid] + children(server.pid)
    before = {pid: processor(pid) for pid in pids}
    time.sleep(3)
    problems = []
    for pid in pids:
        used = processor(pid) - before[pid]
        who = "the supervising process" if pid == server.pid else f"worker {pid}"
        problems.append(used > 0.3 and f"{who} used {used:.2f}s of processor in 3s with nothing to do")
    now = files(server.pid)
    problems.append(now > files_before + 16 and f"the supervising process has {now} files open, and had {files_before} after the first load")
    running = threads_of(server.pid)
    problems.append(running > 160 and f"the supervising process has {running} threads")
    problems.append(ask("/ping")[0] != 200 and "the server does not answer")
    check(name, problems)


build = tempfile.mkdtemp(prefix="koo-check-")
binary, errors = os.path.join(build, "check"), os.path.join(build, "stderr.txt")
if "--stderr" in sys.argv:
    errors = os.path.abspath(sys.argv[sys.argv.index("--stderr") + 1])
if given:
    binary = os.path.abspath(given)
else:
    command = [koo, "build", "--release", os.path.join(here, "check.koo"), "-o", binary]
    if target:
        command[2:2] = ["--target", target]
    built = subprocess.run(command, capture_output=True, text=True, env={k: v for k, v in os.environ.items() if k != "KOO_TARGET"})
    if built.returncode != 0:
        print("FAIL  check.koo was not built\n" + built.stderr)
        sys.exit(1)

WORKERS = 4
print(f"== {WORKERS} workers: the actors are in the supervising process")
server = start(WORKERS)
burst("600 asks at once, three times over, each answered exactly once", range(0, 16), 38, 3)
# The workers keep the connections they asked on, for the next to ask: what is open now
# is what is open after any such load, and is compared with what is open at the end.
time.sleep(1)
files_before = files(server.pid)
prompt("asks one after another are each answered at once")
fresh("handlers start, ask and stop actors of a second kind")
large("long messages and long answers arrive whole")
slow_and_quick("slow actors do not hold up quick ones")
tells("3000 told messages all arrive")
panics("a panic or an exhausted stack in an actor costs one request")
bad_clients("300 clients that say half of something do not hold up the rest")
shared("300 requests that wait are shared out, and a quick one is answered at once")
left("clients that leave are not kept places for, and what they asked of actors is done", server)
kills("workers killed outright: nothing hangs, repeats or forgets", server)
strangers("300 strangers on the supervising process's socket do no harm", server)
forgers("a peer that names an actor by another kind, or says too much, does no harm", server)
crowded("a table full of connections that cannot be let go of is waited out, not spun on", server)
if LINUX:
    no_room("a supervising process with no room for another connection is waited for", server)
burst("600 asks at once again, after all that", range(0, 16), 38, 1)
settled("afterwards nothing spins and nothing is left open", server, files_before)
stop(server, "told to stop, the server ends at once with status 0")
server = start(WORKERS)
stop_while_sending(server, "told to stop while long answers are going out, the server sends them whole")

WORKERS = 1
print(f"== {WORKERS} worker: the actors are in the process that answers")
server = start(WORKERS)
burst("300 asks at once, each answered exactly once", range(0, 16), 19, 2)
fresh("handlers start, ask and stop actors of a second kind")
large("long messages and long answers arrive whole")
tells("3000 told messages all arrive")
panics("a panic in an actor costs one request")
bad_clients("300 clients that say half of something do not hold up the rest")
left("clients that leave are not kept places for", server)
stop(server, "told to stop, the server ends at once with status 0")

if failures:
    print(f"\n{len(failures)} failed; what the server wrote is in {errors}")
    sys.exit(1)
# Nothing is kept of a run in which nothing failed.
shutil.rmtree(build, ignore_errors=True)
print("\nall passed")
