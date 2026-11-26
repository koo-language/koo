for i in $(seq 1 50); do curl -s -m 1 -o /dev/null http://127.0.0.1:8080/ping && break; sleep 0.1; done
python3 /x/bad2.py $ARGS
echo "   just after: /next/1 -> $(curl -s -m 10 -w '[%{http_code} %{time_total}s]' http://127.0.0.1:8080/next/1 | tr '\n' ' ')"
t0=$(date +%s)
for i in $(seq 1 60); do
  r=$(curl -s -m 2 -o /dev/null -w '%{http_code} %{time_total}' http://127.0.0.1:8080/ping)
  case "$r" in 200*) echo "   the server answers a ping again $(( $(date +%s) - t0 )) s after the clients have gone ($r)"; break;; esac
  sleep 2
done
