#!/bin/sh
# F1: a worker started in place of one that died has room for fewer waiting handlers than
# it reckons with; with long messages the whole server's actors stop answering for ever.
# NOKILL=1 runs the same load without killing the workers first (the control).
cd /x/min; . ./env.sh
WORKERS=2 start ./m
echo "supervisor $SUP, workers $(workers)"
python3 - <<'PY'
import L
r = L.run([("GET", "/nap/%d/200" % (1 + i % 200)) for i in range(600)], conc=600)
print("600 asks of 200 slow actors:", L.summary(r))
print("one tell:", L.get("/tell/0"))
PY
echo "supervisor now: $(stat $SUP)"
if [ "$NOKILL" != 1 ]; then
  for w in $(workers); do kill -9 $w; done
  sleep 1
  echo "both workers killed; workers now $(workers)"
fi
python3 - <<'PY'
import L, time
body = b"y" * 300000
t = time.time()
r = L.run([("POST", "/echo/%d" % (1 + i % 200), body) for i in range(600)], conc=600, timeout=30)
print("600 asks with 300 kB each way:", L.summary(r), "(-5 is no answer in 30 s)", "right bodies:", sum(1 for s, b in r if s == 200 and b == body), "in %.1fs" % (time.time() - t))
time.sleep(10)
print("10 s later: /ping", L.get("/ping", 5), " /next/0", L.get("/next/0", 5), " /next/7", L.get("/next/7", 5))
PY
echo "supervisor: $(stat $SUP)"; for w in $(workers); do echo "worker: $(stat $w)"; done
