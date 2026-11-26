import asyncio, sys, time, os, collections

PORT = int(os.environ.get("PORT", "8080"))

async def one(method, path, body=b"", timeout=60.0, port=None):
    """Returns (status, body) or (-1, reason)."""
    try:
        r, w = await asyncio.wait_for(asyncio.open_connection("127.0.0.1", port or PORT), timeout)
    except Exception as e:
        return (-1, ("connect:" + repr(e)).encode())
    try:
        head = f"{method} {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Length: {len(body)}\r\n\r\n".encode()
        w.write(head + body)
        await w.drain()
        data = await asyncio.wait_for(r.read(-1), timeout)
        if not data:
            return (-2, b"empty")
        try:
            h, _, b = data.partition(b"\r\n\r\n")
            status = int(h.split(b" ", 2)[1])
            cl = None
            for line in h.split(b"\r\n")[1:]:
                k, _, v = line.partition(b":")
                if k.lower() == b"content-length":
                    cl = int(v)
            if cl is not None and len(b) != cl:
                return (-3, b"short body %d of %d" % (len(b), cl))
            return (status, b)
        except Exception as e:
            return (-4, repr(e).encode() + data[:80])
    except asyncio.TimeoutError:
        return (-5, b"timeout")
    except Exception as e:
        return (-6, repr(e).encode())
    finally:
        try:
            w.close()
        except Exception:
            pass

async def many(reqs, conc=None, timeout=60.0):
    """reqs: list of (method, path, body). All concurrently, or at most conc at a time."""
    sem = asyncio.Semaphore(conc or len(reqs) or 1)
    out = [None] * len(reqs)
    async def go(i, rq):
        async with sem:
            out[i] = await one(rq[0], rq[1], rq[2] if len(rq) > 2 else b"", timeout)
    await asyncio.gather(*(go(i, rq) for i, rq in enumerate(reqs)))
    return out

def summary(results):
    c = collections.Counter(r[0] for r in results)
    return dict(sorted(c.items()))

def get(path, timeout=30.0):
    return asyncio.run(one("GET", path, b"", timeout))

def run(reqs, conc=None, timeout=60.0):
    return asyncio.run(many(reqs, conc, timeout))
