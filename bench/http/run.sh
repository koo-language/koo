#!/bin/sh
# Requests a second for "Hello, world!", from Koo, Rust (axum on tokio) and Node, each
# on one core, measured by the same load generator on the same machine.
#
#     sh bench/http/run.sh
#
# Needs `cargo` (which fetches axum the first time), `node`, and a built `koo`.
set -e
cd "$(dirname "$0")"
koo=../../target/release/koo
[ -x "$koo" ] || (cd ../.. && cargo build --release)
dir=$(mktemp -d)
"$koo" build --release server.koo -o "$dir/koo_server"
rustc -O load.rs -o "$dir/load"
(cd axum && cargo build --release --quiet)

measure() {
    name=$1; port=$2; shift 2
    "$@" > /dev/null 2>&1 &
    server=$!
    sleep 1
    "$dir/load" "$port" / 16 2 > /dev/null
    printf '%-22s %8s requests a second\n' "$name" "$("$dir/load" "$port" / 16 5)"
    kill "$server" 2> /dev/null
    wait "$server" 2> /dev/null || true
}

measure "koo, 1 worker" 18090 env WORKERS=1 "$dir/koo_server"
measure "rust axum, 1 thread" 18092 env WORKERS=1 axum/target/release/bench_rs
measure "node" 18091 node server.js
rm -r "$dir"
