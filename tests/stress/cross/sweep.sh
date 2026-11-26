#!/bin/sh
# run inside container: /x ro, /o rw
for d in "$@"; do
  mkdir -p /o/$d
  for p in /x/bin/$d/*; do
    n=$(basename $p)
    KOO_LEAKS=1 timeout 60 $p >/o/$d/$n.out 2>/o/$d/$n.err </dev/null
    echo $? >/o/$d/$n.code
  done
done
