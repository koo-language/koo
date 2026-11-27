#!/bin/sh
# Throws what is nearly right at everything the support library reads from outside,
# with the library built so that AddressSanitizer sees inside it: a read or a write
# beyond what was allocated, or of what was freed, is then a report, where without the
# sanitizer it may be nothing that anyone sees. Linux, and a nightly Rust compiler
# (`rustup toolchain install nightly --profile minimal`), as for `races.sh`.
#
#     sh tests/stress/fuzz.sh <path to koo> [seconds for each reader] [seed]
koo=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
cd "$(dirname "$0")/../.." || exit 1
out=$(mktemp -d)
host=$(rustc +nightly -vV | sed -n 's/^host: //p')
rustc +nightly --edition 2024 --crate-type staticlib --crate-name koo_rt \
    -C opt-level=1 -C panic=abort -C codegen-units=1 \
    -Zsanitizer=address -Cunsafe-allow-abi-mismatch=sanitizer \
    --target "$host" -o "$out/libkoo_rt.a" compiler/koo-rt/src/lib.rs || exit 1
if ! KOO_RUNTIME="$out/libkoo_rt.a" "$koo" build --sanitize tests/stress/fuzz -o "$out/server" > "$out/built.txt" 2>&1; then
    echo "FAIL  the server was not built"
    head -20 "$out/built.txt"
    exit 1
fi
# A server built so answers one request at a time in each of its workers, where one
# built as usual answers many: its handlers are given three tenths of a second each, and its clients
# wait their turn, so that what is thrown is read and not left waiting.
DEADLINE_MS=300 \
python3 -u tests/stress/fuzz.py "$koo" --binary "$out/server" --patience 120 --in-turn --seconds "${2:-15}" ${3:+--seed "$3"}
status=$?
rm -rf "$out"
exit $status
