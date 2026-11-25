//! Builds the support library `koo-rt` as a static library and leaves it where `main.rs`
//! embeds it, so that the `koo` binary carries everything `koo build` links into a program.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../koo-rt/src/lib.rs");
    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-changed={}", source.with_file_name("http.rs").display());
    println!("cargo:rerun-if-changed={}", source.with_file_name("actor.rs").display());
    println!(
        "cargo:rerun-if-changed={}",
        source.with_file_name("thread.rs").display()
    );
    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let target = env::var("TARGET").unwrap();
    let build = |target: &str| {
        Command::new(&rustc)
            .args([
                "--edition",
                "2024",
                "--crate-type",
                "staticlib",
                "--crate-name",
                "koo_rt",
            ])
            .args(["-C", "opt-level=3", "-C", "panic=abort", "-C", "codegen-units=1"])
            .args(["--target", target, "-o"])
            .arg(out.join("libkoo_rt.a"))
            .arg(&source)
            .status()
            .expect("rustc runs")
            .success()
    };
    // A released compiler is built for musl, so that the one binary runs on any Linux.
    // The programs it builds are another matter: each is linked by the clang of the
    // machine it is built on, against that machine's C library, whichever it is. The
    // support library names only functions that both C libraries have, and is built for
    // the GNU target where that is to be had, so that nothing of musl's comes with it.
    let other = target.replace("-musl", "-gnu");
    let built = (other != target && build(&other)) || build(&target);
    assert!(built, "the support library failed to compile");
}
