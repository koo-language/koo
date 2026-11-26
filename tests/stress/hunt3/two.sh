#!/bin/sh
# two.sh NAME SCRIPT "<docker args for the server container>" [docker args for the driver...]
# The server is the only thing in its container (so its limits are its own); the script runs in a
# sibling container that shares its network and its processes. In the script the server is PID 1.
X=KOO_ROOT/target/hunt3
name=$1; script=$2; srvargs=$3; shift 3
d() { docker --context colima "$@"; }
d rm -f koo-hunt3-$name-srv >/dev/null 2>&1
eval d run -d --name koo-hunt3-$name-srv -v $X:/x $srvargs koo-hunt3-tools ${SRV:-/x/s} >/dev/null || exit 1
d run --rm --name koo-hunt3-$name-drv -v $X:/x --network container:koo-hunt3-$name-srv --pid container:koo-hunt3-$name-srv "$@" koo-hunt3-tools timeout ${T:-280} sh /x/$script
echo "--- server container: $(d inspect -f 'status={{.State.Status}} exit={{.State.ExitCode}} oom={{.State.OOMKilled}}' koo-hunt3-$name-srv)"
echo "--- server stderr (most frequent lines):"; d logs koo-hunt3-$name-srv 2>&1 >/dev/null | sort | uniq -c | sort -rn | head -${LOGN:-12}
d rm -f koo-hunt3-$name-srv >/dev/null 2>&1
