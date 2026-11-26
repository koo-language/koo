#!/bin/zsh
# usage: run.sh <binary> <port> <workers> [env...]  -> starts server in background, prints pid
bin=$1; port=$2; workers=$3; shift 3
env PORT=$port WORKERS=$workers "$@" ./$bin > logs/$bin.$port.out 2> logs/$bin.$port.err &
echo $!
