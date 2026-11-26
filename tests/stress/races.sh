#!/bin/sh
# Looks for races inside the support library, on Linux.
#
#     sh tests/stress/races.sh <path to koo>
#
# A program built with `koo build --sanitize=thread` is checked by ThreadSanitizer, but
# the support library it is linked with is not: the sanitizer sees what the program's
# own code reads and writes, and nothing of the tables the library keeps for it. Here
# the library is built again so that the sanitizer sees inside it, which takes a
# nightly Rust compiler (`rustup toolchain install nightly --profile minimal`), and
# every test program that has actors or threads is built with that library and run.
# Then the server of `check.py` is. Any report of a race is a failure.
koo=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
cd "$(dirname "$0")/../.." || exit 1
out=$(mktemp -d)
host=$(rustc +nightly -vV | sed -n 's/^host: //p')
rustc +nightly --edition 2024 --crate-type staticlib --crate-name koo_rt \
    -C opt-level=1 -C panic=abort -C codegen-units=1 \
    -Zsanitizer=thread -Cunsafe-allow-abi-mismatch=sanitizer \
    --target "$host" -o "$out/libkoo_rt.a" compiler/koo-rt/src/lib.rs || exit 1
programs=0
failed=0
for program in $(grep -l "^actor \|parallel\|spawn" tests/run/*.koo); do
    programs=$((programs + 1))
    if ! KOO_RUNTIME="$out/libkoo_rt.a" "$koo" build --sanitize=thread "$program" -o "$out/program" > "$out/built.txt" 2>&1; then
        echo "FAIL  $program was not built"
        head -5 "$out/built.txt"
        failed=$((failed + 1))
        continue
    fi
    timeout 300 "$out/program" > "$out/said.txt" 2>&1 < /dev/null
    # A race, or a sanitizer that gave up, which would otherwise pass for no race found.
    if grep -q "ThreadSanitizer" "$out/said.txt"; then
        echo "FAIL  $program"
        grep -A16 "ThreadSanitizer" "$out/said.txt" | head -40
        failed=$((failed + 1))
    else
        echo "pass  $program"
    fi
done
echo "$programs programs, $failed with races"
# And a server under load (`check.py`), whose process that keeps the actors answers on
# many threads at once. The sanitizer makes it many times slower, so what the check
# says of times is of no account here, nor is a stack that runs out tried: what counts
# is whether the sanitizer reports a race.
if KOO_RUNTIME="$out/libkoo_rt.a" "$koo" build --sanitize=thread tests/stress/check.koo -o "$out/check" > "$out/built.txt" 2>&1; then
    (ulimit -n 16384 2>/dev/null
     CHECK_NO_DIVE=1 TSAN_OPTIONS="die_after_fork=0 halt_on_error=0 report_signal_unsafe=0" \
        timeout 1500 python3 tests/stress/check.py --binary "$out/check" --stderr "$out/server.txt" > "$out/check.txt" 2>&1)
    reports=$(grep -c "ThreadSanitizer: \|ThreadSanitizer$" "$out/server.txt")
    passed=$(grep -c '^pass' "$out/check.txt")
    echo "a server under load: $passed checks passed of $(grep -c '^pass\|^FAIL' "$out/check.txt") made, $reports reports from the sanitizer"
    if [ "$reports" != 0 ]; then
        grep -A16 "ThreadSanitizer" "$out/server.txt" | head -60
        failed=$((failed + 1))
    fi
    # A server that did not start, or fell over at once, was not looked at: most of the
    # checks that are not of times pass under the sanitizer, and are to have passed.
    if [ "$passed" -lt 8 ]; then
        echo "FAIL  the server under load was not checked: too few of its checks passed"
        cat "$out/check.txt" | head -30
        failed=$((failed + 1))
    fi
    pkill -9 -x check 2>/dev/null
else
    echo "FAIL  tests/stress/check.koo was not built"
    head -5 "$out/built.txt"
    failed=$((failed + 1))
fi
rm -rf "$out"
[ "$failed" = 0 ] && [ "$programs" -gt 0 ]
