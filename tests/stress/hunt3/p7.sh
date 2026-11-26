for i in $(seq 1 50); do curl -s -m 1 -o /dev/null http://127.0.0.1:8080/ping && break; sleep 0.1; done
echo "workers: $(pgrep -P 1 -x s | wc -l)  sup threads: $(ls /proc/1/task | wc -l)"
for p in /next/1 /tell/1 /tolds/1 /slowtell/2/2000 /next/3 /tolds/2 /toldboom/3 /next/3 /boom/4 /next/4 /deep/5 /next/5 /tolddeep/6 /next/6 /say/7/5 /life/3 /keep /ping; do
  echo "$p -> $(curl -s -m 20 -w ' [%{http_code} %{time_total}s]' http://127.0.0.1:8080$p | tr '\n' ' ')"
  [ -d /proc/1 ] || break
done
echo "workers: $(pgrep -P 1 -x s | wc -l)  sup threads: $(ls /proc/1/task | wc -l)"
