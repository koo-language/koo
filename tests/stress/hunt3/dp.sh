for i in $(seq 1 50); do curl -s -m 1 -o /dev/null http://127.0.0.1:8080/ping && break; sleep 0.1; done
echo "workers: $(pgrep -P 1 -x dp | wc -l)  threads of the supervising process: $(ls /proc/1/task | wc -l)"
for p in /count /dig /count /ping; do
  echo "$p -> $(curl -s -m 5 -w ' [%{http_code}]' http://127.0.0.1:8080$p | tr '\n' ' ')"
done
