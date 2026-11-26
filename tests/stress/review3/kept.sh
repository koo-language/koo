#!/bin/sh
# Counts the connections each worker keeps to the supervising process.
cd "$(dirname "$0")"
WORKERS=2 ./m > out.log 2> err.log &
SUP=$!
for i in $(seq 1 50); do curl -s -m 1 -o /dev/null http://127.0.0.1:39902/ping && break; sleep 0.1; done
W=$(pgrep -P $SUP | tr '\n' ' ')
count() { for w in $W; do printf "worker %s unix sockets: %s   " $w "$(lsof -a -p $w -U 2>/dev/null | grep -c unix)"; done; echo; }
echo "before any ask:"; count
for i in $(seq 1 24); do curl -s -m 5 -o /dev/null http://127.0.0.1:39902/nap/$i/400 & done
sleep 0.2; echo "24 asks in flight:"; count
wait $(jobs -p | grep -v "^$SUP$") 2>/dev/null
sleep 0.5; echo "after the 24 answers (connections given back):"; count
for k in 1 2 3 4; do curl -s -m 5 -o /dev/null http://127.0.0.1:39902/next/1; done
sleep 0.3; echo "after 4 more single asks, one at a time:"; count
kill $SUP; sleep 0.3
