cd /x
echo "somaxconn $(cat /proc/sys/net/core/somaxconn)"
WORKERS=2 ./t-linux > /tmp/out.log 2> /tmp/err.log &
SUP=$!
for i in $(seq 1 100); do curl -s -m 1 -o /dev/null http://127.0.0.1:39904/ping && break; sleep 0.1; done
curl -s -m 5 http://127.0.0.1:39904/tell/200; sleep 0.3
echo "told once with nothing else going on; actor 200 was told: $(curl -s -m 5 http://127.0.0.1:39904/seen/200)"
for i in $(seq 1 140); do curl -s -m 40 -o /dev/null http://127.0.0.1:39904/nap/$i/9000 & done
sleep 2
echo "supervisor threads while 140 slow asks are answered: $(ls /proc/$SUP/task | wc -l)"
python3 fill.py $SUP 4 &
sleep 1.5
for k in 1 2 3; do curl -s -m 5 http://127.0.0.1:39904/tell/200 | tr '\n' ' '; done; echo
sleep 12
echo "after everything settled, actor 200 was told: $(curl -s -m 5 http://127.0.0.1:39904/seen/200) (4 were sent)"
echo "--- stderr of the server:"; sort /tmp/err.log | uniq -c | head
kill $SUP
