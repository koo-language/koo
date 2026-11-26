#!/bin/sh
cd /x/min
python3 - <<'PY'
import L, os, time
r = L.run([("GET", "/nap/%d/200" % (1 + i % 250)) for i in range(600)], conc=600, timeout=40)
print("600 asks of 250 slow actors:", L.summary(r), " threads of the server:", len(os.listdir("/proc/1/task")), flush=True)
print("one tell:", L.get("/tell/3"), flush=True)
time.sleep(1)
print("1 s later: /ping", L.get("/ping", 5))
PY
