#!/bin/sh
# A name server that answers with a stream of datagrams (wrong ids) for 8 s; the program that looks
# the name up has a tenth of a processor and gave the lookup 1000 ms. d5run.sh [cpus]
X=KOO_ROOT/target/hunt3
d() { docker --context colima "$@"; }
d rm -f koo-hunt3-d5-srv >/dev/null 2>&1
d run -d --name koo-hunt3-d5-srv -e PROCS=3 -v $X:/x koo-hunt3-tools sh /x/d5srv.sh >/dev/null
d run --rm --name koo-hunt3-d5-drv --cpus ${1:-0.1} -v $X:/x --network container:koo-hunt3-d5-srv koo-hunt3-tools timeout 150 sh /x/d5.sh
d rm -f koo-hunt3-d5-srv >/dev/null 2>&1
