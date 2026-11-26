#!/bin/sh
# run.sh <script in this directory> [arguments for docker run...]
# Runs a script in a container of its own, with this scratch directory at /x.
X=KOO_ROOT/target/hunt2-b
script=$1; shift
docker --context colima image inspect koo-hunt2-b-tools >/dev/null 2>&1 || docker --context colima build -q -t koo-hunt2-b-tools $X/min >/dev/null
exec docker --context colima run --rm --name koo-hunt2-b-${script%.sh} -v $X:/x "$@" koo-hunt2-b-tools timeout 280 sh /x/min/$script
