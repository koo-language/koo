//! Builds the support library `koo-rt` as a static library and leaves it where `main.rs`
//! embeds it, so that the `koo` binary carries everything `koo build` links into a program.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../koo-rt/src/lib.rs");
    println!("cargo:rerun-if-changed={}", source.display());
    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let target = env::var("TARGET").unwrap();
    let status = Command::new(rustc)
        .args([
            "--edition",
            "2024",
            "--crate-type",
            "staticlib",
            "--crate-name",
            "koo_rt",
        ])
        .args(["-C", "opt-level=3", "-C", "panic=abort", "-C", "codegen-units=1"])
        .args(["--target", &target, "-o"])
        .arg(out.join("libkoo_rt.a"))
        .arg(&source)
        .status()
        .expect("rustc runs");
    assert!(status.success(), "the support library failed to compile");
}
