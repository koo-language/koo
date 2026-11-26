#!/bin/sh
python3 /x/fakepg_c.py 39600 > /dev/null &
/x/hsrv &
sleep 1
(wget -q -O - -T 100 'http://127.0.0.1:8090/pg?url=postgres://u:p@127.0.0.1:39603/x' ; echo "pg request ended: $?") &
sleep 1
for i in 1 2 3; do
  s=$(date +%s); wget -q -O - -T 8 http://127.0.0.1:8090/ping; echo " ping exit $? after $(( $(date +%s) - s )) s"
done
top -b -n 1 | grep -E "hsrv|CPU" | head -5
