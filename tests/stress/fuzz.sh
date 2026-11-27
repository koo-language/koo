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
mkdir "$out/package"
cp tests/stress/fuzz/main.koo packages/redis/redis.koo "$out/package/"
if ! KOO_RUNTIME="$out/libkoo_rt.a" "$koo" build --sanitize "$out/package" -o "$out/server" > "$out/built.txt" 2>&1; then
    echo "FAIL  the server was not built"
    head -20 "$out/built.txt"
    exit 1
fi
python3 tests/stress/fuzz.py "$koo" --binary "$out/server" --seconds "${2:-15}" ${3:+--seed "$3"}
status=$?
rm -rf "$out"
exit $status
