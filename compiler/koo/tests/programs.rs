//! End-to-end tests: every program in `tests/run` is built and run, and every program in
//! `tests/fail` is rejected with the errors it names.
//!
//! A program states what it expects in comments:
//!
//! ```text
//! //= a line of standard output
//! //! exit 101              the exit status (0 if absent)
//! //! stderr some text      text that standard error contains
//! //! error some text       (tests/fail) text that a compile error contains
//! ```
//!
//! Programs in `tests/run` are built twice, unoptimized and with `--release`, and run with
//! `KOO_LEAKS` set, so a program that ends normally with memory still allocated fails.
//! The tests need `clang`, as `koo build` does.

use std::path::{Path, PathBuf};
use std::process::Command;

fn programs(dir: &str) -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(dir);
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap_or_else(|err| panic!("cannot read {}: {err}", root.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "koo"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no programs in {}", root.display());
    paths
}

struct Expectation {
    stdout: String,
    exit: i32,
    stderr: Vec<String>,
    errors: Vec<String>,
}

fn expectation(source: &str) -> Expectation {
    let mut expected = Expectation {
        stdout: String::new(),
        exit: 0,
        stderr: Vec::new(),
        errors: Vec::new(),
    };
    for line in source.lines() {
        if let Some(text) = line.strip_prefix("//=") {
            expected.stdout.push_str(text.strip_prefix(' ').unwrap_or(text));
            expected.stdout.push('\n');
        } else if let Some(text) = line.strip_prefix("//! exit ") {
            expected.exit = text.trim().parse().expect("an exit status");
        } else if let Some(text) = line.strip_prefix("//! stderr ") {
            expected.stderr.push(text.to_string());
        } else if let Some(text) = line.strip_prefix("//! error ") {
            expected.errors.push(text.to_string());
        }
    }
    expected
}

#[test]
fn programs_run_as_expected() {
    let mut failures = Vec::new();
    for path in programs("tests/run") {
        let source = std::fs::read_to_string(&path).unwrap();
        let expected = expectation(&source);
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        for release in [false, true] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_koo"));
            command.arg("run").arg(&path).env("KOO_LEAKS", "1");
            if release {
                command.arg("--release");
            }
            let output = command.output().expect("koo runs");
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let mode = if release { "release" } else { "debug" };
            if stdout != expected.stdout {
                failures.push(format!(
                    "{name} ({mode}): wrong output\n--- expected\n{}--- found\n{stdout}--- stderr\n{stderr}",
                    expected.stdout
                ));
                continue;
            }
            if output.status.code() != Some(expected.exit) {
                failures.push(format!(
                    "{name} ({mode}): exit status {:?}, expected {}\n{stderr}",
                    output.status.code(),
                    expected.exit
                ));
            }
            for text in &expected.stderr {
                if !stderr.contains(text) {
                    failures.push(format!("{name} ({mode}): stderr lacks {text:?}\n{stderr}"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

/// A chain of list methods prints the same whether the compiler makes one pass of it or
/// not (Section 4.5). `programs_run_as_expected` ran the fused form; this runs each
/// program that has a chain with the fusing turned off.
#[test]
fn chains_print_the_same_unfused() {
    let mut failures = Vec::new();
    for path in programs("tests/run") {
        let source = std::fs::read_to_string(&path).unwrap();
        if !source.contains(".map(") && !source.contains(".filter(") {
            continue;
        }
        let expected = expectation(&source);
        let output = Command::new(env!("CARGO_BIN_EXE_koo"))
            .arg("run")
            .arg(&path)
            .env("KOO_LEAKS", "1")
            .env("KOO_NO_FUSE", "1")
            .output()
            .expect("koo runs");
        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout != expected.stdout || output.status.code() != Some(expected.exit) {
            failures.push(format!(
                "{}: unfused, wrong output\n--- expected\n{}--- found\n{stdout}--- stderr\n{}",
                path.display(),
                expected.stdout,
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

/// The programs in `examples` build and run cleanly in both modes.
#[test]
fn examples_run() {
    for path in programs("examples") {
        for release in [false, true] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_koo"));
            command.arg("run").arg(&path).env("KOO_LEAKS", "1");
            if release {
                command.arg("--release");
            }
            let output = command.output().expect("koo runs");
            assert!(
                output.status.success(),
                "{} failed:\n{}",
                path.display(),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}

/// `koo test` runs the tests of each file in `tests/test` and reports them as expected.
/// `TESTFILE` in an expected line stands for the path of the file.
#[test]
fn test_files_report_as_expected() {
    for path in programs("tests/test") {
        let source = std::fs::read_to_string(&path).unwrap();
        let expected = expectation(&source);
        let output = Command::new(env!("CARGO_BIN_EXE_koo"))
            .arg("test")
            .arg(&path)
            .output()
            .expect("koo runs");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let wanted = expected.stdout.replace("TESTFILE", &path.to_string_lossy());
        assert_eq!(
            stdout,
            wanted,
            "{}\n{}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.status.code(), Some(expected.exit), "{}", path.display());
    }
}

/// A directory in `tests/packages` is a package (Section 8.1): its files are one program.
/// What is expected of it is written in its `main.koo`, as for the other tests: either
/// its output, or the errors it is rejected with.
#[test]
fn packages_build_as_one_program() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/packages");
    let mut packages: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect();
    packages.sort();
    assert!(!packages.is_empty());
    let mut failures = Vec::new();
    for package in packages {
        let name = package.file_name().unwrap().to_string_lossy().to_string();
        let expected = expectation(&std::fs::read_to_string(package.join("main.koo")).unwrap());
        if !expected.errors.is_empty() {
            let output = Command::new(env!("CARGO_BIN_EXE_koo"))
                .arg("check")
                .arg(&package)
                .output()
                .unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            for text in expected.errors.iter().chain(&expected.stderr) {
                if !stderr.contains(text) {
                    failures.push(format!("{name}: no error containing {text:?}\n{stderr}"));
                }
            }
            let reported = stderr.lines().filter(|line| line.starts_with("error: ")).count();
            if reported != expected.errors.len() {
                failures.push(format!(
                    "{name}: {reported} errors, {} expected\n{stderr}",
                    expected.errors.len()
                ));
            }
            continue;
        }
        for mode in ["debug", "release"] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_koo"));
            command.arg("run");
            if mode == "release" {
                command.arg("--release");
            }
            let output = command.arg(&package).env("KOO_LEAKS", "1").output().unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stdout != expected.stdout || output.status.code() != Some(expected.exit) {
                failures.push(format!(
                    "{name} ({mode}): status {:?}\n--- expected\n{}--- found\n{stdout}--- stderr\n{stderr}",
                    output.status.code(),
                    expected.stdout
                ));
            }
            for text in &expected.stderr {
                if !stderr.contains(text) {
                    failures.push(format!("{name} ({mode}): stderr lacks {text:?}\n{stderr}"));
                }
            }
        }
        // The tests of the package, if it has any.
        let has_tests = std::fs::read_dir(&package)
            .unwrap()
            .any(|entry| entry.unwrap().file_name().to_string_lossy().ends_with("_test.koo"));
        if has_tests {
            let output = Command::new(env!("CARGO_BIN_EXE_koo"))
                .arg("test")
                .arg(&package)
                .output()
                .unwrap();
            if !output.status.success() {
                failures.push(format!(
                    "{name}: its tests fail\n{}",
                    String::from_utf8_lossy(&output.stdout)
                ));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

/// Every program in the language tour runs and prints what the tour says it prints: a
/// `koo` block followed by a `text` block is a program and its output.
#[test]
fn tour_programs_print_what_the_tour_says() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let tour = std::fs::read_to_string(root.join("docs/tour.md")).unwrap();
    let dir = std::env::temp_dir().join(format!("koo-tour-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut count = 0;
    let mut rest = tour.as_str();
    while let Some(start) = rest.find("```koo\n") {
        let after = &rest[start + 7..];
        let end = after.find("```").expect("a closed block");
        let source = &after[..end];
        rest = &after[end + 3..];
        let Some(expected) = rest.strip_prefix("\n\n```text\n") else {
            continue;
        };
        let expected = &expected[..expected.find("```").expect("a closed block")];
        let path = dir.join(format!("block{count}.koo"));
        std::fs::write(&path, source).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_koo"))
            .arg("run")
            .arg(&path)
            .env("KOO_LEAKS", "1")
            .output()
            .expect("koo runs");
        let line = source.lines().next().unwrap_or("");
        assert!(
            output.status.success(),
            "the tour program beginning `{line}` failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            expected,
            "the tour program beginning `{line}`"
        );
        count += 1;
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(count >= 15, "the tour has programs: found {count}");
}

#[test]
fn bad_programs_are_rejected() {
    let mut failures = Vec::new();
    for path in programs("tests/fail") {
        let source = std::fs::read_to_string(&path).unwrap();
        let expected = expectation(&source);
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        assert!(!expected.errors.is_empty(), "{name} names no expected error");
        let output = Command::new(env!("CARGO_BIN_EXE_koo"))
            .arg("check")
            .arg(&path)
            .output()
            .expect("koo runs");
        let stderr = String::from_utf8_lossy(&output.stderr);
        if output.status.success() {
            failures.push(format!("{name}: accepted"));
            continue;
        }
        for text in &expected.errors {
            if !stderr.contains(text) {
                failures.push(format!("{name}: no error containing {text:?}\n{stderr}"));
            }
        }
        // Help the errors come with, where a test asks for it.
        for text in &expected.stderr {
            if !stderr.contains(text) {
                failures.push(format!("{name}: no help containing {text:?}\n{stderr}"));
            }
        }
        // Every error is expected: the count of errors is the count of expectations.
        let reported = stderr.lines().filter(|line| line.starts_with("error: ")).count();
        if reported != expected.errors.len() {
            failures.push(format!(
                "{name}: {reported} errors reported, {} expected\n{stderr}",
                expected.errors.len()
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}
