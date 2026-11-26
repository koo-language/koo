#!/bin/zsh
# usage: workers.sh <docker run options...>   counts srv-a processes in a container
H=KOO_ROOT/target/hunt-c
name=koo-hunt-c-w
docker --context colima run -d --rm --name $name "$@" -v $H:/x:ro alpine /x/n/srv-a >/dev/null || exit 1
sleep 1.5
n=$(docker --context colima exec $name sh -c 'c=0; for p in /proc/[0-9]*; do [ "$(cat $p/comm 2>/dev/null)" = "srv-a" ] && c=$((c+1)); done; echo $c; cat /sys/fs/cgroup/cpu.max 2>/dev/null; nproc')
echo "[$*] processes (1 watcher + workers) / cpu.max / nproc: $(echo $n | tr '\n' ' ')"
docker --context colima kill $name >/dev/null
sleep 0.7
