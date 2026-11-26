# Under a limit on processes (L): all answering threads but one are kept by slow actors, quick
# asks keep the last one taking messages, and a worker is killed every quarter of a second.
# Every worker that ends should be replaced.
L=${L:-12}
st() { echo "sup threads=$(ls /proc/1/task | wc -l) workers=[$(pgrep -P 1 -x s | tr '\n' ' ')]"; }
nw() { pgrep -P 1 -x s | wc -l; }
for i in $(seq 1 50); do curl -s -m 1 -o /dev/null http://127.0.0.1:8080/ping && break; sleep 0.1; done
echo "at the start: $(st)"
busy=0
keep_busy() { # as many slow actors as leave one answering thread free
  want=$((L - $(nw) - 2))
  while [ $busy -lt $want ]; do
    curl -s -m 600 -o /dev/null http://127.0.0.1:8080/nap/$busy/600000 &
    busy=$((busy + 1)); sleep 0.15
  done
}
keep_busy; echo "slow actors busy: $(st)"
( while true; do ab -r -q -c ${C:-16} -t 5 http://127.0.0.1:8080/next/50 > /dev/null 2>&1; done ) &
sleep 1
for round in $(seq 1 ${ROUNDS:-600}); do
  before=$(nw)
  kill -9 $(pgrep -P 1 -x s | head -1)
  sleep 0.25
  after=$(nw)
  if [ $after -lt $before ]; then echo "round $round: a worker was not replaced: $(st)"; fi
  [ $after -eq 0 ] && break
  keep_busy
done
echo "rounds: $round"
pkill ab
sleep 3
echo "after the load: $(st)"
echo "ping: $(curl -s -m 5 -w '[%{http_code} %{time_total}]' http://127.0.0.1:8080/ping | tr '\n' ' ')"
sleep 15
echo "15 s later: $(st)"
echo "ping: $(curl -s -m 5 -w '[%{http_code} %{time_total}]' http://127.0.0.1:8080/ping | tr '\n' ' ')"
echo "listening: $(netstat -ltn 2>/dev/null | grep 8080)"
