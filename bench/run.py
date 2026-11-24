#!/usr/bin/env python3
"""Builds each benchmark in Koo and in Rust, checks that they print the same thing, and
times them: the best of several runs of each.

    python3 bench/run.py [--runs N] [name ...]

Needs `cargo`, `rustc` and `clang`. Rust is built with `-C opt-level=3 -C codegen-units=1`,
and a second time with `-C overflow-checks=on`, because Koo checks integer overflow in
every build (Section 4.2 of SPEC.md) and Rust's release builds do not."""

import os
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BENCH = os.path.join(ROOT, "bench")


def run(args, **kwargs):
    return subprocess.run(args, check=True, capture_output=True, text=True, **kwargs)


def timed(args, runs):
    best, output = None, None
    for _ in range(runs):
        start = time.perf_counter()
        output = run(args).stdout
        elapsed = time.perf_counter() - start
        best = elapsed if best is None else min(best, elapsed)
    return best, output


def main():
    args = sys.argv[1:]
    runs = 5
    if "--runs" in args:
        i = args.index("--runs")
        runs = int(args[i + 1])
        del args[i:i + 2]
    names = args or sorted(f[:-4] for f in os.listdir(BENCH) if f.endswith(".koo"))

    run(["cargo", "build", "--release", "-q", "-p", "koo"], cwd=ROOT)
    koo = os.path.join(ROOT, "target", "release", "koo")
    out = tempfile.mkdtemp(prefix="koo-bench-")

    print(f"best of {runs} runs, seconds\n")
    print(f"{'benchmark':<10} {'koo':>8} {'rust':>8} {'rust+checks':>12} {'koo/rust':>9} {'koo/rust+checks':>16}"
          f" {'build koo':>10} {'build rust':>11}")
    for name in names:
        source = os.path.join(BENCH, name)
        binaries = {k: os.path.join(out, f"{name}.{k}") for k in ("koo", "rust", "checked")}

        start = time.perf_counter()
        run([koo, "build", "--release", source + ".koo", "-o", binaries["koo"]])
        build_koo = time.perf_counter() - start
        start = time.perf_counter()
        rustc = ["rustc", "--edition", "2021", "-C", "opt-level=3", "-C", "codegen-units=1"]
        run(rustc + ["-o", binaries["rust"], source + ".rs"])
        build_rust = time.perf_counter() - start
        run(rustc + ["-C", "overflow-checks=on", "-o", binaries["checked"], source + ".rs"])

        results = {k: timed([path], runs) for k, path in binaries.items()}
        outputs = {k: output for k, (_, output) in results.items()}
        if len(set(outputs.values())) != 1:
            print(f"{name}: the programs disagree: {outputs}")
            sys.exit(1)
        t = {k: seconds for k, (seconds, _) in results.items()}
        print(f"{name:<10} {t['koo']:>8.3f} {t['rust']:>8.3f} {t['checked']:>12.3f} {t['koo'] / t['rust']:>9.2f}"
              f" {t['koo'] / t['checked']:>16.2f} {build_koo:>10.2f} {build_rust:>11.2f}")


if __name__ == "__main__":
    main()
