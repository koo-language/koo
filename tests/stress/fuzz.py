#!/usr/bin/env python3
"""Throws what is nearly right at everything the support library reads from outside.

    python3 tests/stress/fuzz.py <path to koo> [--binary built-server] [--seconds 15] [--seed 1]

Five readers, each for so many seconds: requests of HTTP, bodies of JSON, and the
answers of a server of HTTP, of a PostgreSQL and of a Redis, which three are peers made
here that answer with a right answer spoiled in one way or another. The server aimed at
is `tests/stress/fuzz/main.koo`. Whatever it is sent, it must go on: answer a plain
request afterwards, keep the workers it began with (one that fell over is replaced, and
would otherwise not be missed), write no panic of its own and no report of a sanitizer
it was built with (`fuzz.sh`), and end with status 0 when it is told to stop.

What a run threw is decided by its seed, which it prints; a run that fails says what
the server wrote, and the same seed throws the same again.
"""
import http.client, os, random, shutil, signal, socket, struct, subprocess, sys, tempfile, threading, time

args = sys.argv[1:]


def option(name, default):
    return args[args.index(name) + 1] if name in args else default


SECONDS = float(option("--seconds", os.environ.get("FUZZ_SECONDS", "15")))
SEED = int(option("--seed", str(int(time.time()))))


def free_port():
    """A port that nothing has: asked of the system, since one that is written down may
    be in use the day the check is run, by a connection that was given it by chance."""
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


PORT = int(option("--port", "0")) or free_port()
# How long the plain request after each reader may take. A server built for a sanitizer
# does without the stacks it switches between, so each of its workers answers one
# request at a time, and those still waiting for a peer that will not answer are each
# waited out before the plain one is reached: `fuzz.sh` allows for that.
PATIENCE = float(option("--patience", "10"))
# And with `--in-turn` the clients of the three peers' readers wait for each answer
# before they ask again, so that such a server is not left with more to answer than it
# can in that time. (Its handlers are given less time each than five seconds: `DEADLINE_MS`.)
IN_TURN = "--in-turn" in args
rng = random.Random(SEED)
root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def spoil(data, rng):
    """A right thing made nearly right: a few bytes changed, cut, doubled or put in."""
    data = bytearray(data)
    for _ in range(rng.choice([1, 1, 1, 2, 3, 8])):
        if not data:
            break
        how = rng.randrange(8)
        at = rng.randrange(len(data))
        if how == 0:
            data[at] = rng.randrange(256)
        elif how == 1:
            del data[at:at + rng.choice([1, 2, 8, 64])]
        elif how == 2:
            data[at:at] = data[at:at + rng.choice([1, 4, 32, 256])]
        elif how == 3:
            data[at:at] = bytes(rng.randrange(256) for _ in range(rng.choice([1, 3, 16])))
        elif how == 4:
            del data[at:]
        elif how == 5:
            data[at:at] = rng.choice([b"\r\n", b"\n", b"\x00", b"\xff\xfe", b"%00", b"\\u0000", b"\\ud800", b'"', b"{", b"[" * 40])
        elif how == 6:
            # A number in it made one that no reader should take on trust.
            digits = [i for i, b in enumerate(data) if 48 <= b <= 57]
            if digits:
                at = rng.choice(digits)
                data[at:at + 1] = rng.choice([b"99999999999999999999", b"-1", b"4294967296", b"2147483648", b"0x10", b"1e400", b"00"])
        else:
            data[at] ^= 1 << rng.randrange(8)
    return bytes(data)


# ---- the peers that answer the server -----------------------------------------------------

def listen():
    server = socket.socket()
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server.bind(("127.0.0.1", 0))
    server.listen(128)
    return server, server.getsockname()[1]


spoiling = threading.Event()


def serve(server, answer):
    def run():
        while True:
            try:
                connection, _ = server.accept()
            except OSError:
                return
            threading.Thread(target=one, args=(connection,), daemon=True).start()

    def one(connection):
        connection.settimeout(5)
        mine = random.Random(rng.random())
        try:
            answer(connection, mine)
        except (OSError, ValueError, struct.error, IndexError):
            pass
        finally:
            connection.close()

    threading.Thread(target=run, daemon=True).start()


def send(connection, data, mine):
    """Whole, or spoiled; and now and then in pieces, or half and no more."""
    if spoiling.is_set() and mine.random() < 0.8:
        data = spoil(data, mine)
        if mine.random() < 0.1:
            connection.sendall(data[:len(data) // 2])
            raise OSError("half")
        if mine.random() < 0.2:
            for at in range(0, len(data), 7):
                connection.sendall(data[at:at + 7])
            return
    connection.sendall(data)


def other_server(connection, mine):
    data = b""
    while True:
        while b"\r\n\r\n" not in data:
            more = connection.recv(4096)
            if not more:
                return
            data += more
        data = b""
        body = b"x" * mine.choice([0, 5, 5000])
        if mine.random() < 0.3:
            chunks = b"".join(b"%x\r\n%s\r\n" % (len(part), part) for part in [body[:3], body[3:]] if part) + b"0\r\n\r\n"
            reply = b"HTTP/1.1 200 OK\r\nX-Up: yes\r\nTransfer-Encoding: chunked\r\n\r\n" + chunks
        elif mine.random() < 0.1:
            reply = b"HTTP/1.1 302 Found\r\nLocation: /again\r\nContent-Length: 0\r\n\r\n"
        else:
            reply = b"HTTP/1.1 200 OK\r\nX-Up: yes\r\nContent-Length: %d\r\n\r\n%s" % (len(body), body)
        send(connection, reply, mine)


def message(kind, body):
    return kind + struct.pack(">i", len(body) + 4) + body


def database(connection, mine):
    def exactly(count):
        data = b""
        while len(data) < count:
            more = connection.recv(count - len(data))
            if not more:
                raise OSError("gone")
            data += more
        return data

    # The beginning is said as it should be: what is spoiled is what follows.
    size = struct.unpack(">i", exactly(4))[0]
    first = exactly(size - 4)
    if first[:4] == struct.pack(">i", 80877103):
        connection.sendall(b"N")
        size = struct.unpack(">i", exactly(4))[0]
        first = exactly(size - 4)
    if first[:4] == struct.pack(">i", 80877102):
        return
    connection.sendall(message(b"R", struct.pack(">i", 0)) + message(b"S", b"server_version\x0016.0\x00")
                       + message(b"K", struct.pack(">ii", 7, 9)) + message(b"Z", b"I"))
    while True:
        kind = exactly(1)
        size = struct.unpack(">i", exactly(4))[0]
        exactly(size - 4)
        if kind == b"X":
            return
        if kind not in (b"S", b"Q"):
            continue
        columns = struct.pack(">h", 2) + b"".join(
            name + b"\x00" + struct.pack(">ihihih", 0, 0, oid, -1, -1, 0) for name, oid in [(b"a", 25), (b"b", 23)])
        rows = b"".join(
            message(b"D", struct.pack(">h", 2) + struct.pack(">i", len(a)) + a + struct.pack(">i", len(b)) + b)
            for a, b in [(b"text", b"12"), (b"\xc3\xa9", b"-3"), (b"", b"7")][:mine.randrange(4)])
        reply = (message(b"1", b"") + message(b"2", b"") + message(b"T", columns) + rows
                 + message(b"C", b"SELECT 3\x00") + message(b"Z", b"I"))
        if mine.random() < 0.1:
            reply = (message(b"1", b"") + message(b"2", b"")
                     + message(b"E", b"SERROR\x00C42601\x00Msyntax error\x00\x00") + message(b"Z", b"I"))
        send(connection, reply, mine)


def cache(connection, mine):
    lines = connection.makefile("rb")
    while True:
        head = lines.readline()
        if not head:
            return
        words = []
        for _ in range(int(head.strip()[1:] or b"0")):
            lines.readline()
            words.append(lines.readline().strip())
        # (The word with which a client sees that the server is there, when it connects,
        # is answered as a server answers it: a program that cannot connect does not
        # start, and what is thrown at it is for after it has.)
        if words[:1] == [b"PING"]:
            send(connection, b"+PONG\r\n", mine)
            continue
        reply = mine.choice([b"$5\r\nvalue\r\n", b"$-1\r\n", b"+OK\r\n", b":42\r\n", b"-ERR no\r\n",
                             b"*2\r\n$1\r\na\r\n*1\r\n:1\r\n", b"$0\r\n\r\n"])
        send(connection, reply, mine)


# ---- the server -----------------------------------------------------------------------------

build = tempfile.mkdtemp(prefix="koo-fuzz-")
errors = os.path.join(build, "stderr.txt")
binary = option("--binary", os.path.join(build, "server"))
if "--binary" not in args:
    package = os.path.join(root, "tests/stress/fuzz")
    built = subprocess.run([args[0], "build", package, "-o", binary], capture_output=True, text=True)
    if built.returncode != 0:
        print("FAIL  the server was not built\n" + built.stderr)
        sys.exit(1)

peers = {}
for name, answer in [("UP_HTTP", other_server), ("UP_PG", database), ("UP_REDIS", cache)]:
    server, port = listen()
    serve(server, answer)
    peers[name] = port
# (More workers where each answers one request at a time, so that as many are read.)
environment = dict(os.environ, PORT=str(PORT), WORKERS="8" if IN_TURN else "2", KOO_CHECK="1",
                   UP_HTTP=f"http://127.0.0.1:{peers['UP_HTTP']}/", UP_PG=f"postgres://fuzz@127.0.0.1:{peers['UP_PG']}/fuzz?sslmode=disable",
                   UP_REDIS=f"redis://127.0.0.1:{peers['UP_REDIS']}",
                   ASAN_OPTIONS="detect_leaks=0:abort_on_error=0", TSAN_OPTIONS="die_after_fork=0")
target = subprocess.Popen([binary], env=environment, stdout=subprocess.DEVNULL, stderr=open(errors, "w"))


def ask(path, timeout=10):
    try:
        connection = http.client.HTTPConnection("127.0.0.1", PORT, timeout=timeout)
        connection.request("GET", path)
        reply = connection.getresponse()
        reply.read()
        connection.close()
        return reply.status
    except Exception as error:
        return type(error).__name__


for attempt in range(100):
    if ask("/ping", 1) == 200:
        break
    time.sleep(0.1)
else:
    print("FAIL  the server did not start:\n" + open(errors).read()[-2000:])
    target.kill()
    sys.exit(1)


def children(pid):
    try:
        with open(f"/proc/{pid}/task/{pid}/children") as f:
            return sorted(f.read().split())
    except OSError:
        out = subprocess.run(["pgrep", "-P", str(pid)], capture_output=True, text=True).stdout
        return sorted(out.split())


workers = children(target.pid)
failures = []
# Before anything is spoiled, each of the peers is read rightly: a reader that is never
# reached is not looked at, however much is thrown.
# (A peer also answers with a failure of its own now and then, which is a right answer
# and is passed on as one: so each is asked several times, and must once be read.)
for path in ["/fetch", "/pg", "/redis"]:
    statuses = []
    while 200 not in statuses and len(statuses) < 12:
        statuses.append(ask(path))
    if 200 not in statuses:
        print(f"FAIL  {path} is answered {statuses} when its peer answers rightly:\n" + open(errors).read()[-1500:])
        target.kill()
        sys.exit(1)
print(f"seed {SEED}, {SECONDS:.0f} seconds for each reader")

# ---- what is thrown ------------------------------------------------------------------------

document = (b'{"name": "a doc", "items": [{"id": 7, "tags": ["x", "\\u00e9"], "score": 1.5, "flags": {"on": true}},'
            b' {"id": -2, "tags": [], "score": null, "flags": {}}], "note": null, "n": 3}')
requests = [
    b"GET /ping HTTP/1.1\r\nHost: fuzz\r\n\r\n",
    b"GET /q/one/two/three?x=1&y=%41 HTTP/1.1\r\nHost: fuzz\r\nX-H: head\r\nCookie: c=1; d=2\r\n\r\n",
    b"POST /echo HTTP/1.1\r\nHost: fuzz\r\nContent-Length: 11\r\n\r\nhello there",
    b"POST /echo HTTP/1.1\r\nHost: fuzz\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n there\r\n0\r\n\r\n",
    b"POST /form HTTP/1.1\r\nHost: fuzz\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: 11\r\n\r\na=1&b=%41+b",
    b"POST /json HTTP/1.1\r\nHost: fuzz\r\nContent-Type: application/json\r\nContent-Length: %d\r\n\r\n%s" % (len(document), document),
    b"HEAD /ping HTTP/1.1\r\nHost: fuzz\r\nConnection: close\r\n\r\n",
    b"GET /nowhere HTTP/1.0\r\n\r\n",
]


def throw(make, name, path=None):
    """Clients that each send what `make` gives them, for the time there is."""
    until = time.time() + SECONDS
    sent = [0]

    def client(seed):
        mine = random.Random(seed)
        while time.time() < until:
            if IN_TURN and path:
                ask(path, timeout=30)
                sent[0] += 1
                continue
            try:
                connection = socket.create_connection(("127.0.0.1", PORT), timeout=3)
                for _ in range(mine.choice([1, 1, 2, 5])):
                    connection.sendall(make(mine))
                    sent[0] += 1
                if mine.random() < 0.7:
                    connection.settimeout(mine.choice([0.01, 0.2, 2]))
                    try:
                        while connection.recv(65536):
                            pass
                    except OSError:
                        pass
                connection.close()
            except OSError:
                time.sleep(0.01)

    clients = [threading.Thread(target=client, args=(rng.random(),), daemon=True) for _ in range(8)]
    for c in clients:
        c.start()
    for c in clients:
        c.join()
    # Whatever was thrown, the server goes on, with the workers it had.
    problems = []
    status = ask("/ping", timeout=PATIENCE)
    if status != 200:
        problems.append(f"a plain request afterwards was answered {status}")
    if target.poll() is not None:
        problems.append(f"the server ended, with status {target.returncode}")
    now = children(target.pid)
    if now != workers:
        problems.append(f"its workers were {workers} and are {now}: one fell over and was replaced")
    written = open(errors, errors="replace").read()
    for sign in ["Sanitizer", "panic:", "internal error", "KOO_CHECK"]:
        if sign in written:
            problems.append(f"it wrote `{sign}`")
    print(("FAIL" if problems else "pass") + f"  {name}: {sent[0]} thrown")
    for problem in problems:
        print("        " + problem)
    if problems:
        failures.append(name)


def json_body(mine):
    body = spoil(document, mine) if mine.random() < 0.9 else b"[" * mine.choice([10, 1000, 100000])
    return b"POST /json HTTP/1.1\r\nHost: fuzz\r\nContent-Length: %d\r\n\r\n%s" % (len(body), body)


throw(lambda mine: spoil(mine.choice(requests), mine), "requests that are nearly requests")
throw(json_body, "bodies that are nearly JSON")
spoiling.set()
for path, name in [("/fetch", "answers of another server that are nearly HTTP"),
                   ("/pg", "answers of a database that are nearly PostgreSQL's"),
                   ("/redis", "answers of a cache that are nearly a Redis's")]:
    throw(lambda mine, path=path: b"GET %s HTTP/1.1\r\nHost: fuzz\r\n\r\n" % path.encode(), name, path)
spoiling.clear()

target.send_signal(signal.SIGTERM)
try:
    status = target.wait(timeout=40)
    if status != 0:
        failures.append("stop")
        print(f"FAIL  told to stop, the server ended with status {status}")
    else:
        print("pass  told to stop, the server ended with status 0")
except subprocess.TimeoutExpired:
    target.kill()
    failures.append("stop")
    print("FAIL  told to stop, the server did not end in forty seconds")

if failures:
    print(f"\n{len(failures)} failed with seed {SEED}; what the server wrote is in {errors}, and ends:")
    for line in open(errors, errors="replace").read().splitlines()[-40:]:
        print("    " + line[:300])
    sys.exit(1)
shutil.rmtree(build, ignore_errors=True)
print("\nall passed")
