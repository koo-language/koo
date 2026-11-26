echo "nameserver 127.0.0.2" > /etc/resolv.conf
python3 /x/web.py > /dev/null 2>&1 &
QUIET=1 python3 /x/hd.py 127.0.0.2 byname > /tmp/dns.log 2>&1 &
sleep 0.7
for w in 2 1; do
  echo "=== $w workers"
  WEB_CONCURRENCY=$w /x/s > /tmp/out.log 2> /tmp/err.log &
  srv=$!
  sleep 0.5
  python3 /x/d6.py
  kill $srv; wait $srv
  sort /tmp/err.log | uniq -c | head -5
done
