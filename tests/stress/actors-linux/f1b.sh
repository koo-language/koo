#!/bin/sh
# F1, without long messages: a worker started in place of one that died serves its
# requests one at a time once its places are taken, and does not come back to the
# handlers that wait. NOKILL=1 is the control; NOTELL=1 leaves out the one `send`, and
# with it the five threads that take the supervising process past 129.
cd /x/min; . ./env.sh
WORKERS=2 start ./m
NOTELL=$NOTELL python3 - <<'PY'
import L, os
r = L.run([("GET", "/nap/%d/200" % (1 + i % 200)) for i in range(600)], conc=600)
print("600 asks of 200 slow actors:", L.summary(r))
if not os.environ.get("NOTELL"): print("one tell:", L.get("/tell/0"))
PY
echo "supervisor now: $(stat $SUP)"
if [ "$NOKILL" != 1 ]; then
  for w in $(workers); do kill -9 $w; done
  sleep 1
  echo "both workers killed; workers now $(workers)"
fi
python3 - <<'PY'
import L, time
t = time.time()
r = L.run([("GET", "/nap/%d/3000" % (1 + i % 250)) for i in range(500)], conc=500, timeout=60)
print("500 asks of 250 actors that take 3 s:", L.summary(r), "(-5 is no answer in 60 s) in %.1fs" % (time.time() - t))
PY
echo "workers at the end: $(workers)"
echo "the server's standard error:"; sort /tmp/err.log | uniq -c | sort -rn | head -4
