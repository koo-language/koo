//! Builds the support library `koo-rt` as a static library and leaves it where `main.rs`
//! embeds it, so that the `koo` binary carries everything `koo build` links into a program.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../koo-rt/src/lib.rs");
    println!("cargo:rerun-if-changed={}", source.display());
    for file in [
        "http.rs",
        "actor.rs",
        "digest.rs",
        "fiber.rs",
        "codec.rs",
        "client.rs",
        "log.rs",
        "net.rs",
        "decimal.rs",
        "decimals.rs",
        "pg.rs",
        "thread.rs",
        "tls.rs",
    ] {
        println!("cargo:rerun-if-changed={}", source.with_file_name(file).display());
    }
    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let target = env::var("TARGET").unwrap();
    let build = |target: &str, library: &Path| {
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
            .arg(library)
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
    let built =
        (other != target && build(&other, &out.join("libkoo_rt.a"))) || build(&target, &out.join("libkoo_rt.a"));
    assert!(built, "the support library failed to compile");

    // What `koo build --target` needs to build for a Linux machine from another kind of
    // machine: the support library compiled for it, and a C library to link in, which
    // is musl's as the Rust toolchain carries it for its own `-musl` targets. A compiler
    // built where that target is not installed (`rustup target add <triple>`) is built
    // without it, and says so when it is asked for.
    let sysroot = Command::new(&rustc).args(["--print", "sysroot"]).output().ok();
    let sysroot = sysroot.map(|output| PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()));
    if let Some(sysroot) = &sysroot {
        println!("cargo:rerun-if-changed={}", sysroot.join("lib/rustlib").display());
    }
    // Debugging information is most of these libraries and no use to a Koo program.
    let objcopy = sysroot.as_ref().and_then(|sysroot| {
        let host = env::var("HOST").ok()?;
        let tool = sysroot.join("lib/rustlib").join(host).join("bin/rust-objcopy");
        tool.exists().then_some(tool)
    });
    let slim = |library: &Path| {
        if let Some(objcopy) = &objcopy {
            let slimmed = library.with_extension("slim");
            let done = Command::new(objcopy)
                .arg("--strip-debug")
                .arg(library)
                .arg(&slimmed)
                .status();
            if done.is_ok_and(|status| status.success()) {
                let _ = std::fs::rename(&slimmed, library);
            } else {
                let _ = std::fs::remove_file(&slimmed);
            }
        }
    };
    const PARTS: [&str; 6] = ["crt1.o", "crti.o", "crtbegin.o", "crtend.o", "crtn.o", "libc.a"];
    for (name, triple) in [
        ("linux-arm64", "aarch64-unknown-linux-musl"),
        ("linux-x86-64", "x86_64-unknown-linux-musl"),
    ] {
        let dir = out.join(name);
        std::fs::create_dir_all(&dir).expect("the output directory can be written");
        let from = sysroot
            .as_ref()
            .map(|sysroot| sysroot.join("lib/rustlib").join(triple).join("lib/self-contained"));
        let have = from
            .as_ref()
            .is_some_and(|from| PARTS.iter().all(|part| from.join(part).exists()));
        let runtime = dir.join("libkoo_rt.a");
        let built = have && build(triple, &runtime);
        for part in PARTS {
            let copied = built && std::fs::copy(from.as_ref().unwrap().join(part), dir.join(part)).is_ok();
            // An empty file stands for a part this compiler was built without.
            if !copied {
                std::fs::write(dir.join(part), b"").expect("the output directory can be written");
            }
        }
        if built {
            slim(&runtime);
            slim(&dir.join("libc.a"));
        } else {
            std::fs::write(&runtime, b"").expect("the output directory can be written");
        }
    }
}
