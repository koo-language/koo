for i in $(seq 1 50); do curl -s -m 1 -o /dev/null http://127.0.0.1:8080/ping && break; sleep 0.1; done
python3 /x/bad2.py $ARGS
echo "   sup: threads=$(ls /proc/1/task | wc -l) fds=$(ls /proc/1/fd | wc -l); workers fds: $(for w in $(pgrep -P 1 -x s); do ls /proc/$w/fd | wc -l; done | tr '\n' ' ')"
