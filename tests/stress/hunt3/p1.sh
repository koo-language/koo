. /x/env.sh
cd /tmp
WEB_CONCURRENCY=2 start /x/s
echo sup=$SUP workers=$(workers)
python3 /x/p1.py $SUP
echo workers=$(workers)
kill -TERM $SUP; wait $SUP; echo exit=$?
echo --- err; sort /tmp/err.log | uniq -c | sort -rn | head -40
