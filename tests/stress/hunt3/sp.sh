. /x/env.sh
cd /tmp
WEB_CONCURRENCY=2 start /x/s
python3 /x/sp.py $SUP
kill $SUP; wait $SUP
