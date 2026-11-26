#!/bin/sh
# F2: 128 actors asked at once and then 123 or more told something slow at once: the
# process that watches ends with "koo: too many threads", and the server with it.
cd /x/min; . ./env.sh
WORKERS=2 start ./m
echo "supervisor $SUP, workers $(workers)"
python3 - <<'PY'
import L, time
r = L.run([("GET", "/nap/%d/200" % (1 + i % 200)) for i in range(600)], conc=600)
print("600 asks of 200 slow actors:", L.summary(r))
PY
echo "supervisor now: $(stat $SUP)"
python3 - <<'PY'
import L, time
r = L.run([("GET", "/slowtell/%d/3000" % (1 + i % 200)) for i in range(200)], conc=200)
print("200 actors each told something that takes 3 s:", L.summary(r))
time.sleep(4)
print("4 s later: /ping", L.get("/ping", 5), " /next/0", L.get("/next/0", 5))
PY
wait $SUP; echo "the server ended with status $?; its standard error:"; cat /tmp/err.log
