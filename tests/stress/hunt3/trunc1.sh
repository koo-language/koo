. /x/env.sh
cd /tmp
start /x/trunc
SUP=$SUP python3 /x/trunc.py $ARGS
case "$ARGS" in none*) kill $SUP;; esac
wait $SUP; echo "server exit=$? out=$(cat /tmp/out.log) err=$(sort /tmp/err.log | uniq -c | head -3)"
