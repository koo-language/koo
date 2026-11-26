import asyncio, sys, hc, time, random, collections
port = int(sys.argv[1]); secs = float(sys.argv[2])
stop = False; tally = collections.Counter(); wrong = 0; counts = []
async def aborter(k):
    rnd = random.Random(k)
    while not stop:
        try:
            r, w = await asyncio.open_connection("127.0.0.1", port)
            if rnd.random() < 0.5:
                w.write(f"GET /slow/{rnd.randrange(16)}/{rnd.randrange(1, 30)} HTTP/1.1\r\nHost: x\r\n\r\n".encode())
            else:
                body = b"z" * rnd.randrange(1, 6000)
                w.write(f"POST /echo/{rnd.randrange(16)} HTTP/1.1\r\nHost: x\r\nContent-Length: {len(body)}\r\n\r\n".encode() + body[:rnd.randrange(len(body) + 1)])
            await w.drain()
            await asyncio.sleep(rnd.random() * 0.01)
            w.transport.abort() if rnd.random() < 0.5 else w.close()
            tally["aborted"] += 1
        except Exception as e:
            tally["abort-err:" + type(e).__name__] += 1
async def verifier(k):
    global wrong
    rnd = random.Random(1000 + k)
    while not stop:
        if rnd.random() < 0.5:
            body = (b"%06d-%03d-" % (rnd.randrange(10**6), k)) * rnd.randrange(1, 500)
            r = await hc.req(port, "POST", f"/echo/{rnd.randrange(16)}", body, timeout=20)
            if r[0] == 200 and r[1] != body: wrong += 1
        else:
            r = await hc.req(port, "GET", "/count", timeout=20)
            if r[0] == 200: counts.append(int(r[1]))
        tally[r[0]] += 1
async def main():
    global stop
    tasks = [asyncio.create_task(aborter(k)) for k in range(30)] + [asyncio.create_task(verifier(k)) for k in range(30)]
    await asyncio.sleep(secs); stop = True
    await asyncio.gather(*tasks)
    print(dict(tally), "wrong echoes:", wrong, "duplicate counts:", len(counts) - len(set(counts)))
    total = int((await hc.req(port, "GET", "/total", timeout=10))[1])
    print("count successes", len(counts), "actor total", total, "equal:", total == len(counts))
    res = await hc.many(port, [("GET", "/slow/1/5")] * 60, timeout=15)
    print("60 concurrent /slow afterwards:", hc.tally(res))
asyncio.run(main())
