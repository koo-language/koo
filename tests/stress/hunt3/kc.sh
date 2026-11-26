. /x/env.sh
cd /tmp
start /x/kc
python3 /x/kc.py $ARGS
sleep 0.5
if kill -0 $SUP 2>/dev/null; then echo "the server is still running"; kill $SUP; else wait $SUP; echo "the server has ended: exit status $?"; fi
cat /tmp/err.log | head -5
