#!/bin/sh
# What actors cost in Koo and in Erlang: an ask, an idle actor, a message, and work
# done by many at once.
#
#     sh bench/actors/run.sh
#
# Needs a built `koo` and Erlang (`erl`, `erlc`). Each program is in both languages,
# with a `gen_server` for each actor in Erlang:
#
#     ask       ten million asks of one actor, one after the other
#     many      a million actors started, each asked twice, all of them kept
#     ring      a thousand actors in a ring pass a message round twenty million times
#     fan       sixteen actors tell sixty-four others 12.8 million things at once
#     parallel  sixteen actors each count the primes in a range of a million numbers
#
# The time is from start to end as a clock sees it, the best of three runs, and the
# memory is the most the program held.
set -e
cd "$(dirname "$0")"
koo=../../target/release/koo
[ -x "$koo" ] || (cd ../.. && cargo build --release)
dir=$(mktemp -d)
programs="ask many ring fan parallel"
for name in $programs; do
    "$koo" build --release "$name.koo" -o "$dir/$name"
    erlc -o "$dir" "$name.erl"
done

# Seconds and megabytes of one run of a command.
once() {
    /usr/bin/time -l "$@" > "$dir/out" 2> "$dir/time" || /usr/bin/time -v "$@" > "$dir/out" 2> "$dir/time"
    awk '/ real / {seconds = $1}
         /Elapsed/ {n = split($NF, part, ":"); seconds = part[n]; if (n > 1) seconds += 60 * part[n - 1]}
         /maximum resident set size/ {memory = $1 / 1048576}
         /Maximum resident set size/ {memory = $NF / 1024}
         END {printf "%.2f %d\n", seconds, memory}' "$dir/time"
}

measure() {
    label=$1; shift
    best=""
    for run in 1 2 3; do
        result=$(once "$@")
        seconds=${result% *}
        if [ -z "$best" ] || [ "$(echo "$seconds < ${best% *}" | bc)" = 1 ]; then best=$result; fi
    done
    printf '%-18s %6s s  %6s MB   %s\n' "$label" "${best% *}" "${best#* }" "$(head -1 "$dir/out")"
}

for name in $programs; do
    measure "koo $name" "$dir/$name"
    measure "erlang $name" erl +P 2000000 -noshell -pa "$dir" -s "$name" main
done
rm -r "$dir"
