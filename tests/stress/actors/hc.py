# Minimal asyncio HTTP/1.1 client: one connection per request, hard timeout.
import asyncio, time, collections, sys

async def req(port, method, path, body=b"", timeout=30.0, host="127.0.0.1"):
    """Returns (status, body_bytes, seconds). status: int, or 'timeout'/'error:..'"""
    t0 = time.monotonic()
    async def go():
        r, w = await asyncio.open_connection(host, port)
        try:
            head = f"{method} {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Length: {len(body)}\r\n\r\n".encode()
            w.write(head + body)
            await w.drain()
            data = await r.read()
            return data
        finally:
            w.close()
    try:
        data = await asyncio.wait_for(go(), timeout)
    except asyncio.TimeoutError:
        return ("timeout", b"", time.monotonic() - t0)
    except Exception as e:
        return (f"error:{type(e).__name__}", b"", time.monotonic() - t0)
    if not data:
        return ("empty", b"", time.monotonic() - t0)
    try:
        headend = data.index(b"\r\n\r\n")
        status = int(data[:headend].split(b" ", 2)[1])
        heads = data[:headend].decode("latin1").split("\r\n")[1:]
        clen = None
        for h in heads:
            k, _, v = h.partition(":")
            if k.strip().lower() == "content-length":
                clen = int(v.strip())
        payload = data[headend + 4:]
        if clen is not None and len(payload) != clen:
            return (f"short:{status}:{len(payload)}/{clen}", payload, time.monotonic() - t0)
        return (status, payload, time.monotonic() - t0)
    except Exception as e:
        return (f"bad:{type(e).__name__}", data, time.monotonic() - t0)

async def many(port, specs, concurrency=None, timeout=30.0):
    """specs: list of (method, path, body). Returns list of results in order."""
    sem = asyncio.Semaphore(concurrency or len(specs) or 1)
    async def one(s):
        async with sem:
            return await req(port, s[0], s[1], s[2] if len(s) > 2 else b"", timeout)
    return await asyncio.gather(*[one(s) for s in specs])

def tally(results):
    c = collections.Counter(r[0] for r in results)
    return dict(c)

def pct(results, p):
    xs = sorted(r[2] for r in results)
    return xs[min(len(xs) - 1, int(len(xs) * p))] if xs else 0
