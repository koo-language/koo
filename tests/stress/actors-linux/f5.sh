#!/bin/sh
# F5 (run on the Mac, not in a container): with few processes allowed (--pids-limit), the
# threads that answer for the actors take all there are, and the first `send` ends the server.
# The server is PID 1 of one container; the load is in another that shares its network.
X=KOO_ROOT/target/hunt2-b
D="docker --context colima"
$D image inspect koo-hunt2-b-tools >/dev/null 2>&1 || $D build -q -t koo-hunt2-b-tools $X/min >/dev/null
$D rm -f koo-hunt2-b-f5 koo-hunt2-b-f5-load >/dev/null 2>&1
$D run -d --name koo-hunt2-b-f5 --pids-limit ${PIDS:-64} -e WORKERS=2 -v $X:/x alpine /x/min/m >/dev/null
sleep 1
$D run --rm --name koo-hunt2-b-f5-load --network container:koo-hunt2-b-f5 --pid container:koo-hunt2-b-f5 -v $X:/x koo-hunt2-b-tools sh /x/min/f5load.sh
echo "the server's container: $($D inspect -f 'status={{.State.Status}} exit={{.State.ExitCode}}' koo-hunt2-b-f5); its standard error:"
$D logs koo-hunt2-b-f5 2>&1 >/dev/null | tail -3
$D rm -f koo-hunt2-b-f5 >/dev/null 2>&1
