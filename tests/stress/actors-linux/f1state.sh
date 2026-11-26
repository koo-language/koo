#!/bin/sh
# F1 with long messages, looked at twice, 70 s apart: what each process is waiting in.
# Needs strace: run in an image that has it, with --cap-add SYS_PTRACE.
cd /x/min; . ./env.sh
WORKERS=2 start ./m
python3 - <<'PY'
import L
r = L.run([("GET", "/nap/%d/200" % (1 + i % 200)) for i in range(600)], conc=600)
print("600 asks of 200 slow actors:", L.summary(r), " one tell:", L.get("/tell/0"))
PY
for w in $(workers); do kill -9 $w; done; sleep 1
echo "both workers killed; workers now $(workers)"
python3 - <<'PY'
import L, time
body = b"y" * 300000
r = L.run([("POST", "/echo/%d" % (1 + i % 200), body) for i in range(600)], conc=600, timeout=30)
print("600 asks with 300 kB each way:", L.summary(r))
PY
look() {
  echo "--- $1: workers $(workers)"
  for w in $(workers); do echo "worker $w ($(ls /proc/$w/fd | wc -l) files) is in: $(timeout 2 strace -p $w 2>&1 | sed -n 2p | cut -c1-90)"; done
  echo "threads of the supervising process that are not asleep on a lock (futex) or a clock:"
  timeout 3 strace -f -p $SUP 2>&1 | grep -E '^\[pid +[0-9]+\] (read|write|recv|send|ppoll|poll|accept)' | cut -c1-70 | sort | uniq -c | head -6
  python3 -c "
import L
print('/next/0', L.get('/next/0', 5), ' /next/7', L.get('/next/7', 5))"
}
look "now"
sleep 70
look "70 s later"
sort /tmp/err.log | uniq -c | sort -rn | head -3
