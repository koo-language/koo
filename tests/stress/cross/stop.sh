#!/bin/zsh
# usage: stop.sh <binary> <signal> <docker run options...>
H=KOO_ROOT/target/hunt-c
name=koo-hunt-c-s
bin=$1; sig=$2; shift 2
docker --context colima run -d --rm --name $name "$@" -v $H:/x:ro alpine /x/n/$bin >/dev/null || exit 1
sleep 1.5
docker --context colima logs -f $name > $H/out/stop.log 2>&1 &
docker --context colima wait $name > $H/out/stop.code 2>&1 &
sleep 0.3
s=$(date +%s)
if [[ $sig == TERM ]]; then docker --context colima stop $name >/dev/null; else docker --context colima kill -s $sig $name >/dev/null; fi
wait
e=$(date +%s)
echo "[$bin $sig $*] took $((e-s))s exit=$(cat $H/out/stop.code) log: $(cat $H/out/stop.log | tr '\n' '|')"
sleep 0.5
