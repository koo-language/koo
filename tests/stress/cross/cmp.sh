#!/bin/zsh
# usage: cmp.sh dir
H=KOO_ROOT/target/hunt-c
for f in KOO_ROOT/tests/run/*.koo; do
  n=$(basename $f .koo)
  grep '^//=' $f | sed -e 's#^//= ##' -e 's#^//=$##' > $H/out/exp.txt
  e=$(grep '^//! exit ' $f | sed 's#//! exit ##'); e=${e:-0}
  if ! diff -q $H/out/exp.txt $H/out/$1/$n.out >/dev/null; then echo "DIFF $1 $n"; diff $H/out/exp.txt $H/out/$1/$n.out | head -8; fi
  c=$(cat $H/out/$1/$n.code)
  [[ "$c" == "$e" ]] || echo "EXIT $1 $n got $c want $e: $(head -c 300 $H/out/$1/$n.err)"
  grep '^//! stderr ' $f | sed 's#//! stderr ##' | while IFS= read -r line; do grep -qF -- "$line" $H/out/$1/$n.err || echo "STDERR $1 $n lacks: $line // got: $(head -c 300 $H/out/$1/$n.err)"; done
done
