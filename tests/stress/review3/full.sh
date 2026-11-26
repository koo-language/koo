#!/bin/sh
cd "$(dirname "$0")"
ulimit -n 8192
WORKERS=2 ./m > out.log 2> err.log &
SUP=$!
for i in $(seq 1 50); do curl -s -m 1 -o /dev/null http://127.0.0.1:39902/ping && break; sleep 0.1; done
curl -s -m 2 http://127.0.0.1:39902/next/1 >/dev/null
SOCK=$(ls ${TMPDIR:-/tmp}/koo-$SUP-*.sock 2>/dev/null | head -1)
echo "socket: $(basename "$SOCK")"
python3 full.py "$SOCK" $SUP
kill $SUP; sleep 0.3
