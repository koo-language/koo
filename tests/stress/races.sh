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
# Any report of a race is a failure.
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
    if grep -q "WARNING: ThreadSanitizer" "$out/said.txt"; then
        echo "FAIL  $program"
        grep -A16 "WARNING: ThreadSanitizer" "$out/said.txt" | head -40
        failed=$((failed + 1))
    else
        echo "pass  $program"
    fi
done
rm -rf "$out"
echo "$programs programs, $failed with races"
[ "$failed" = 0 ] && [ "$programs" -gt 0 ]
