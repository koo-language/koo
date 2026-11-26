import asyncio, sys, hc, time, os, signal, subprocess, random, collections
port = int(sys.argv[1]); sup = int(sys.argv[2]); secs = float(sys.argv[3]); every = float(sys.argv[4])
def workers():
    return [int(x) for x in subprocess.run(["pgrep", "-P", str(sup)], capture_output=True, text=True).stdout.split()]
stop = False
tally = collections.Counter(); answers = []
async def client(k):
    rnd = random.Random(k)
    while not stop:
        r = await hc.req(port, "GET", f"/work/{rnd.randrange(16)}/{rnd.randrange(6)}", timeout=20)
        tally[r[0]] += 1
        if r[0] == 200: answers.append(r[1].decode())
async def killer():
    while not stop:
        await asyncio.sleep(every)
        ws = workers()
        if ws:
            try: os.kill(random.choice(ws), signal.SIGKILL)
            except ProcessLookupError: pass
async def main():
    global stop
    tasks = [asyncio.create_task(client(k)) for k in range(40)]
    kt = asyncio.create_task(killer())
    await asyncio.sleep(secs); stop = True
    await kt; await asyncio.gather(*tasks)
    print(dict(tally), "duplicate answers:", len(answers) - len(set(answers)))
asyncio.run(main())
