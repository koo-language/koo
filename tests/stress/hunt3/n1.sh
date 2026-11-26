for i in $(seq 1 50); do curl -s -m 1 -o /dev/null http://127.0.0.1:8080/ping && break; sleep 0.1; done
echo "workers: $(pgrep -P 1 -x s | wc -l)"
python3 /x/n1.py $ARGS
