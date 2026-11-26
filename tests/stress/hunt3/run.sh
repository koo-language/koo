#!/bin/sh
# run.sh NAME SCRIPT [docker run args...]: runs /x/SCRIPT in container koo-hunt3-NAME
X=KOO_ROOT/target/hunt3
name=$1; script=$2; shift 2
exec docker --context colima run --rm --name koo-hunt3-$name -v $X:/x "$@" koo-hunt3-tools timeout ${T:-280} sh /x/$script
