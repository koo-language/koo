#!/bin/sh
# F4: the same with few files: the server may open 256, and has 4 workers. The load has
# all the files it wants.
cd /x/min; . ./env.sh
WORKERS=4 start sh -c "ulimit -n 256; exec ./m"
echo "supervisor $SUP, workers $(workers); $(grep 'open files' /proc/$SUP/limits)"
python3 - <<'PY'
import L, time
for k in range(2):
    t = time.time()
    r = L.run([("GET", "/nap/%d/100" % (1 + i % 250)) for i in range(600)], conc=600, timeout=25)
    print("600 asks that take 100 ms:", L.summary(r), "(-5 is no answer in 25 s) in %.1fs" % (time.time() - t), flush=True)
time.sleep(30)
t = time.time()
r = L.run([("GET", "/next/0")] * 100, conc=1, timeout=5)
print("30 s later, at rest: 100 asks one at a time:", L.summary(r), "in %.1fs" % (time.time() - t), flush=True)
t = time.time()
r = L.run([("GET", "/next/0")] * 300, conc=100, timeout=10)
print("300 asks, 100 at a time:", L.summary(r), "(-5 is no answer in 10 s)", flush=True)
PY
echo "supervisor: $(stat $SUP)"; for w in $(workers); do echo "worker: $(stat $w)"; done
sort /tmp/err.log | uniq -c | sort -rn | head -3
