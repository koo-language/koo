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
//! The files in `tests/diag` hold what the compiler says about common mistakes, word for
//! word. Each case is a title, a program and the errors it gets:
//!
//! ```text
//! === a title
//! fn main() {
//!     ...
//! }
//! ---
//! error: ...
//! ```
//!
//! Run the tests with `KOO_BLESS=1` to write what the compiler says now into the files,
//! and read the difference before keeping it: these messages are what a programmer sees.
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

/// `koo build --target` builds a program for Linux on any machine: the file it leaves is
/// a Linux program for the architecture that was named, with everything linked in. On a
/// machine of that kind the program is run as well. A compiler built without a target,
/// or a machine without the linker, says which is missing, and then the test says so
/// and checks nothing for that target; with `KOO_TEST_TARGETS` set, as where the
/// compiler is built to be released, it fails instead.
#[test]
fn a_program_is_built_for_another_machine() {
    let koo = env!("CARGO_BIN_EXE_koo");
    let hello = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/hello.koo");
    let dir = std::env::temp_dir().join(format!("koo-targets-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let here = if cfg!(target_os = "linux") {
        std::env::consts::ARCH
    } else {
        ""
    };
    for (target, spelled, machine, architecture) in [
        ("linux-arm64", "linux/arm64", 0xb7, "aarch64"),
        ("linux-x86-64", "linux/amd64", 0x3e, "x86_64"),
    ] {
        for (name, release) in [(target, false), (spelled, true)] {
            let executable = dir.join(format!("{target}-{release}"));
            let mut build = Command::new(koo);
            build
                .arg("build")
                .arg("--target")
                .arg(name)
                .arg(&hello)
                .arg("-o")
                .arg(&executable);
            if release {
                build.arg("--release");
            }
            let built = build.env_remove("KOO_TARGET").output().expect("koo runs");
            let said = String::from_utf8_lossy(&built.stderr);
            if !built.status.success() && (said.contains("was built without") || said.contains("needs LLVM's linker")) {
                assert!(std::env::var_os("KOO_TEST_TARGETS").is_none(), "{target}: {said}");
                eprintln!("{target} is not checked: {said}");
                continue;
            }
            assert!(built.status.success(), "{target}: {said}");
            let program = std::fs::read(&executable).unwrap();
            assert_eq!(&program[..5], b"\x7fELF\x02", "{target}: a 64-bit Linux program");
            assert_eq!(
                u16::from_le_bytes([program[18], program[19]]),
                machine,
                "{target}: the architecture"
            );
            // A program that loads its C library when it starts names the loader.
            assert!(
                !program.windows(8).any(|bytes| bytes == b"/lib/ld-"),
                "{target}: all linked in"
            );
            if here == architecture {
                let output = Command::new(&executable).output().expect("the program runs");
                assert_eq!(
                    String::from_utf8_lossy(&output.stdout),
                    "hello, world\nfib(30) from Koo is 832040\n",
                    "{target}"
                );
                assert!(output.status.success(), "{target}");
            }
        }
    }
    let refused = |arguments: &[&str], said: &str| {
        let output = Command::new(koo)
            .args(arguments)
            .arg(&hello)
            .env_remove("KOO_TARGET")
            .output()
            .expect("koo runs");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{arguments:?}: {stderr}");
        assert!(stderr.contains(said), "{arguments:?}: {stderr}");
    };
    refused(
        &["build", "--target", "windows"],
        "there is no target `windows`. The targets are",
    );
    refused(&["build", "--target=macos"], "A program for macOS is built on a Mac");
    let output = Command::new(koo)
        .args(["build", "--target"])
        .output()
        .expect("koo runs");
    assert!(String::from_utf8_lossy(&output.stderr).contains("`--target` is followed by a target"));
    refused(
        &["build", "--sanitize", "--target", "linux-arm64"],
        "`--sanitize` is for a program built without",
    );
    let other = if here == "aarch64" {
        "linux-x86-64"
    } else {
        "linux-arm64"
    };
    refused(
        &["run", "--target", other],
        "this machine does not run a program built for",
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A function that calls itself without end is a panic, and not the end of the process
/// by a signal, however much the program was given as its environment: where the stack
/// of a program ends is counted from above its environment, which a C library may not
/// count in what it calls the stack.
#[test]
fn deep_recursion_is_a_panic_whatever_the_environment() {
    let program = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/run/stack_overflow.koo");
    for size in [0, 6_000, 20_000, 100_000] {
        let output = Command::new(env!("CARGO_BIN_EXE_koo"))
            .arg("run")
            .arg(&program)
            .env("KOO_TEST_PADDING", "x".repeat(size))
            .output()
            .expect("koo runs");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            output.status.code(),
            Some(101),
            "with {size} bytes more of environment: {stderr}"
        );
        assert!(
            stderr.contains("the call stack is exhausted"),
            "with {size} bytes more of environment: {stderr}"
        );
    }
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

/// Actors run on several threads, and what they hand each other is shared between those
/// threads. Built with ThreadSanitizer, each program with actors runs without a report:
/// nothing is reached from two threads that is not frozen or handed over. Where clang
/// has no ThreadSanitizer the test says so and checks nothing.
#[test]
fn actors_share_only_what_is_frozen() {
    let dir = std::env::temp_dir().join(format!("koo-threads-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut failures = Vec::new();
    for path in programs("tests/run") {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        if !name.starts_with("actor") {
            continue;
        }
        let executable = dir.join(&name);
        let built = Command::new(env!("CARGO_BIN_EXE_koo"))
            .arg("build")
            .arg("--sanitize=thread")
            .env_remove("KOO_TARGET")
            .arg(&path)
            .arg("-o")
            .arg(&executable)
            .output()
            .expect("koo runs");
        if !built.status.success() {
            let said = String::from_utf8_lossy(&built.stderr);
            if said.contains("tsan") || said.contains("sanitizer") {
                eprintln!("no ThreadSanitizer here: actors were not checked for races");
                let _ = std::fs::remove_dir_all(&dir);
                return;
            }
            failures.push(format!("{}: does not build\n{said}", path.display()));
            continue;
        }
        // More than once: what goes wrong between threads goes wrong now and then.
        for _ in 0..3 {
            let output = Command::new(&executable).output().expect("the program runs");
            let said = String::from_utf8_lossy(&output.stderr);
            if said.contains("WARNING: ThreadSanitizer") {
                failures.push(format!("{}: a race\n{said}", path.display()));
                break;
            }
            // The sanitizer itself may be unable to run, as in a container that does not
            // let it arrange the memory of the process: then nothing was checked.
            if said.contains("ThreadSanitizer: CHECK failed") || said.contains("FATAL: ThreadSanitizer") {
                eprintln!("ThreadSanitizer cannot run here: actors were not checked for races");
                let _ = std::fs::remove_dir_all(&dir);
                return;
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
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

/// The address of the PostgreSQL server that the tests of `std/sql` use, if there is one:
/// the environment variable `KOO_TEST_POSTGRES`, as `postgres://user:password@host/name`.
/// The tests make and drop tables whose names begin with `koo_`, and a schema named
/// `koo_book`; without the variable they are skipped.
fn test_database() -> Option<String> {
    // What the environment says of a database, by the names PostgreSQL's client knows,
    // is what an address that leaves a part out has for it: whoever runs the tests may
    // have such variables set, and the programs the tests run are not to hear them.
    static CLEARED: std::sync::Once = std::sync::Once::new();
    CLEARED.call_once(|| {
        for name in ["PGHOST", "PGPORT", "PGUSER", "PGPASSWORD", "PGDATABASE", "PGSSLMODE"] {
            // (The tests of this file read and change the environment only through
            // the standard library, which does both one at a time.)
            unsafe { std::env::remove_var(name) };
        }
    });
    std::env::var("KOO_TEST_POSTGRES").ok().filter(|url| !url.is_empty())
}

/// Only one test at a time uses the database.
static DATABASE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The address at which a program of the book or the tour finds a database of its own,
/// in the schema `koo_book`, if the tests have a server to use.
fn book_database() -> Option<String> {
    test_database().map(|url| {
        let joint = if url.contains('?') { '&' } else { '?' };
        format!("{url}{joint}options=-csearch_path%3Dkoo_book")
    })
}

/// Empties the schema `koo_book`, with a program written into `dir` for the purpose.
fn empty_book_database(dir: &Path, url: &str) -> Result<(), String> {
    let path = dir.join("empty.koo");
    std::fs::write(
        &path,
        "import std/os\nimport std/sql\nimport std/sql/postgres\n\nfn main() ! sql.Error {\n    \
         db := postgres.connect(os.env(\"DATABASE_URL\") else \"\")?\n    \
         db.exec(\"drop schema if exists koo_book cascade\")?\n    \
         db.exec(\"create schema koo_book\")?\n}\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_koo"))
        .arg("run")
        .arg(&path)
        .env("DATABASE_URL", url)
        .output()
        .expect("koo runs");
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "the database cannot be emptied:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

/// The programs in `tests/sql` do, against a real database, what they say they do; and
/// again over TLS, if `KOO_TEST_POSTGRES_TLS` is the address of a server that speaks it,
/// with the `sslmode` and the `sslrootcert` to use.
#[test]
fn a_database_answers_queries() {
    let Some(url) = test_database() else {
        eprintln!("KOO_TEST_POSTGRES is not set: the tests of `std/sql` against a database are skipped");
        return;
    };
    let _alone = DATABASE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let encrypted = std::env::var("KOO_TEST_POSTGRES_TLS")
        .ok()
        .filter(|url| !url.is_empty());
    let mut failures = Vec::new();
    for path in programs("tests/sql") {
        let source = std::fs::read_to_string(&path).unwrap();
        let expected = expectation(&source);
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if let Some(encrypted) = &encrypted {
            let output = Command::new(env!("CARGO_BIN_EXE_koo"))
                .arg("run")
                .arg(&path)
                .env("KOO_LEAKS", "1")
                .env("KOO_TEST_POSTGRES", encrypted)
                .output()
                .expect("koo runs");
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout != expected.stdout || output.status.code() != Some(expected.exit) {
                failures.push(format!(
                    "{name} (over TLS): exit status {:?}\n--- expected\n{}--- found\n{stdout}--- stderr\n{}",
                    output.status.code(),
                    expected.stdout,
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
        }
        for release in [false, true] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_koo"));
            command
                .arg("run")
                .arg(&path)
                .env("KOO_LEAKS", "1")
                .env("KOO_TEST_POSTGRES", &url);
            if release {
                command.arg("--release");
            }
            let output = command.output().expect("koo runs");
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let mode = if release { "release" } else { "debug" };
            if stdout != expected.stdout || output.status.code() != Some(expected.exit) {
                failures.push(format!(
                    "{name} ({mode}): exit status {:?}\n--- expected\n{}--- found\n{stdout}--- stderr\n{stderr}",
                    output.status.code(),
                    expected.stdout
                ));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

/// A proxy in front of the test database that fails on purpose. A statement with the
/// comment `koo_cut_reply` is passed to the database, and when the database has
/// answered it, the connection is closed in place of passing the answer on; one with
/// `koo_close_idle` is answered, and the connection closed just after. Returns the
/// address of the proxy, as the address of the database with the proxy's in its place.
fn failing_database(url: &str) -> String {
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU8, Ordering};
    let after = url.find('@').map_or(url.find("//").map_or(0, |at| at + 2), |at| at + 1);
    let end = url[after..].find(['/', '?']).map_or(url.len(), |at| after + at);
    let mut upstream = url[after..end].to_string();
    if !upstream.contains(':') {
        upstream.push_str(":5432");
    }
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port for the proxy");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for client in listener.incoming().flatten() {
            let Ok(server) = TcpStream::connect(&upstream) else {
                continue;
            };
            let told = Arc::new(AtomicU8::new(0));
            let (mut from_client, mut to_server) = (client.try_clone().unwrap(), server.try_clone().unwrap());
            let heard = told.clone();
            std::thread::spawn(move || {
                let mut chunk = [0u8; 65536];
                while let Ok(n) = from_client.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    let has = |word: &[u8]| chunk[..n].windows(word.len()).any(|at| at == word);
                    if has(b"koo_cut_reply") {
                        heard.store(1, Ordering::SeqCst);
                    } else if has(b"koo_close_idle") {
                        heard.store(2, Ordering::SeqCst);
                    }
                    if to_server.write_all(&chunk[..n]).is_err() {
                        break;
                    }
                }
                let _ = to_server.shutdown(Shutdown::Both);
            });
            let (mut from_server, mut to_client) = (server, client);
            std::thread::spawn(move || {
                let mut chunk = [0u8; 65536];
                while let Ok(n) = from_server.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    match told.swap(0, Ordering::SeqCst) {
                        // The database has answered, so the statement is done; the
                        // answer goes nowhere.
                        1 => break,
                        2 => {
                            let _ = to_client.write_all(&chunk[..n]);
                            std::thread::sleep(std::time::Duration::from_millis(100));
                            break;
                        }
                        _ => {
                            if to_client.write_all(&chunk[..n]).is_err() {
                                break;
                            }
                        }
                    }
                }
                let _ = to_client.shutdown(Shutdown::Both);
                let _ = from_server.shutdown(Shutdown::Both);
            });
        }
    });
    format!("{}127.0.0.1:{port}{}", &url[..after], &url[end..])
}

/// A database that is cut off in the middle of a statement, or closes a connection
/// that was idle, is survived: nothing is done twice, and what can be done again is.
#[test]
fn a_failing_database_is_survived() {
    let Some(url) = test_database() else {
        eprintln!("KOO_TEST_POSTGRES is not set: the tests of a failing database are skipped");
        return;
    };
    let _alone = DATABASE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let proxy = failing_database(&url);
    let mut failures = Vec::new();
    for path in programs("tests/faults") {
        let expected = expectation(&std::fs::read_to_string(&path).unwrap());
        let output = Command::new(env!("CARGO_BIN_EXE_koo"))
            .arg("run")
            .arg(&path)
            .env("KOO_LEAKS", "1")
            .env("KOO_TEST_POSTGRES", &proxy)
            .output()
            .expect("koo runs");
        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout != expected.stdout || output.status.code() != Some(expected.exit) {
            failures.push(format!(
                "{}: exit status {:?}\n--- expected\n{}--- found\n{stdout}--- stderr\n{}",
                path.display(),
                output.status.code(),
                expected.stdout,
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

/// `Type.from_env()` reads a struct of settings from the environment: `tests/env/*.koo`
/// are run with the environment that the file of the same name with `.env` lists, a
/// variable to a line, and print what the file with `.txt` holds.
#[test]
fn settings_come_from_the_environment() {
    let bless = std::env::var_os("KOO_BLESS").is_some();
    let mut failures = Vec::new();
    for path in programs("tests/env") {
        let given = std::fs::read_to_string(path.with_extension("env")).unwrap_or_default();
        let mut command = Command::new(env!("CARGO_BIN_EXE_koo"));
        command.arg("run").arg(&path).env("KOO_LEAKS", "1");
        for line in given.lines() {
            if let Some((name, value)) = line.split_once('=') {
                command.env(name, value);
            }
        }
        let output = command.output().expect("koo runs");
        let found = String::from_utf8_lossy(&output.stdout).to_string();
        let expected_path = path.with_extension("txt");
        let expected = std::fs::read_to_string(&expected_path).unwrap_or_default();
        if !output.status.success() {
            failures.push(format!(
                "{}: it fails\n{}",
                path.display(),
                String::from_utf8_lossy(&output.stderr)
            ));
        } else if found != expected {
            if bless {
                std::fs::write(&expected_path, &found).unwrap();
            } else {
                failures.push(format!(
                    "{}: not as expected\n--- expected\n{expected}--- found\n{found}",
                    path.display()
                ));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

/// With `KOO_LOG=json` each line of the log is an object, with the values of the line as
/// members: `tests/log/*.koo` are run so, with the time left out, and what they write to
/// the standard error output is what the file of the same name with `.json` holds.
#[test]
fn logs_are_lines_of_json() {
    let bless = std::env::var_os("KOO_BLESS").is_some();
    let mut failures = Vec::new();
    for path in programs("tests/log") {
        let output = Command::new(env!("CARGO_BIN_EXE_koo"))
            .arg("run")
            .arg(&path)
            .env("KOO_LEAKS", "1")
            .env("KOO_LOG", "json")
            .env("KOO_LOG_TIME", "off")
            .output()
            .expect("koo runs");
        let found = String::from_utf8_lossy(&output.stderr).to_string();
        let expected_path = path.with_extension("json");
        let expected = std::fs::read_to_string(&expected_path).unwrap_or_default();
        if !output.status.success() {
            failures.push(format!("{}: it fails\n{found}", path.display()));
        } else if found != expected {
            if bless {
                std::fs::write(&expected_path, &found).unwrap();
            } else {
                failures.push(format!(
                    "{}: the log is not as expected\n--- expected\n{expected}--- found\n{found}",
                    path.display()
                ));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
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

/// A fenced block of a page of the book: what it is tagged as, and the lines it is on.
struct Block {
    tag: String,
    /// The first line inside the fence, and the line of the closing fence.
    first: usize,
    end: usize,
}

/// A program in the book, and the blocks after it that say what it does.
struct Listing {
    page: usize,
    program: usize,
    told: Vec<usize>,
}

/// What a program of the book did when it was built and run.
struct Outcome {
    /// For each block that says what the program does, by tag: what the block should hold.
    found: Vec<(String, String)>,
    /// What is wrong besides a block that holds something else.
    wrong: Vec<String>,
}

fn run_listing(dir: &Path, source: &str, tags: &[(String, String)]) -> Outcome {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("main.koo"), source).unwrap();
    // The files the book says are there when the program runs.
    for (tag, text) in tags {
        if let Some(name) = tag.strip_prefix("file ") {
            let path = dir.join(name.trim());
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }
    let has = |tag: &str| tags.iter().any(|(found, _)| found == tag);
    let told = |tag: &str| {
        tags.iter()
            .find(|(found, _)| found == tag)
            .map(|(_, text)| text.clone())
    };
    // A program that uses a database is given an empty one, in a schema of its own, at
    // the address it reads from `DATABASE_URL`. Without a database to give, it is only
    // checked, and what the book says it does is not compared.
    let uses_database = source.contains("import std/sql") && !has("error");
    let database = book_database().filter(|_| uses_database);
    let _alone = database
        .as_ref()
        .map(|_| DATABASE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()));
    let database_url = database.clone().unwrap_or_default();
    let koo = |args: &[&str], input: Option<String>| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_koo"));
        // `json` in place of the arguments runs the program with its log as JSON.
        let as_json = args.first() == Some(&"json");
        command
            .args(if as_json { &[][..] } else { args })
            .current_dir(dir)
            .env("DATABASE_URL", &database_url)
            // The time of a line of the log would be different at every run.
            .env("KOO_LOG_TIME", "off")
            .env("KOO_LOG", if as_json { "json" } else { "text" })
            // The environment the book says the program is started in.
            .envs(
                told("env")
                    .unwrap_or_default()
                    .lines()
                    .filter_map(|line| line.split_once('='))
                    .map(|(name, value)| (name.to_string(), value.to_string())),
            )
            .args(if as_json { &["run", "main.koo"][..] } else { &[][..] })
            .env("KOO_LEAKS", "1")
            .env("NO_COLOR", "1")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = command.spawn().expect("koo runs");
        {
            use std::io::Write;
            let mut stdin = child.stdin.take().unwrap();
            let _ = stdin.write_all(input.unwrap_or_default().as_bytes());
        }
        let output = child.wait_with_output().expect("koo ends");
        (
            String::from_utf8_lossy(&output.stdout).to_string(),
            String::from_utf8_lossy(&output.stderr).to_string(),
            output.status.code(),
        )
    };
    let mut outcome = Outcome {
        found: Vec::new(),
        wrong: Vec::new(),
    };
    if uses_database {
        if database.is_none() {
            let (_, stderr, status) = koo(&["check", "main.koo"], None);
            if status != Some(0) {
                outcome.wrong.push(format!("the compiler refuses it:\n{stderr}"));
            }
            return outcome;
        }
        if let Err(wrong) = empty_book_database(dir, &database_url) {
            outcome.wrong.push(wrong);
            return outcome;
        }
    }
    if let Some(asked) = told("http") {
        // A server: it is started, sent the requests the book lists, each a line that
        // begins with `> `, and what it answers is written after each: the status, then
        // the body. The book says port 8080; the test uses one that is free.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|listener| listener.local_addr())
            .expect("a free port")
            .port();
        std::fs::write(
            dir.join("main.koo"),
            source
                .replace("port: 8080", &format!("port: {port}"))
                .replace("localhost:8080", &format!("localhost:{port}")),
        )
        .unwrap();
        let (_, stderr, status) = koo(&["build", "main.koo", "-o", "server"], None);
        if status != Some(0) {
            outcome.wrong.push(format!("the compiler refuses it:\n{stderr}"));
            return outcome;
        }
        let server = Command::new(dir.join("server"))
            .current_dir(dir)
            .env("DATABASE_URL", &database_url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("the server starts");
        let _server = Server(server);
        let started = std::time::Instant::now();
        while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
            if started.elapsed() > std::time::Duration::from_secs(10) {
                outcome.wrong.push("the server does not listen".to_string());
                return outcome;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let mut found = String::new();
        for line in asked.lines() {
            let Some(request) = line.strip_prefix("> ") else {
                continue;
            };
            let mut parts = request.splitn(3, ' ');
            let (method, path, body) = (
                parts.next().unwrap_or(""),
                parts.next().unwrap_or("/"),
                parts.next().unwrap_or(""),
            );
            // The body says what it is, as a client would: JSON, a form, or text.
            let kind = if body.starts_with('{') || body.starts_with('[') {
                "application/json"
            } else if body.contains('=') && !body.contains(' ') {
                "application/x-www-form-urlencoded"
            } else {
                "text/plain"
            };
            let answer = http(
                port,
                &format!(
                    "{method} {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                ),
            );
            let status = answer.split(' ').nth(1).unwrap_or("?");
            let said = answer.split_once("\r\n\r\n").map_or("", |(_, said)| said);
            found.push_str(&format!("> {request}\n{status}\n{said}"));
            if !said.is_empty() && !said.ends_with('\n') {
                found.push('\n');
            }
        }
        outcome.found.push(("http".to_string(), found));
    } else if has("error") {
        // A program the compiler refuses, and what it says.
        let (_, stderr, status) = koo(&["check", "main.koo"], None);
        if status == Some(0) {
            outcome.wrong.push("the compiler accepts it".to_string());
        }
        outcome.found.push(("error".to_string(), stderr));
    } else if has("test") {
        let (stdout, _, _) = koo(&["test", "main.koo"], None);
        outcome.found.push(("test".to_string(), stdout));
    } else if has("text") || has("panic") || has("stderr") || has("log") {
        let mut args = vec!["run".to_string(), "main.koo".to_string()];
        if let Some(given) = told("args") {
            args.push("--".to_string());
            args.extend(given.lines().map(str::to_string));
        }
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let (stdout, stderr, status) = koo(&args, told("input"));
        let wanted = if has("panic") {
            101
        } else if has("stderr") {
            1
        } else {
            0
        };
        if status != Some(wanted) {
            outcome
                .wrong
                .push(format!("it ends with status {status:?}, not {wanted}\n{stderr}"));
        }
        if has("text") {
            outcome.found.push(("text".to_string(), stdout));
        } else if !stdout.is_empty() {
            outcome
                .wrong
                .push(format!("it prints, and the book does not say what:\n{stdout}"));
        }
        if has("panic") {
            outcome.found.push(("panic".to_string(), stderr));
        } else if has("stderr") {
            outcome.found.push(("stderr".to_string(), stderr));
        } else if has("log") {
            outcome.found.push(("log".to_string(), stderr));
            if has("json") {
                let (_, stderr, _) = koo(&["json"], told("input"));
                outcome.found.push(("json".to_string(), stderr));
            }
        } else if !stderr.is_empty() {
            outcome.wrong.push(format!(
                "it writes to standard error, and the book does not say what:\n{stderr}"
            ));
        }
    } else {
        // A program that is only shown, such as a server: it must at least be a program.
        let (_, stderr, status) = koo(&["check", "main.koo"], None);
        if status != Some(0) {
            outcome.wrong.push(format!("the compiler refuses it:\n{stderr}"));
        }
    }
    outcome
}

/// Every program in the book is a whole program, and does what the book says it does.
///
/// A block tagged `koo` is a program. The blocks after it, up to the next program, say
/// what it does: `input` is what it reads, `args` its arguments, one to a line, `file
/// name` a file that is beside it when it runs, `env` variables of its environment, one
/// `NAME=value` to a line, `text`
/// what it prints, `panic` what a panic writes (the program then ends with status 101),
/// `stderr` what it writes when `main` fails (status 1), `log` what it writes to standard
/// error when it ends well all the same (and `json` the same with its log asked for as
/// JSON), `error` what the compiler says
/// when it refuses the program, `test` what `koo test` prints, and `http` what a server
/// answers to the requests listed in it. A program with none of these is checked and not
/// run. Each is compared to the letter.
///
/// With `KOO_BLESS=1` the blocks are written from what the programs do, which is how the
/// output in the book is put there in the first place: by the programs, not by hand.
#[test]
fn the_book_is_true() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/book");
    let mut pages: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap_or_else(|err| panic!("cannot read {}: {err}", root.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect();
    pages.sort();
    let mut texts: Vec<Vec<String>> = Vec::new();
    let mut blocks: Vec<Vec<Block>> = Vec::new();
    let mut listings: Vec<Listing> = Vec::new();
    const TOLD: [&str; 11] = [
        "input", "args", "env", "text", "panic", "stderr", "log", "json", "error", "test", "http",
    ];
    for (page, path) in pages.iter().enumerate() {
        let lines: Vec<String> = std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        let mut found: Vec<Block> = Vec::new();
        let mut open: Option<(String, usize)> = None;
        for (number, line) in lines.iter().enumerate() {
            let Some(tag) = line.strip_prefix("```") else { continue };
            match open.take() {
                Some((tag, first)) => found.push(Block {
                    tag,
                    first,
                    end: number,
                }),
                None => open = Some((tag.trim().to_string(), number + 1)),
            }
        }
        assert!(open.is_none(), "{}: a block is not closed", path.display());
        for (index, block) in found.iter().enumerate() {
            if block.tag == "koo" {
                listings.push(Listing {
                    page,
                    program: index,
                    told: Vec::new(),
                });
            } else if TOLD.contains(&block.tag.as_str()) || block.tag.starts_with("file ") {
                match listings.last_mut() {
                    Some(listing) if listing.page == page => listing.told.push(index),
                    _ => panic!(
                        "{}:{}: a `{}` block with no program before it",
                        path.display(),
                        block.first,
                        block.tag
                    ),
                }
            }
        }
        texts.push(lines);
        blocks.push(found);
    }
    assert!(!listings.is_empty(), "the book has no programs");

    let inside = |page: usize, block: &Block| {
        let mut text = texts[page][block.first..block.end].join("\n");
        if !text.is_empty() {
            text.push('\n');
        }
        text
    };
    let dir = std::env::temp_dir().join(format!("koo-book-{}", std::process::id()));
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let outcomes: std::sync::Mutex<Vec<(usize, Outcome)>> = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(listing) = listings.get(index) else { break };
                    let source = inside(listing.page, &blocks[listing.page][listing.program]);
                    let tags: Vec<(String, String)> = listing
                        .told
                        .iter()
                        .map(|told| {
                            let block = &blocks[listing.page][*told];
                            (block.tag.clone(), inside(listing.page, block))
                        })
                        .collect();
                    let outcome = run_listing(&dir.join(format!("p{index}")), &source, &tags);
                    outcomes.lock().unwrap().push((index, outcome));
                }
            });
        }
    });
    let _ = std::fs::remove_dir_all(&dir);

    let bless = std::env::var_os("KOO_BLESS").is_some();
    let mut failures = Vec::new();
    // What to put in a block, by page and by the block's place on it.
    let mut rewrites: Vec<(usize, usize, String)> = Vec::new();
    let mut outcomes = outcomes.into_inner().unwrap();
    outcomes.sort_by_key(|(index, _)| *index);
    for (index, outcome) in outcomes {
        let listing = &listings[index];
        let page = &pages[listing.page];
        let place = format!(
            "{}:{}",
            page.file_name().unwrap().to_string_lossy(),
            blocks[listing.page][listing.program].first
        );
        for wrong in outcome.wrong {
            failures.push(format!("{place}: {wrong}"));
        }
        for (tag, found) in outcome.found {
            let told = listing
                .told
                .iter()
                .find(|told| blocks[listing.page][**told].tag == tag)
                .expect("a block of the tag");
            let block = &blocks[listing.page][*told];
            let expected = inside(listing.page, block);
            if expected != found {
                if bless {
                    rewrites.push((listing.page, *told, found));
                } else {
                    failures.push(format!(
                        "{place}: the `{tag}` block is not what the program does\n--- the book\n{expected}--- the program\n{found}"
                    ));
                }
            }
        }
    }
    if bless {
        for (page, path) in pages.iter().enumerate() {
            let mut mine: Vec<&(usize, usize, String)> = rewrites.iter().filter(|(of, _, _)| *of == page).collect();
            if mine.is_empty() {
                continue;
            }
            // From the last block to the first, so that the lines before keep their numbers.
            mine.sort_by_key(|(_, block, _)| std::cmp::Reverse(*block));
            let mut lines = texts[page].clone();
            for (_, block, found) in mine {
                let block = &blocks[page][*block];
                lines.splice(block.first..block.end, found.lines().map(str::to_string));
            }
            std::fs::write(path, lines.join("\n") + "\n").unwrap();
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
        let line = source.lines().next().unwrap_or("");
        // A program that uses a database is run against an empty one, if the tests
        // have a server, and is otherwise only checked.
        let mut database = String::new();
        let mut _alone = None;
        if source.contains("import std/sql/postgres") {
            let Some(url) = book_database() else {
                let output = Command::new(env!("CARGO_BIN_EXE_koo"))
                    .arg("check")
                    .arg(&path)
                    .output()
                    .expect("koo runs");
                assert!(
                    output.status.success(),
                    "the tour program beginning `{line}` is refused:\n{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                count += 1;
                continue;
            };
            _alone = Some(DATABASE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()));
            empty_book_database(&dir, &url).unwrap_or_else(|wrong| panic!("{wrong}"));
            database = url;
        }
        let output = Command::new(env!("CARGO_BIN_EXE_koo"))
            .arg("run")
            .arg(&path)
            .env("KOO_LEAKS", "1")
            .env("DATABASE_URL", &database)
            .output()
            .expect("koo runs");
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

/// One case of a file in `tests/diag`: its title, its program and the errors expected.
struct Case {
    title: String,
    program: String,
    expected: String,
}

fn cases(text: &str) -> Vec<Case> {
    let mut cases: Vec<Case> = Vec::new();
    let mut in_program = false;
    for line in text.lines() {
        if let Some(title) = line.strip_prefix("=== ") {
            cases.push(Case {
                title: title.to_string(),
                program: String::new(),
                expected: String::new(),
            });
            in_program = true;
        } else if let Some(case) = cases.last_mut() {
            if in_program && line == "---" {
                in_program = false;
            } else if in_program {
                case.program.push_str(line);
                case.program.push('\n');
            } else {
                case.expected.push_str(line);
                case.expected.push('\n');
            }
        }
    }
    for case in &mut cases {
        case.expected = format!("{}\n", case.expected.trim_end());
    }
    cases
}

#[test]
fn mistakes_are_explained_as_expected() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/diag");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap_or_else(|err| panic!("cannot read {}: {err}", root.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "txt"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no files in {}", root.display());
    let bless = std::env::var_os("KOO_BLESS").is_some();
    let dir = std::env::temp_dir().join(format!("koo-diag-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut failures = Vec::new();
    let mut count = 0;
    for file in files {
        let name = file.file_name().unwrap().to_string_lossy().to_string();
        let text = std::fs::read_to_string(&file).unwrap();
        let mut blessed = String::new();
        for case in cases(&text) {
            count += 1;
            std::fs::write(dir.join("main.koo"), &case.program).unwrap();
            let output = Command::new(env!("CARGO_BIN_EXE_koo"))
                .args(["check", "main.koo"])
                .current_dir(&dir)
                .env("NO_COLOR", "1")
                .output()
                .expect("koo runs");
            let found = format!("{}\n", String::from_utf8_lossy(&output.stderr).trim_end());
            if output.status.success() {
                failures.push(format!("{name}: `{}` is accepted", case.title));
            } else if found != case.expected && !bless {
                failures.push(format!(
                    "{name}: `{}`\n--- expected\n{}--- found\n{found}",
                    case.title, case.expected
                ));
            }
            blessed.push_str(&format!("=== {}\n{}---\n{found}\n", case.title, case.program));
        }
        if bless && blessed != text {
            std::fs::write(&file, blessed).unwrap();
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(count >= 100, "tests/diag has cases: found {count}");
    assert!(
        failures.is_empty(),
        "\n{}\n\nIf the new messages are the better ones, run the tests with KOO_BLESS=1.",
        failures.join("\n\n")
    );
}

/// One request on a connection of its own. The request asks for the connection to be
/// closed, so the answer is everything the server sends.
fn http(port: u16, request: &str) -> String {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("the server accepts");
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut answer = Vec::new();
    let _ = stream.read_to_end(&mut answer);
    String::from_utf8_lossy(&answer).to_string()
}

/// A request and its answer as bytes, for bodies that are not text.
fn http_bytes(port: u16, request: &[u8]) -> Vec<u8> {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("the server accepts");
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    stream.write_all(request).unwrap();
    let mut answer = Vec::new();
    let _ = stream.read_to_end(&mut answer);
    answer
}

fn get(port: u16, path: &str) -> String {
    http(
        port,
        &format!("GET {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n"),
    )
}

fn post(port: u16, path: &str, body: &str) -> String {
    http(
        port,
        &format!(
            "POST {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ),
    )
}

/// A server that is running. It is ended when the test is done with it, however the
/// test ends.
struct Server(std::process::Child);

impl Drop for Server {
    /// Asks the server to stop, so that it can tidy up after itself, and kills it only
    /// if it has not stopped in two seconds.
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(None)) {
            return;
        }
        let _ = Command::new("kill").args(["-TERM", &self.0.id().to_string()]).status();
        for _ in 0..200 {
            if !matches!(self.0.try_wait(), Ok(None)) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Starts `tests/http/server.koo` on a free port, with a number of worker processes.
fn serve(executable: &Path, workers: usize, files: &Path) -> (Server, u16) {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut child = Command::new(executable)
        .env("PORT", port.to_string())
        .env("WORKERS", workers.to_string())
        .env("STATIC", files)
        .stdout(std::process::Stdio::null())
        .env("KOO_LEAKS", "1")
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("the server starts");
    for _ in 0..200 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return (Server(child), port);
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!("the server did not start listening on port {port}");
}

/// Asks the server to stop, as Ctrl-C or a process manager would, and checks that it
/// does: with status 0, which under `KOO_LEAKS` also says that it freed what it allocated.
fn stop(mut server: Server, port: u16) {
    let child = &mut server.0;
    Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    let status = child.wait().unwrap();
    assert!(status.success(), "the server ended with {status}");
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(
        std::net::TcpStream::connect(("127.0.0.1", port)).is_err(),
        "something still listens on port {port}"
    );
}

/// The service in `examples/service`, which the README holds up as the pattern of a
/// service, is started as a twelve-factor app is, with nothing but its environment, and
/// answers what its README says it answers. It needs a database.
#[test]
fn the_example_service_serves() {
    let Some(url) = book_database() else {
        eprintln!("KOO_TEST_POSTGRES is not set: the example service is only built");
        let checked = Command::new(env!("CARGO_BIN_EXE_koo"))
            .arg("check")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/service/service.koo"))
            .output()
            .expect("koo runs");
        assert!(checked.status.success(), "{}", String::from_utf8_lossy(&checked.stderr));
        return;
    };
    let _alone = DATABASE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::temp_dir().join(format!("koo-service-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    empty_book_database(&dir, &url).unwrap_or_else(|wrong| panic!("{wrong}"));
    let executable = dir.join("service");
    let built = Command::new(env!("CARGO_BIN_EXE_koo"))
        .arg("build")
        .arg(root.join("examples/service/service.koo"))
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("koo runs");
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));

    // Without its settings it says which are missing, and does not start.
    let unset = Command::new(&executable).env_remove("DATABASE_URL").output().unwrap();
    assert_eq!(unset.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&unset.stderr).contains("the environment variable `DATABASE_URL` is not set"),
        "{}",
        String::from_utf8_lossy(&unset.stderr)
    );

    // A port or a number of workers that is none is said at once, and nothing is served.
    for (name, value) in [
        ("PORT", "99999"),
        ("PORT", "-5"),
        ("PORT", "http"),
        ("WEB_CONCURRENCY", "0"),
        ("WEB_CONCURRENCY", "many"),
    ] {
        let refused = Command::new(&executable)
            .env("DATABASE_URL", &url)
            .env("PORT", "1")
            .env(name, value)
            .output()
            .expect("the service runs");
        let said = String::from_utf8_lossy(&refused.stderr);
        assert_eq!(refused.status.code(), Some(1), "{name}={value}: {said}");
        assert!(
            said.contains(&format!("the environment variable `{name}` is `{value}`")),
            "{name}={value}: {said}"
        );
    }

    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let log = dir.join("log.txt");
    let child = Command::new(&executable)
        .env("DATABASE_URL", &url)
        .env("PORT", port.to_string())
        .env("WEB_CONCURRENCY", "2")
        .env("KOO_LOG", "json")
        .stdout(std::process::Stdio::null())
        .stderr(std::fs::File::create(&log).unwrap())
        .spawn()
        .expect("the service starts");
    let server = Server(child);
    for _ in 0..200 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    let has = |answer: &str, parts: &[&str]| {
        for part in parts {
            assert!(answer.contains(part), "no {part:?} in:\n{answer}");
        }
    };
    has(&get(port, "/healthz"), &["200 OK", r#"{"database":"ok"}"#]);
    has(
        &post(port, "/orders", r#"{"customer": "ada", "sku": "hammer", "qty": 2}"#),
        &["201 Created", r#"{"id":1,"customer":"ada","sku":"hammer","qty":2}"#],
    );
    has(
        &post(port, "/orders", r#"{"customer": "bob", "sku": "nails"}"#),
        &["201 Created", r#""id":2"#, r#""qty":1"#],
    );
    has(
        &post(port, "/orders", r#"{"customer": "bob", "sku": "nails", "qty": 0}"#),
        &["422 "],
    );
    has(&post(port, "/orders", r#"{"customer": "bob"}"#), &["400 ", "/sku"]);
    has(&get(port, "/orders/2"), &["200 OK", r#""customer":"bob""#]);
    has(&get(port, "/orders/9"), &["404 ", "there is no order 9"]);
    has(&get(port, "/orders/nine"), &["400 "]);
    // Told to stop, it stops, with status 0, and its log is a log a program can read.
    stop(server, port);
    let written = std::fs::read_to_string(&log).unwrap();
    for line in written.lines() {
        assert!(
            line.starts_with('{') && line.ends_with('}'),
            "not a line of JSON: {line}"
        );
        // Every line begins with its time, in one form whoever wrote the line: the
        // program, or the server for a request. `2026-01-31T12:00:00.123456Z`.
        let time = line
            .strip_prefix(r#"{"time":""#)
            .and_then(|rest| rest.split('"').next());
        assert!(
            time.is_some_and(|time| time.len() == 27 && time.as_bytes()[19] == b'.' && time.ends_with('Z')),
            "the time of a line is not as the others are: {line}"
        );
    }
    has(
        &written,
        &[
            r#""message":"the orders service is starting""#,
            r#""message":"order 1 placed by ada","order.id":1,"order.customer":"ada""#,
            r#""message":"POST /orders 201","method":"POST","path":"/orders","status":201"#,
            r#""message":"the orders service has stopped""#,
        ],
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A server for the tests of `std/net`, which answers each line as the line's first
/// word says: well, late, twice, without end, with what is not text, a byte at a time,
/// half and then nothing, half and then closed. Every answer it gives begins with the
/// number of the connection, counted from 1 as they are accepted.
///
/// It counts the connections it has open, so that a test can see that none is left
/// when its program is done, and no more at any time than the program may have.
fn tcp_server() -> (
    std::net::TcpListener,
    String,
    std::sync::Arc<std::sync::atomic::AtomicI64>,
) {
    use std::sync::atomic::{AtomicI64, Ordering};
    /// One fewer when the thread that has the connection is done with it, however.
    struct Open(std::sync::Arc<AtomicI64>);
    impl Drop for Open {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
    let accepting = listener.try_clone().unwrap();
    let open = std::sync::Arc::new(AtomicI64::new(0));
    let counted = open.clone();
    std::thread::spawn(move || {
        use std::io::{BufRead, Write};
        for (before, stream) in accepting.incoming().enumerate() {
            let Ok(mut stream) = stream else { break };
            let number = before + 1;
            counted.fetch_add(1, Ordering::SeqCst);
            let open = Open(counted.clone());
            std::thread::spawn(move || {
                let _open = open;
                let mut lines = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                loop {
                    line.clear();
                    if lines.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let (how, text) = line.trim_end().split_once(' ').unwrap_or((line.trim_end(), ""));
                    let answer = format!("{number} {text}\n");
                    let pause = |millis| std::thread::sleep(std::time::Duration::from_millis(millis));
                    let sent = match how {
                        "echo" => stream.write_all(answer.as_bytes()),
                        "late" => {
                            pause(400);
                            stream.write_all(answer.as_bytes())
                        }
                        "twice" => stream.write_all(format!("{answer}{answer}").as_bytes()),
                        "again" => {
                            let _ = stream.write_all(answer.as_bytes());
                            pause(60);
                            stream.write_all(answer.as_bytes())
                        }
                        "long" => stream.write_all("x".repeat(text.parse().unwrap_or(1)).as_bytes()),
                        "bytes" => stream.write_all(b"\xff\xfe not text\n"),
                        "slow" => answer.bytes().try_for_each(|byte| {
                            pause(3);
                            stream.write_all(&[byte])
                        }),
                        "stall" => stream.write_all(&answer.as_bytes()[..3]),
                        "cut" => {
                            let _ = stream.write_all(&answer.as_bytes()[..3]);
                            return;
                        }
                        _ => return,
                    };
                    if sent.is_err() {
                        return;
                    }
                }
            });
        }
    });
    (listener, address, open)
}

/// Waits a moment for the connections a server of the tests has open to be so many.
fn open_connections(open: &std::sync::atomic::AtomicI64, wanted: impl Fn(i64) -> bool) -> i64 {
    for _ in 0..100 {
        if wanted(open.load(std::sync::atomic::Ordering::SeqCst)) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    open.load(std::sync::atomic::Ordering::SeqCst)
}

/// A program talks to another over TCP, in conversations that each have a connection
/// only while they last. A connection is kept for the next conversation if nothing
/// went wrong on it, and not if the other end answered late, said more than it was
/// asked, or the program's function failed; what the other end sends is not taken on
/// trust; and a connection is of no use outside the function it was given to.
#[test]
fn a_program_talks_over_tcp() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let (_listener, address, open) = tcp_server();
    let output = Command::new(env!("CARGO_BIN_EXE_koo"))
        .arg("run")
        .arg(root.join("tests/net/talk.koo"))
        .env("ADDRESS", &address)
        .env("KOO_LEAKS", "1")
        .output()
        .expect("koo runs");
    let said = String::from_utf8_lossy(&output.stdout).to_string();
    let expected = "\
fresh: true false true
kept: 1 a | 1 b | 1 c
late: Timeout | 3 after
swallowed: gave up | 4 after
more: 4 y | 5 after
later: 5 q
later: 6 next
failed: the program's own | 7 after
outside: Closed: this connection is over: one is used only inside the function it was given to
long: TooLong
bytes: NotText
slow: 4 then e byte at a time
stall: Timeout
cut: Closed
full: Timeout: every connection to `ADDRESS` was in use for as long as was waited
addresses: 0 refused for being too many
once: 13 once
after all: 14 end
"
    .replace("ADDRESS", &address);
    assert!(
        output.status.success() && said == expected,
        "{}\n--- expected\n{expected}--- found\n{said}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The program is done, and so is every connection it had.
    assert_eq!(
        open_connections(&open, |open| open == 0),
        0,
        "connections left open by a program that has ended"
    );
}

/// The first conversations of a process, sixteen at once on as many threads: each is
/// answered with its own answer. (Two threads that came at the same moment to tables
/// that no thread had used each took the other's connections for those of another
/// process, and closed them: five runs in sixteen hundred.)
#[test]
fn threads_begin_talking_at_once() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::temp_dir().join(format!("koo-first-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let executable = dir.join("first");
    let built = Command::new(env!("CARGO_BIN_EXE_koo"))
        .arg("build")
        .arg(root.join("tests/net/first.koo"))
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("koo runs");
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let (_listener, address, _open) = tcp_server();
    for round in 0..150 {
        let output = Command::new(&executable).env("ADDRESS", &address).output().unwrap();
        let said = String::from_utf8_lossy(&output.stdout).to_string();
        assert!(
            output.status.success() && said == "16 of 16 right\n",
            "round {round}:\n{said}{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The same inside a server: the handlers of every worker, and an actor that with
/// several workers is in another process, each have their conversations on connections
/// of their own process, also where the pool was made before there were workers; and a
/// handler whose client leaves in the middle of a conversation leaves nothing behind
/// that the next conversation would hear.
#[test]
fn a_server_talks_over_tcp() {
    use std::io::Write;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::temp_dir().join(format!("koo-net-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let executable = dir.join("served");
    let built = Command::new(env!("CARGO_BIN_EXE_koo"))
        .arg("build")
        .arg(root.join("tests/net/served.koo"))
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("koo runs");
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let (_listener, address, open) = tcp_server();
    for workers in [1, 3] {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let log = dir.join(format!("log-{workers}.txt"));
        let child = Command::new(&executable)
            .env("ADDRESS", &address)
            .env("PORT", port.to_string())
            .env("WORKERS", workers.to_string())
            .env("KOO_LEAKS", "1")
            .stdout(std::process::Stdio::null())
            .stderr(std::fs::File::create(&log).unwrap())
            .spawn()
            .expect("the server starts");
        let server = Server(child);
        for _ in 0..200 {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let body = |answer: String| answer.rsplit("\r\n\r\n").next().unwrap_or("").trim().to_string();
        // Each answer is its asker's own: what was asked comes back, after the number
        // of the connection it came on.
        // The connection `main` had its first conversation on, before there were
        // workers, is the process's that made it: with several workers, the one that
        // watches over them, where the actor is, and no handler's.
        let first = body(get(port, "/first"));
        let parents = first.split(' ').next().unwrap_or("").to_string();
        assert!(
            first.ends_with(" first") && !parents.is_empty(),
            "with {workers} workers: {first}"
        );
        let rounds = |tag: &str| {
            let asked: Vec<_> = (0..90)
                .map(|i| {
                    let path = format!("/{}/{tag}{i}", ["main", "own", "actor"][i % 3]);
                    std::thread::spawn(move || (path.clone(), get(port, &path)))
                })
                .collect();
            for answer in asked {
                let (path, answer) = answer.join().unwrap();
                let (said, wanted) = (body(answer), path.rsplit('/').next().unwrap().to_string());
                assert!(
                    said.split_once(' ').is_some_and(|(_, text)| text == wanted),
                    "with {workers} workers, {path} was answered `{said}`"
                );
                let on = said.split(' ').next().unwrap_or("");
                assert!(
                    workers == 1 || path.starts_with("/actor/") || on != parents,
                    "with {workers} workers, {path} was answered on connection {on}, which the process that started the workers made"
                );
            }
        };
        rounds("a");
        // Clients that leave while their handlers wait for an answer that is late: the
        // answers come all the same, to connections that nobody will use again.
        let left: Vec<_> = (0..12)
            .map(|i| {
                let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
                write!(stream, "GET /late/gone{i} HTTP/1.1\r\nHost: test\r\n\r\n").unwrap();
                stream
            })
            .collect();
        std::thread::sleep(std::time::Duration::from_millis(150));
        drop(left);
        rounds("b");
        std::thread::sleep(std::time::Duration::from_millis(500));
        rounds("c");
        // No process has more connections than its pool may, with the one more that
        // an actor in a server of one process is given at need; and those of the
        // clients that left are closed.
        let most = (workers as i64 + 1) * 5;
        let now = open_connections(&open, |open| open <= most);
        assert!(
            now <= most,
            "with {workers} workers, {now} connections are open, and at most {most} may be"
        );
        stop(server, port);
        let written = std::fs::read_to_string(&log).unwrap();
        assert!(
            written.is_empty(),
            "with {workers} workers the server wrote:\n{written}"
        );
        // The server is gone, and so is every connection it had.
        assert_eq!(
            open_connections(&open, |open| open == 0),
            0,
            "with {workers} workers, connections left open"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// What the Redis client says first on a connection: who is asking, with a password
/// that is written in the address as an address writes it, and of which database;
/// and that on the first connection of the pool, which was made before anything was
/// asked, as on every later one. The server is one of the test's own, which answers
/// `OK` to everything and writes down what it was told.
#[test]
fn a_redis_is_greeted() {
    use std::io::{BufRead, Write};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::temp_dir().join(format!("koo-greeting-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(root.join("packages/redis/redis.koo"), dir.join("redis.koo")).unwrap();
    std::fs::copy(root.join("tests/redis/greeting/main.koo"), dir.join("main.koo")).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let told = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let writing = told.clone();
    std::thread::spawn(move || {
        for (number, stream) in listener.incoming().enumerate() {
            let Ok(mut stream) = stream else { break };
            let writing = writing.clone();
            std::thread::spawn(move || {
                let mut lines = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                let mut next = |line: &mut String| {
                    line.clear();
                    lines.read_line(line).unwrap_or(0) > 0
                };
                // A command: `*` and how many words, and for each its length and itself.
                while next(&mut line) {
                    let words: usize = line.trim().trim_start_matches('*').parse().unwrap_or(0);
                    let mut command = Vec::new();
                    for _ in 0..words {
                        if !next(&mut line) || !next(&mut line) {
                            return;
                        }
                        command.push(line.trim_end().to_string());
                    }
                    writing
                        .lock()
                        .unwrap()
                        .push(format!("{}: {}", number + 1, command.join(" ")));
                    if stream.write_all(b"+OK\r\n").is_err() {
                        return;
                    }
                }
            });
        }
    });
    let output = Command::new(env!("CARGO_BIN_EXE_koo"))
        .arg("run")
        .arg(&dir)
        .env("REDIS_URL", format!("redis://koo:p%40ss%3Aw%2F@127.0.0.1:{port}/2"))
        .env("KOO_LEAKS", "1")
        .output()
        .expect("koo runs");
    assert!(
        output.status.success() && String::from_utf8_lossy(&output.stdout) == "OK\n",
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        *told.lock().unwrap(),
        ["1: AUTH koo p@ss:w/", "1: SELECT 2", "1: PING", "1: GET k"]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The Redis client of `packages/redis`, which is written in Koo on `std/net`, against a
/// real server: by a program, and by the handlers and an actor of a server with one
/// worker and with several, where every count must come out exact. It needs a Redis,
/// whose address is in `KOO_TEST_REDIS`.
#[test]
fn a_redis_is_spoken_to() {
    let Some(url) = std::env::var("KOO_TEST_REDIS").ok().filter(|url| !url.is_empty()) else {
        eprintln!("KOO_TEST_REDIS is not set: the Redis client is only checked by the compiler");
        return;
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::temp_dir().join(format!("koo-redis-{}", std::process::id()));
    let run = format!("{}", std::process::id());
    // A program that uses the client has it in its own directory, which is its package.
    let package = |name: &str| {
        let at = dir.join(name);
        std::fs::create_dir_all(&at).unwrap();
        std::fs::copy(root.join("packages/redis/redis.koo"), at.join("redis.koo")).unwrap();
        std::fs::copy(
            root.join("tests/redis").join(name).join("main.koo"),
            at.join("main.koo"),
        )
        .unwrap();
        at
    };
    let output = Command::new(env!("CARGO_BIN_EXE_koo"))
        .arg("run")
        .arg(package("plain"))
        .env("REDIS_URL", &url)
        .env("KOO_TEST_RUN", &run)
        .env("KOO_LEAKS", "1")
        .output()
        .expect("koo runs");
    let said = String::from_utf8_lossy(&output.stdout).to_string();
    let expected = "\
none
hello, world: \u{e9}\r
$5
1 42
true false
3
a b c none
none
Refused
Text { value: \"still in step\" }
none | in the other | in the other | 1
Refused: `MULTI` changes what a connection means for whoever has it next, and is not sent with `command`
[Number { value: 43 }, Number { value: 44 }]
Refused
45 true
2
";
    assert!(
        output.status.success() && said == expected,
        "{}\n--- expected\n{expected}--- found\n{said}",
        String::from_utf8_lossy(&output.stderr)
    );

    let executable = dir.join("served-redis");
    let built = Command::new(env!("CARGO_BIN_EXE_koo"))
        .arg("build")
        .arg(package("served"))
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("koo runs");
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    for workers in [1, 3] {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let log = dir.join(format!("log-{workers}.txt"));
        let child = Command::new(&executable)
            .env("REDIS_URL", &url)
            .env("PORT", port.to_string())
            .env("WORKERS", workers.to_string())
            .env("KOO_LEAKS", "1")
            .stdout(std::process::Stdio::null())
            .stderr(std::fs::File::create(&log).unwrap())
            .spawn()
            .expect("the server starts");
        let server = Server(child);
        for _ in 0..200 {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let body = |answer: String| answer.rsplit("\r\n\r\n").next().unwrap_or("").trim().to_string();
        let key = format!("koo-served-{run}-{workers}");
        // A hundred and twenty counts by handlers and sixty by the actor, all at once
        // and on one key: each of the numbers from 1 to 180 is answered once.
        let asked: Vec<_> = (0..180)
            .map(|i| {
                let path = format!("/{}/{key}", if i % 3 == 2 { "tally" } else { "hit" });
                std::thread::spawn(move || get(port, &path))
            })
            .collect();
        let mut counts: Vec<i64> = asked
            .into_iter()
            .map(|answer| {
                let answer = body(answer.join().unwrap());
                answer
                    .parse()
                    .unwrap_or_else(|_| panic!("with {workers} workers a count was answered `{answer}`"))
            })
            .collect();
        counts.sort();
        assert_eq!(counts, (1..=180).collect::<Vec<i64>>(), "with {workers} workers");
        // And what each of sixty asks to have kept is what it is given back.
        let kept: Vec<_> = (0..60)
            .map(|i| std::thread::spawn(move || (i, get(port, &format!("/keep/koo-kept-{port}-{i}/text{i}")))))
            .collect();
        for answer in kept {
            let (i, answer) = answer.join().unwrap();
            assert_eq!(body(answer), format!("text{i}"), "with {workers} workers");
        }
        assert_eq!(body(get(port, &format!("/drop/{key}"))), "1", "with {workers} workers");
        for i in 0..60 {
            get(port, &format!("/drop/koo-kept-{port}-{i}"));
        }
        stop(server, port);
        let written = std::fs::read_to_string(&log).unwrap();
        assert!(
            written.is_empty(),
            "with {workers} workers the server wrote:\n{written}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A database is found by what the environment says of it, where the address leaves it
/// out: a place that hands a program its database in pieces, the password among them,
/// is heard as PostgreSQL's own client hears it. What the address says comes first,
/// and a password from the environment is taken as it is. It needs a database.
#[test]
fn a_database_is_found_by_its_environment() {
    let Some(url) = test_database() else {
        eprintln!("KOO_TEST_POSTGRES is not set: a database found by its environment is not checked");
        return;
    };
    let _alone = DATABASE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    // The parts of `postgres://user:password@host:port/database`.
    let rest = url.split("://").nth(1).unwrap().split('?').next().unwrap();
    let (who, place) = rest.rsplit_once('@').expect("an address with a user");
    let (user, password) = who.split_once(':').unwrap_or((who, ""));
    let (place, database) = place.split_once('/').unwrap_or((place, user));
    let (host, port) = place.rsplit_once(':').unwrap_or((place, "5432"));
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::temp_dir().join(format!("koo-pgenv-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let run = |program: &Path, env: &[(&str, &str)]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_koo"));
        command.arg("run").arg(program);
        for name in [
            "PGHOST",
            "PGPORT",
            "PGUSER",
            "PGPASSWORD",
            "PGDATABASE",
            "PGSSLMODE",
            "ADDRESS",
        ] {
            command.env_remove(name);
        }
        for (name, value) in env {
            command.env(name, value);
        }
        let output = command.output().expect("koo runs");
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    };
    let found = root.join("tests/pgenv/found.koo");
    let pieces = [
        ("PGHOST", host),
        ("PGPORT", port),
        ("PGUSER", user),
        ("PGPASSWORD", password),
        ("PGDATABASE", database),
    ];
    // Every part from the environment, and an address that says nothing.
    assert_eq!(run(&found, &pieces), format!("connected as {user} to {database}\n"));
    // What the address says comes before what the environment says.
    let mut wrong = vec![("ADDRESS", url.as_str())];
    wrong.extend([
        ("PGHOST", "nowhere.invalid"),
        ("PGPORT", "1"),
        ("PGUSER", "nobody"),
        ("PGPASSWORD", "wrong"),
    ]);
    assert_eq!(run(&found, &wrong), format!("connected as {user} to {database}\n"));
    // A port that is none is said, whoever said it.
    let mut odd = pieces.to_vec();
    odd[1] = ("PGPORT", "abc");
    assert_eq!(run(&found, &odd), "Connection: `abc` is not a port\n");
    // A password is taken as it is: one with every sign in it that an address would
    // have to write another way, `%41` among them, which in an address is an `A`.
    let awkward = "p@ss:/w%41rd?&#";
    let role = dir.join("role.koo");
    let made = |statement: &str| {
        std::fs::write(
            &role,
            format!(
                "import std/os\nimport std/sql\nimport std/sql/postgres\n\nfn main() ! sql.Error {{\n    \
                 db := postgres.connect(os.env(\"DATABASE_URL\") else \"\")?\n    \
                 db.exec(\"{statement}\")?\n}}\n"
            ),
        )
        .unwrap();
        let said = run(&role, &[("DATABASE_URL", url.as_str())]);
        assert_eq!(said, "", "{statement}");
    };
    made("drop role if exists koo_env_test");
    made(&format!("create role koo_env_test login password '{awkward}'"));
    let mut other = pieces.to_vec();
    other[2] = ("PGUSER", "koo_env_test");
    other[3] = ("PGPASSWORD", awkward);
    let said = run(&found, &other);
    made("drop role koo_env_test");
    assert_eq!(said, format!("connected as koo_env_test to {database}\n"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A client that leaves before it is answered does not keep what its handler held: a
/// connection of the pool that a slow statement had is given back, a transaction is
/// undone, and nothing is said in the log, since nothing went wrong. It needs a database.
#[test]
fn a_client_that_leaves_gives_back_its_connection() {
    use std::io::Write;
    let Some(url) = book_database() else {
        eprintln!("KOO_TEST_POSTGRES is not set: clients that leave a slow statement are not checked");
        return;
    };
    let _alone = DATABASE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::temp_dir().join(format!("koo-left-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    empty_book_database(&dir, &url).unwrap_or_else(|wrong| panic!("{wrong}"));
    let executable = dir.join("left");
    let built = Command::new(env!("CARGO_BIN_EXE_koo"))
        .arg("build")
        .arg(root.join("tests/http/left.koo"))
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("koo runs");
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    // Asks, and leaves after so long without waiting for the answer.
    let leave = |port: u16, paths: &[String], after: u64| {
        let asked: Vec<_> = paths
            .iter()
            .map(|path| {
                let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("the server accepts");
                write!(stream, "GET {path} HTTP/1.1\r\nHost: test\r\n\r\n").unwrap();
                stream
            })
            .collect();
        std::thread::sleep(std::time::Duration::from_millis(after));
        drop(asked);
    };
    // A server that takes connections and says nothing, for a handler to ask.
    let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let upstream = format!("http://127.0.0.1:{}/", silent.local_addr().unwrap().port());
    let start = |workers: usize, limit: &str, log: &Path| {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let child = Command::new(&executable)
            .env("DATABASE_URL", &url)
            .env("UPSTREAM", &upstream)
            .env("PORT", port.to_string())
            .env("WORKERS", workers.to_string())
            .env("TIMEOUT_MS", limit)
            .stdout(std::process::Stdio::null())
            .stderr(std::fs::File::create(log).unwrap())
            .spawn()
            .expect("the server starts");
        let server = Server(child);
        for _ in 0..200 {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        (server, port)
    };
    let body = |answer: String| answer.rsplit("\r\n\r\n").next().unwrap_or("").trim().to_string();
    let quick_is_quick = |port: u16, workers: usize, after: &str| {
        for _ in 0..4 {
            let started = std::time::Instant::now();
            let got = get(port, "/quick");
            assert!(got.ends_with("\r\n\r\nquick\n"), "with {workers} workers: {got}");
            assert!(
                started.elapsed() < std::time::Duration::from_millis(1500),
                "with {workers} workers, a statement after {after} took {:?}",
                started.elapsed()
            );
        }
    };
    let mut kept = 0;
    for workers in [1, 2] {
        // Clients that leave, of a server that gives its handlers all the time they want.
        let log = dir.join(format!("log-{workers}.txt"));
        let (server, port) = start(workers, "0", &log);
        assert!(
            get(port, "/quick").ends_with("\r\n\r\nquick\n"),
            "with {workers} workers"
        );
        // Six statements of four seconds, in a pool of two for each worker, and all
        // six clients gone: the next to ask is answered at once, and not in four
        // seconds, or eight.
        leave(port, &vec!["/slow/4".to_string(); 6], 300);
        std::thread::sleep(std::time::Duration::from_millis(200));
        quick_is_quick(port, workers, "six clients left");
        // A transaction whose client leaves in the middle of it is undone, now and
        // when its statement would have ended.
        let count = || body(get(port, "/count"));
        assert_eq!(count(), format!("{kept}"), "with {workers} workers");
        leave(port, &[format!("/kept/{workers}/2"), format!("/kept/{workers}/2")], 300);
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert_eq!(count(), format!("{kept}"), "with {workers} workers");
        std::thread::sleep(std::time::Duration::from_millis(2500));
        assert_eq!(count(), format!("{kept}"), "with {workers} workers");
        // And one that is waited for is kept.
        assert_eq!(body(get(port, &format!("/kept/{workers}/0.1"))), "kept");
        kept += 1;
        assert_eq!(count(), format!("{kept}"), "with {workers} workers");
        // Nothing was said of any of it: nothing went wrong.
        let written = std::fs::read_to_string(&log).unwrap();
        assert!(
            written.is_empty(),
            "with {workers} workers the server wrote:\n{written}"
        );
        // What a handler is told whose client has left: a statement it was in the
        // middle of fails as one whose connection was lost, one it begins afterwards
        // as one whose time ran out, and so does a request to another server.
        leave(port, &["/told/4".to_string(), "/via".to_string()], 300);
        std::thread::sleep(std::time::Duration::from_millis(400));
        stop(server, port);
        let written = std::fs::read_to_string(&log).unwrap();
        for said in [
            "in the middle of a statement: Connection",
            "a statement begun afterwards: Timeout",
            "in the middle of a request: Timeout",
        ] {
            assert_eq!(
                written.matches(said).count(),
                1,
                "with {workers} workers the server wrote:\n{written}"
            );
        }

        // Handlers that take longer than the second they are given, of clients that wait.
        let log = dir.join(format!("log-timed-{workers}.txt"));
        let (server, port) = start(workers, "1000", &log);
        let count = || body(get(port, "/count"));
        // In the middle of a statement, of a transaction, and of a request to another
        // server: each client is answered 503 in about a second, whatever answer the
        // handler had of its own.
        let asked: Vec<_> = ["/slow/4", "/told/4", "/via", "/kept/9/4"]
            .iter()
            .map(|path| {
                std::thread::spawn(move || {
                    let started = std::time::Instant::now();
                    (path, get(port, path), started.elapsed().as_millis())
                })
            })
            .collect();
        for answer in asked {
            let (path, answer, took) = answer.join().unwrap();
            assert!(
                answer.starts_with("HTTP/1.1 503 ") && answer.ends_with("\r\n\r\nService Unavailable\n"),
                "with {workers} workers, {path}: {answer}"
            );
            assert!(
                (900..2500).contains(&took),
                "with {workers} workers, {path} took {took} ms"
            );
        }
        // The pool is free, the transaction is undone, and the next is kept.
        quick_is_quick(port, workers, "four handlers ran out of time");
        assert_eq!(count(), format!("{kept}"), "with {workers} workers");
        assert_eq!(body(get(port, &format!("/kept/{workers}/0.1"))), "kept");
        kept += 1;
        std::thread::sleep(std::time::Duration::from_millis(3500));
        assert_eq!(count(), format!("{kept}"), "with {workers} workers");
        stop(server, port);
        let written = std::fs::read_to_string(&log).unwrap();
        for (said, times) in [
            ("the handler was given 1000 ms and had not answered", 4),
            ("in the middle of a statement: Connection", 1),
            ("a statement begun afterwards: Timeout", 1),
            ("in the middle of a request: Timeout", 1),
        ] {
            assert_eq!(
                written.matches(said).count(),
                times,
                "with {workers} workers the server wrote:\n{written}"
            );
        }
        assert_eq!(
            written.lines().count(),
            7,
            "with {workers} workers the server wrote:\n{written}"
        );
    }
    drop(silent);
    let _ = std::fs::remove_dir_all(&dir);
}

/// What the environment says of a server is how many workers it has, and nothing else
/// about it: one worker has a process watching over it as two have, and is started
/// again when it is killed.
#[test]
fn the_environment_says_how_many_workers() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::temp_dir().join(format!("koo-plain-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let executable = dir.join("plain");
    let built = Command::new(env!("CARGO_BIN_EXE_koo"))
        .arg("build")
        .arg(root.join("tests/http/plain.koo"))
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("koo runs");
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let children = |parent: u32| -> Vec<String> {
        let found = Command::new("pgrep")
            .args(["-P", &parent.to_string()])
            .output()
            .unwrap();
        String::from_utf8_lossy(&found.stdout)
            .split_whitespace()
            .map(str::to_string)
            .collect()
    };
    for workers in [1, 2] {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let child = Command::new(&executable)
            .env("PORT", port.to_string())
            .env("WEB_CONCURRENCY", workers.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("the server starts");
        let server = Server(child);
        for _ in 0..200 {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let first = children(server.0.id());
        assert_eq!(
            first.len(),
            workers,
            "with WEB_CONCURRENCY={workers}, the workers are {first:?}"
        );
        // One that is killed is started again, and the server answers meanwhile and after.
        Command::new("kill").args(["-KILL", &first[0]]).status().unwrap();
        let mut now = Vec::new();
        for _ in 0..200 {
            now = children(server.0.id());
            if now.len() == workers && !now.contains(&first[0]) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert!(
            now.len() == workers && !now.contains(&first[0]),
            "with WEB_CONCURRENCY={workers}, after one was killed the workers are {now:?}"
        );
        let mut answer = String::new();
        for _ in 0..40 {
            answer = get(port, "/");
            if answer.ends_with("\r\n\r\nhere\n") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert!(
            answer.ends_with("\r\n\r\nhere\n"),
            "with WEB_CONCURRENCY={workers}: {answer}"
        );
        stop(server, port);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A handler is given so long for a request, thirty seconds unless `serve` is told
/// otherwise: here half a second. One that waits longer is let wait for nothing more
/// and its client is answered 503, the log saying which request; one that computes is
/// not interrupted; an actor's method is never cut short; and the connection goes on.
#[test]
fn a_handler_has_its_time() {
    use std::io::{Read, Write};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::temp_dir().join(format!("koo-deadline-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let executable = dir.join("deadline");
    let built = Command::new(env!("CARGO_BIN_EXE_koo"))
        .arg("build")
        .arg(root.join("tests/http/deadline.koo"))
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("koo runs");
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let start = |workers: usize, limit: &str, log: &Path| {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let child = Command::new(&executable)
            .env("PORT", port.to_string())
            .env("WORKERS", workers.to_string())
            .env("TIMEOUT_MS", limit)
            .stdout(std::process::Stdio::null())
            .stderr(std::fs::File::create(log).unwrap())
            .spawn()
            .expect("the server starts");
        let server = Server(child);
        for _ in 0..200 {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        (server, port)
    };
    let timed = |port: u16, path: &str| {
        let started = std::time::Instant::now();
        let answer = get(port, path);
        (answer, started.elapsed().as_millis())
    };
    for workers in [1, 2] {
        let log = dir.join(format!("log-{workers}.txt"));
        let (server, port) = start(workers, "500", &log);
        let (answer, _) = timed(port, "/wait/100");
        assert!(answer.ends_with("\r\n\r\nwaited\n"), "with {workers} workers: {answer}");
        // A wait of three seconds, and a hundred of a tenth of a second, are both
        // over in half a second.
        for path in ["/wait/3000", "/poll"] {
            let (answer, took) = timed(port, path);
            assert!(
                answer.starts_with("HTTP/1.1 503 ") && answer.ends_with("\r\n\r\nService Unavailable\n"),
                "with {workers} workers, {path}: {answer}"
            );
            assert!(
                (400..2000).contains(&took),
                "with {workers} workers, {path} took {took} ms"
            );
        }
        // A handler that computes for longer than that is not interrupted.
        let (answer, _) = timed(port, "/spin/800");
        assert!(answer.ends_with("\r\n\r\nspun\n"), "with {workers} workers: {answer}");
        // And one that computes for longer and then fails has failed with what it
        // failed with, to its client and in the log: it was never cut short.
        let (answer, _) = timed(port, "/spent/800");
        assert!(answer.starts_with("HTTP/1.1 500 "), "with {workers} workers: {answer}");
        // An actor's method runs to its end whatever becomes of who asked. In one
        // process it runs in the handler, which is then not cut short either; from
        // another process the asker stops waiting a second after its time is over:
        // that second is what a handler is given for an answer that is on its way.
        let (answer, took) = timed(port, "/nap/3000");
        if workers == 1 {
            assert!(answer.ends_with("\r\n\r\n1\n"), "{answer}");
        } else {
            assert!(answer.starts_with("HTTP/1.1 503 "), "with {workers} workers: {answer}");
            assert!(
                (1400..2400).contains(&took),
                "with {workers} workers, an ask took {took} ms"
            );
            std::thread::sleep(std::time::Duration::from_millis(1800));
        }
        let (answer, _) = timed(port, "/naps");
        assert!(answer.ends_with("\r\n\r\n1\n"), "with {workers} workers: {answer}");
        // The connection of a request that took too long goes on to the next.
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut heard = String::new();
        for (path, until) in [("/wait/3000", "Service Unavailable\n"), ("/wait/10", "waited\n")] {
            write!(stream, "GET {path} HTTP/1.1\r\nHost: test\r\n\r\n").unwrap();
            let mut answer = String::new();
            let mut chunk = [0u8; 4096];
            while !answer.ends_with(until) {
                let n = stream.read(&mut chunk).expect("an answer on the same connection");
                assert!(n > 0, "the connection was closed after {heard}");
                answer.push_str(&String::from_utf8_lossy(&chunk[..n]));
            }
            heard.push_str(answer.split("\r\n").next().unwrap_or(""));
            heard.push(';');
        }
        assert_eq!(
            heard, "HTTP/1.1 503 Service Unavailable;HTTP/1.1 200 OK;",
            "with {workers} workers"
        );
        stop(server, port);
        let written = std::fs::read_to_string(&log).unwrap();
        let expected = if workers == 1 { 3 } else { 4 };
        assert_eq!(
            written
                .matches("the handler was given 500 ms and had not answered")
                .count(),
            expected,
            "with {workers} workers the server wrote:\n{written}"
        );
        assert!(written.contains("GET /poll: the handler was given 500 ms"), "{written}");
        assert!(
            written.contains("GET /spent/800: Spent"),
            "with {workers} workers the server wrote:\n{written}"
        );
    }
    // A server that is told to give its handlers no limit gives them none.
    let log = dir.join("log-none.txt");
    let (server, port) = start(2, "0", &log);
    let (answer, _) = timed(port, "/wait/900");
    assert!(answer.ends_with("\r\n\r\nwaited\n"), "{answer}");
    stop(server, port);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A handler that waits does not make the rest of its worker wait: forty requests that
/// each sleep 300 ms are answered by one worker in well under the twelve seconds they
/// would take one after another. And a connection goes on after a request for a route
/// that is not there.
#[test]
fn handlers_wait_side_by_side() {
    use std::io::{Read, Write};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::temp_dir().join(format!("koo-places-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let executable = dir.join("server");
    let built = Command::new(env!("CARGO_BIN_EXE_koo"))
        .arg("build")
        .arg(root.join("tests/client/server.koo"))
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("koo runs");
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let (server, port) = serve(&executable, 1, &dir);
    let started = std::time::Instant::now();
    let answers: Vec<String> = std::thread::scope(|scope| {
        let asked: Vec<_> = (0..40).map(|_| scope.spawn(move || get(port, "/nap"))).collect();
        asked.into_iter().map(|answer| answer.join().unwrap()).collect()
    });
    let took = started.elapsed();
    for answer in &answers {
        assert!(
            answer.starts_with("HTTP/1.1 200 ") && answer.ends_with("rested\n"),
            "{answer}"
        );
    }
    assert!(
        took < std::time::Duration::from_secs(4),
        "forty naps of 300 ms took {took:?}: the handlers waited one after another"
    );
    // One connection, three requests: the second is for no route, and the third must
    // still be answered.
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let mut heard = String::new();
    for path in ["/none", "/nowhere", "/none"] {
        write!(stream, "GET {path} HTTP/1.1\r\nHost: test\r\n\r\n").unwrap();
        let mut chunk = [0u8; 4096];
        let until = if path == "/nowhere" { "Not Found\n" } else { "\r\n\r\n" };
        let mut answer = String::new();
        while !answer.contains(until) {
            let n = stream.read(&mut chunk).expect("an answer on the same connection");
            assert!(n > 0, "the connection was closed after {heard}");
            answer.push_str(&String::from_utf8_lossy(&chunk[..n]));
        }
        heard.push_str(answer.split("\r\n").next().unwrap_or(""));
        heard.push(';');
    }
    assert_eq!(
        heard,
        "HTTP/1.1 204 No Content;HTTP/1.1 404 Not Found;HTTP/1.1 204 No Content;"
    );
    // A handler that runs out of stack is answered with 500, as one that met any other
    // panic is, and the handlers that were waiting beside it are answered too; then
    // the process, which is no longer to be trusted, ends.
    assert!(get(port, "/depth/1000").ends_with("\r\n\r\n1000\n"));
    let (failed, napped) = std::thread::scope(|scope| {
        let napping: Vec<_> = (0..6).map(|_| scope.spawn(move || get(port, "/nap"))).collect();
        std::thread::sleep(std::time::Duration::from_millis(100));
        let failed = get(port, "/depth/100000000");
        let napped: Vec<String> = napping.into_iter().map(|nap| nap.join().unwrap()).collect();
        (failed, napped)
    });
    assert!(
        failed.starts_with("HTTP/1.1 500 ") && failed.ends_with("Internal Server Error\n"),
        "{failed}"
    );
    for nap in &napped {
        assert!(nap.starts_with("HTTP/1.1 200 ") && nap.ends_with("rested\n"), "{nap}");
    }
    let mut server = server;
    let mut ended = None;
    for _ in 0..200 {
        if let Ok(Some(status)) = server.0.try_wait() {
            ended = status.code();
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert_eq!(
        ended,
        Some(101),
        "a server of one worker ends when its handler runs out of stack"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A server that is told to stop answers the requests it has in hand, even those whose
/// handlers still have an actor to ask, which lives in the process that was told: it
/// must go on answering for its actors until its workers are done, and then end, with
/// status 0, and not wait for ever on workers that wait for it.
#[test]
fn a_stopping_server_answers_what_it_has_in_hand() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::temp_dir().join(format!("koo-stopping-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let executable = dir.join("server");
    let built = Command::new(env!("CARGO_BIN_EXE_koo"))
        .arg("build")
        .arg(root.join("tests/client/server.koo"))
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("koo runs");
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    for workers in [1, 3] {
        let (mut server, port) = serve(&executable, workers, &dir);
        // A handler that asks its own server, and reads one type from the request and
        // from the answer.
        let asked = r#"{"method":"GET","path":"/x","body":"","agent":"","kind":"","token":""}"#;
        let got = post(port, "/relay", asked);
        assert!(
            got.ends_with("\r\n\r\n<POST> /x true\n"),
            "with {workers} workers: {got}"
        );
        // An actor's mail has room for what it says, from whichever process it is told:
        // four wait, and a fifth is the one the actor is at.
        let got = get(port, "/try");
        let accepted: i64 = got.rsplit("\r\n\r\n").next().unwrap().trim().parse().unwrap();
        assert!((4..=5).contains(&accepted), "with {workers} workers: {got}");
        std::thread::sleep(std::time::Duration::from_millis(700));
        let got = get(port, "/send");
        assert!(got.ends_with("\r\n\r\n6\n"), "with {workers} workers: {got}");
        // An actor that a handler starts is the whole server's, as one that `main`
        // starts is: whichever worker answers finds it, and its count is one count.
        let counts: Vec<String> = ["a", "a", "b", "a", "a", "b", "a"]
            .iter()
            .map(|name| {
                let got = get(port, &format!("/room/{name}"));
                got.rsplit("\r\n\r\n").next().unwrap().trim().to_string()
            })
            .collect();
        assert_eq!(counts, ["1", "2", "1", "3", "4", "2", "5"], "with {workers} workers");
        // One that is started with what cannot be sent to the process that keeps the
        // actors can be started where there is one process, and is refused in a word
        // where there are several.
        let got = get(port, "/pinned");
        if workers == 1 {
            assert!(got.ends_with("\r\n\r\n3\n"), "{got}");
        } else {
            assert!(got.starts_with("HTTP/1.1 500"), "with {workers} workers: {got}");
        }
        // A type that writes less of itself as JSON than it is goes to an actor whole,
        // in whichever process the actor is, with what it keeps to itself.
        let got = get(port, "/tag");
        assert!(got.ends_with("\r\n\r\n7 \"t\" 0 42\n"), "with {workers} workers: {got}");
        let got = get(port, "/bytes");
        assert!(
            got.ends_with("\r\n\r\n[97, 195, 191]\n"),
            "with {workers} workers: {got}"
        );
        // Bytes inside other things cross as they are too, wherever they are held.
        let got = get(port, "/parcel");
        assert!(
            got.ends_with("\r\n\r\ntrue 256 [120] [104,101,108,100] [true, false, true] [0, 1, 255]\n"),
            "with {workers} workers: {got}"
        );
        for _ in 0..4 {
            let got = get(port, "/after");
            assert!(
                got.ends_with("\r\n\r\n{\"value\":null}"),
                "with {workers} workers: {got}"
            );
        }
        let got = get(port, "/gone");
        assert!(got.ends_with("\r\n\r\nfalse\n"), "with {workers} workers: {got}");
        // With several workers, one that runs out of stack answers 500 and is
        // replaced, and the server goes on: also when it ran out while it was
        // printing, and so cannot go on to answer what else it had in hand.
        if workers > 1 {
            let failed = get(port, "/noisy/100000000");
            assert!(failed.starts_with("HTTP/1.1 500 "), "with {workers} workers: {failed}");
            let got = get(port, "/depth/7");
            assert!(got.ends_with("\r\n\r\n7\n"), "with {workers} workers: {got}");
            let failed = get(port, "/depth/100000000");
            assert!(failed.starts_with("HTTP/1.1 500 "), "with {workers} workers: {failed}");
            let got = get(port, "/depth/7");
            assert!(got.ends_with("\r\n\r\n7\n"), "with {workers} workers: {got}");
        }
        // A float that is not a number is itself when it comes back from an actor,
        // though the actor is in another process.
        for (asked, answer) in [
            ("1&total=2", "0.5"),
            ("1&total=0", "inf"),
            ("-1&total=0", "-inf"),
            ("0&total=0", "NaN"),
        ] {
            let got = get(port, &format!("/ratio?done={asked}"));
            assert!(
                got.ends_with(&format!("\r\n\r\n{answer}\n")),
                "with {workers} workers: {got}"
            );
        }
        // An actor that takes its time is waited for by the handlers that asked it and
        // by nobody else: while four ask one that naps, the worker they are in answers
        // other requests, and the process that keeps the actors answers for the
        // others. Where there is one process the actor is asked in the handler itself.
        if workers > 1 {
            let napping: Vec<_> = (0..4)
                .map(|_| std::thread::spawn(move || get(port, "/sloth/400")))
                .collect();
            std::thread::sleep(std::time::Duration::from_millis(100));
            let started = std::time::Instant::now();
            for _ in 0..20 {
                let got = get(port, "/whoami");
                assert!(
                    got.ends_with("\r\n\r\nnobody, no token"),
                    "with {workers} workers: {got}"
                );
                let got = get(port, "/bytes");
                assert!(
                    got.ends_with("\r\n\r\n[97, 195, 191]\n"),
                    "with {workers} workers: {got}"
                );
            }
            let took = started.elapsed();
            assert!(
                took < std::time::Duration::from_millis(1000),
                "with {workers} workers, forty requests beside an actor that naps took {took:?}"
            );
            let mut naps: Vec<String> = napping
                .into_iter()
                .map(|answer| {
                    let answer = answer.join().unwrap();
                    assert!(answer.starts_with("HTTP/1.1 200 "), "with {workers} workers: {answer}");
                    answer.rsplit("\r\n\r\n").next().unwrap().trim().to_string()
                })
                .collect();
            naps.sort();
            assert_eq!(naps, ["1", "2", "3", "4"], "with {workers} workers");
            // A method that panics costs the handler that asked, which answers 500,
            // and not those that wait for the same actor before it and after it.
            let before = std::thread::spawn(move || get(port, "/sloth/200"));
            std::thread::sleep(std::time::Duration::from_millis(50));
            let tripped = std::thread::spawn(move || get(port, "/trip"));
            std::thread::sleep(std::time::Duration::from_millis(50));
            let after = std::thread::spawn(move || get(port, "/sloth/1"));
            let (before, tripped, after) = (before.join().unwrap(), tripped.join().unwrap(), after.join().unwrap());
            assert!(before.ends_with("\r\n\r\n5\n"), "with {workers} workers: {before}");
            assert!(
                tripped.starts_with("HTTP/1.1 500 "),
                "with {workers} workers: {tripped}"
            );
            // The actor began again when its method panicked.
            assert!(after.ends_with("\r\n\r\n1\n"), "with {workers} workers: {after}");
            // More ask at once than a worker has room for handlers: all are answered,
            // and the server goes on.
            let many: Vec<_> = (0..450)
                .map(|_| std::thread::spawn(move || get(port, "/sloth/1")))
                .collect();
            for answer in many {
                let answer = answer.join().unwrap();
                assert!(answer.starts_with("HTTP/1.1 200 "), "with {workers} workers: {answer}");
            }
            // Messages and answers longer than a socket holds, a hundred at a time.
            let long = "0123456789".repeat(20_000);
            let sent: Vec<_> = (0..100)
                .map(|_| {
                    let long = long.clone();
                    std::thread::spawn(move || post(port, "/through", &long))
                })
                .collect();
            for answer in sent {
                let answer = answer.join().unwrap();
                assert!(
                    answer.ends_with(long.as_str()),
                    "with {workers} workers: {} bytes",
                    answer.len()
                );
            }
            // The first actor a server starts is held up for like any other: while
            // forty wait for it, those who ask another are answered at once. (The
            // forty are two seconds of waiting, one after another, where the machine
            // is quick: each is given five to be answered in, and a machine that
            // makes a wait of 100 ms last 125 would have none to spare with four.)
            let lingering: Vec<_> = (0..40)
                .map(|_| std::thread::spawn(move || get(port, "/linger/50")))
                .collect();
            std::thread::sleep(std::time::Duration::from_millis(50));
            let started = std::time::Instant::now();
            for _ in 0..20 {
                let got = get(port, "/sloth/0");
                assert!(got.starts_with("HTTP/1.1 200 "), "with {workers} workers: {got}");
            }
            let took = started.elapsed();
            assert!(
                took < std::time::Duration::from_millis(1500),
                "with {workers} workers, twenty asks beside the first actor took {took:?}"
            );
            for answer in lingering {
                let answer = answer.join().unwrap();
                assert!(answer.starts_with("HTTP/1.1 200 "), "with {workers} workers: {answer}");
            }
            // A method of an actor that runs out of stack costs the handler that
            // asked, and the server goes on, its actors with it.
            let dived = get(port, "/dive/100000000");
            assert!(dived.starts_with("HTTP/1.1 500 "), "with {workers} workers: {dived}");
            let got = get(port, "/dive/7");
            assert!(got.ends_with("\r\n\r\n7\n"), "with {workers} workers: {got}");
        } else {
            let tripped = get(port, "/trip");
            assert!(
                tripped.starts_with("HTTP/1.1 500 "),
                "with {workers} workers: {tripped}"
            );
            let got = get(port, "/sloth/1");
            assert!(got.ends_with("\r\n\r\n1\n"), "with {workers} workers: {got}");
        }
        let asked: Vec<_> = (0..6)
            .map(|_| std::thread::spawn(move || get(port, "/visit")))
            .collect();
        std::thread::sleep(std::time::Duration::from_millis(150));
        Command::new("kill")
            .args(["-TERM", &server.0.id().to_string()])
            .status()
            .unwrap();
        let mut numbers: Vec<String> = asked
            .into_iter()
            .map(|answer| {
                let answer = answer.join().unwrap();
                assert!(answer.starts_with("HTTP/1.1 200 "), "with {workers} workers: {answer}");
                answer.rsplit("visit ").next().unwrap_or("").trim().to_string()
            })
            .collect();
        numbers.sort();
        assert_eq!(numbers, ["1", "2", "3", "4", "5", "6"], "with {workers} workers");
        let started = std::time::Instant::now();
        let status = loop {
            if let Some(status) = server.0.try_wait().unwrap() {
                break status;
            }
            assert!(
                started.elapsed() < std::time::Duration::from_secs(10),
                "with {workers} workers the server did not stop"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        assert!(
            status.success(),
            "with {workers} workers the server ended with {status}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// `http.get` and the rest ask a real server, which is `tests/client/server.koo`, and
/// `tests/client/client.koo` prints what it says it prints.
#[test]
fn a_program_asks_another_server() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::temp_dir().join(format!("koo-client-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let executable = dir.join("server");
    let built = Command::new(env!("CARGO_BIN_EXE_koo"))
        .arg("build")
        .arg(root.join("tests/client/server.koo"))
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("koo runs");
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let (server, port) = serve(&executable, 2, &dir);
    let client = root.join("tests/client/client.koo");
    let expected = expectation(&std::fs::read_to_string(&client).unwrap());
    let mut failures = Vec::new();
    for release in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_koo"));
        command
            .arg("run")
            .arg(&client)
            .env("KOO_LEAKS", "1")
            .env("PORT", port.to_string());
        // The server is asked at an address of the new kind where the machine has one.
        if std::net::TcpListener::bind("[::1]:0").is_ok() {
            command.env("KOO_TEST_V6", "1");
        }
        if release {
            command.arg("--release");
        }
        let output = command.output().expect("koo runs");
        // The port is different every time, and is not what is being tested.
        let stdout = String::from_utf8_lossy(&output.stdout).replace(&format!(":{port}"), ":PORT");
        if stdout != expected.stdout || !output.status.success() {
            failures.push(format!(
                "release {release}: {}\n--- expected\n{}--- found\n{stdout}--- stderr\n{}",
                output.status,
                expected.stdout,
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }
    stop(server, port);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

/// With `KOO_LOG=json`, everything a server writes on the standard error output is a
/// line of JSON: what the program logs, the log of requests, a handler's failure, a panic.
#[test]
fn a_servers_lines_are_json() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::temp_dir().join(format!("koo-logged-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let executable = dir.join("server");
    let built = Command::new(env!("CARGO_BIN_EXE_koo"))
        .arg("build")
        .arg(root.join("tests/http/logged.koo"))
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("koo runs");
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let log = dir.join("log.txt");
    let child = Command::new(&executable)
        .env("PORT", port.to_string())
        .env("KOO_LOG", "json")
        .env("KOO_LOG_TIME", "off")
        .stdout(std::process::Stdio::null())
        .stderr(std::fs::File::create(&log).unwrap())
        .spawn()
        .expect("the server starts");
    let server = Server(child);
    for _ in 0..200 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    for path in ["/hello/Ada", "/fail", "/panic", "/nowhere"] {
        get(port, path);
    }
    stop(server, port);
    // How long a request took is different every time.
    let mut found = String::new();
    for line in std::fs::read_to_string(&log).unwrap().lines() {
        let line = match (line.find("\"ms\":"), line.ends_with('}')) {
            (Some(at), true) => format!("{}\"ms\":0}}", &line[..at]),
            _ => line.to_string(),
        };
        // And where the program is, on every machine.
        let source = root.join("tests/http/logged.koo").display().to_string();
        found.push_str(&line.replace(&source, "logged.koo"));
        found.push('\n');
    }
    let expected_path = root.join("tests/http/logged.json");
    let expected = std::fs::read_to_string(&expected_path).unwrap_or_default();
    if found != expected && std::env::var_os("KOO_BLESS").is_some() {
        std::fs::write(&expected_path, &found).unwrap();
    } else {
        assert_eq!(found, expected, "what the server wrote on the standard error output");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_server_answers_requests() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::temp_dir().join(format!("koo-http-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let executable = dir.join("server");
    let built = Command::new(env!("CARGO_BIN_EXE_koo"))
        .arg("build")
        .arg(root.join("tests/http/server.koo"))
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("koo runs");
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));

    // What the server has to serve as files.
    let files = dir.join("public");
    std::fs::create_dir_all(files.join("css")).unwrap();
    std::fs::write(files.join("index.html"), "<h1>index</h1>").unwrap();
    std::fs::write(files.join("css/site.css"), "body { margin: 0 }").unwrap();
    std::fs::create_dir_all(files.join("docs")).unwrap();
    std::fs::write(files.join("docs/index.html"), "<h1>docs</h1>").unwrap();
    std::fs::write(files.join("blob.bin"), [0u8, 159, 146, 150]).unwrap();
    std::fs::write(dir.join("secret.txt"), "not to be served").unwrap();
    // A link that stays in the directory, and two that lead out of it.
    std::os::unix::fs::symlink("css", files.join("styles")).unwrap();
    std::os::unix::fs::symlink("../secret.txt", files.join("leak.txt")).unwrap();
    std::os::unix::fs::symlink("..", files.join("up")).unwrap();

    let (child, port) = serve(&executable, 1, &files);
    let has = |answer: &str, parts: &[&str]| {
        for part in parts {
            assert!(answer.contains(part), "no {part:?} in:\n{answer}");
        }
    };
    // Routes, parameters, queries, headers and bodies.
    has(
        &get(port, "/"),
        &["HTTP/1.1 200 OK", "Content-Length: 13", "\r\n\r\nHello, world!"],
    );
    has(&get(port, "/home"), &["text/html", "<h1>Koo</h1>"]);
    has(&get(port, "/hello/Ada%20L"), &["Hello, Ada L!"]);
    has(&get(port, "/hello/Ada/"), &["Hello, Ada!"]);
    has(&get(port, "/search?q=a+b%21"), &["you searched for a b! with unknown"]);
    // The name of a parameter is read as its value is: `%5B` is `[`, and `+` a space.
    has(
        &get(port, "/filter?filter%5Bname%5D=ada&first+name=Ada+L"),
        &["\r\n\r\nada, Ada L"],
    );
    has(
        &get(port, "/filter?filter%5Bname=x&first%2Bname=y"),
        &["\r\n\r\nnone, none"],
    );
    has(
        &http(
            port,
            "POST /signup HTTP/1.1\r\nConnection: close\r\nContent-Type: application/x-www-form-urlencoded\r\n\
             Content-Length: 43\r\n\r\nuser%5Bemail%5D=a%40b.example&full+name=Ada",
        ),
        &["\r\n\r\na@b.example, Ada"],
    );
    has(&get(port, "/orders/2"), &["200 OK", "order 2"]);
    has(
        &get(port, "/orders/x"),
        &["400 Bad Request", "an order's id is a number"],
    );
    has(&get(port, "/old"), &["302 Found", "Location: /\r\n"]);
    has(&get(port, "/files/a/b.txt"), &["file /files/a/b.txt"]);
    has(
        &http(port, "DELETE /any HTTP/1.1\r\nConnection: close\r\n\r\n"),
        &["200 OK", "DELETE"],
    );
    has(
        &post(port, "/echo", "some body"),
        &["201 Created", "X-Length: 9", "\r\n\r\nsome body"],
    );
    // No route, and a handler that fails: the client is not told why.
    has(&get(port, "/missing"), &["404 Not Found"]);
    let failed = get(port, "/orders/9");
    has(&failed, &["500 Internal Server Error"]);
    assert!(!failed.contains("NotFound"), "{failed}");
    // JSON in and out, and a body that does not fit its type.
    has(
        &post(
            port,
            "/orders",
            r#"{"customer": "Bob", "items": [{"sku": "x"}, {"sku": "y", "qty": 3, "note": "n"}]}"#,
        ),
        &[
            "201 Created",
            "Content-Type: application/json",
            r#"{"id":8,"customer":"Bob","items":[{"sku":"x","qty":1},{"sku":"y","qty":3,"note":"n"}]}"#,
        ],
    );
    has(
        &post(
            port,
            "/orders",
            r#"{"customer": "Bob", "items": [{"sku": "x", "qty": "many"}]}"#,
        ),
        &[
            "400 Bad Request",
            r#""path":"/items/0/qty""#,
            "expected a whole number, found a string",
        ],
    );
    has(
        &post(port, "/orders", "not json"),
        &["400 Bad Request", "expected an object"],
    );
    // A body that is not text is had as its bytes, and bytes are sent as they are.
    let blob = [0u8, 159, 146, 150, 255, 10, 0];
    let posted = |path: &str| {
        let mut request = format!(
            "POST {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
            blob.len()
        )
        .into_bytes();
        request.extend_from_slice(&blob);
        http_bytes(port, &request)
    };
    let said = String::from_utf8_lossy(&posted("/hex")).to_string();
    has(&said, &["\r\n\r\n7 009f9296ff0a00 "]);
    let mirrored = posted("/mirror");
    has(
        &String::from_utf8_lossy(&mirrored),
        &["201 Created", "Content-Type: image/png\r\n", "Content-Length: 7\r\n"],
    );
    assert!(mirrored.ends_with(&blob), "{mirrored:?}");
    // An end of line in a type does not begin another header, and a header has a name.
    let injected = get(port, "/as?type=text/plain%0d%0aSet-Cookie:%20session=attacker");
    has(&injected, &["Content-Type: text/plainSet-Cookie: session=attacker\r\n"]);
    assert!(
        !injected.contains("\r\nSet-Cookie") && !injected.contains("\r\n: nameless"),
        "{injected}"
    );
    // A name is what it is once read: `a%2Bb` is `a+b`, and `a+b` is `a b`.
    has(
        &get(port, "/names?a%2Bb=plus&a+b=space"),
        &["\r\n\r\nplus, space, none"],
    );
    has(&get(port, "/names"), &["\r\n\r\nnone, none, none"]);
    has(&get(port, "/names?&&a+b=1"), &["\r\n\r\nnone, 1, none"]);
    // The server listens at addresses of both kinds, where the machine has both.
    if std::net::TcpListener::bind("[::1]:0").is_ok() {
        use std::io::{Read, Write};
        let mut stream = std::net::TcpStream::connect(("::1", port)).expect("the server listens at ::1");
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        stream
            .write_all(b"GET /self/six HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut answer = String::new();
        let _ = stream.read_to_string(&mut answer);
        has(&answer, &["200 OK", "\r\n\r\nGET /self/six"]);
    }
    // A request as text is its method and its path.
    has(&get(port, "/self/ada?x=1"), &["\r\n\r\nGET /self/ada"]);
    // A header that a handler sets and the server writes too is there once.
    let typed = get(port, "/typed");
    has(
        &typed,
        &["Content-Type: text/csv\r\n", "Content-Length: 4\r\n", "X-Rows: 1\r\n"],
    );
    assert_eq!(typed.to_lowercase().matches("content-type").count(), 1, "{typed}");
    assert_eq!(typed.to_lowercase().matches("content-length").count(), 1, "{typed}");
    // A list for a body, and a map; the path to a mistake begins at its place in them.
    has(
        &post(port, "/batch", r#"[{"sku": "x"}, {"sku": "y", "qty": 3}]"#),
        &["200 OK", "Content-Type: application/json", "\r\n\r\n[1,3]"],
    );
    has(
        &post(port, "/batch", r#"[{"sku": "x"}, {"sku": "y", "qty": "many"}]"#),
        &[
            "400 Bad Request",
            r#""path":"/1/qty""#,
            "expected a whole number, found a string",
        ],
    );
    has(
        &post(port, "/batch", r#"{"sku": "x"}"#),
        &["400 Bad Request", "expected an array"],
    );
    has(
        &post(port, "/stock", r#"{"nails": 40, "screws": null, "glue": 2}"#),
        &["200 OK", "\r\n\r\n3 named, 2 counted"],
    );
    has(
        &post(port, "/stock", r#"{"nails": "some"}"#),
        &["400 Bad Request", r#""path":"/nails""#],
    );
    // A response to HEAD has the headers of the response to GET, and no body.
    let head = http(port, "HEAD / HTTP/1.1\r\nConnection: close\r\n\r\n");
    has(&head, &["200 OK", "Content-Length: 13"]);
    assert!(head.ends_with("\r\n\r\n"), "{head}");
    // Two requests sent at once on one connection are both answered, in order.
    let both = http(
        port,
        "GET /hello/a HTTP/1.1\r\n\r\nGET /hello/b HTTP/1.1\r\nConnection: close\r\n\r\n",
    );
    has(&both, &["Hello, a!", "Hello, b!"]);
    assert_eq!(both.matches("HTTP/1.1 200 OK").count(), 2, "{both}");
    // What cannot be trusted is refused.
    has(&http(port, "GARBAGE\r\n\r\n"), &["400 Bad Request"]);
    has(&http(port, "GET / HTTP/1.1\r\nHost : a\r\n\r\n"), &["400 Bad Request"]);
    has(
        &http(
            port,
            "POST /echo HTTP/1.1\r\nContent-Length: 3\r\nContent-Length: 5\r\n\r\nabcde",
        ),
        &["400 Bad Request"],
    );
    has(
        &http(
            port,
            "POST /echo HTTP/1.1\r\nContent-Length: 3\r\nTransfer-Encoding: chunked\r\n\r\nabc",
        ),
        &["400 Bad Request"],
    );
    has(
        &http(port, "POST /echo HTTP/1.1\r\nTransfer-Encoding: gzip\r\n\r\nabc"),
        &["501 Not Implemented"],
    );
    // A body sent in chunks is put together, whatever the pieces it arrives in.
    let chunked = "POST /echo HTTP/1.1\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n\
                   5\r\nhello\r\n1;note=x\r\n,\r\nA\r\n chunked b\r\n3\r\nody\r\n0\r\nTrailer: x\r\n\r\n";
    has(
        &http(port, chunked),
        &["201 Created", "X-Length: 19", "\r\n\r\nhello, chunked body"],
    );
    {
        use std::io::{Read, Write};
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        for piece in chunked.as_bytes().chunks(3) {
            stream.write_all(piece).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let mut answer = String::new();
        let _ = stream.read_to_string(&mut answer);
        has(&answer, &["201 Created", "hello, chunked body"]);
    }
    has(
        &http(
            port,
            "POST /echo HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\nab\r\n0\r\n\r\n",
        ),
        &["400 Bad Request"],
    );
    // A path that has a route, but not for the method.
    has(&post(port, "/", "x"), &["405 Method Not Allowed", "Allow: GET\r\n"]);
    // Files: the type by the name, the index of a directory, and nothing outside it.
    has(&get(port, "/static"), &["200 OK", "text/html", "<h1>index</h1>"]);
    has(&get(port, "/static/css/site.css"), &["text/css", "body { margin: 0 }"]);
    // A directory inside is answered with its own `index.html`, if it has one.
    has(&get(port, "/static/docs/"), &["200 OK", "text/html", "<h1>docs</h1>"]);
    has(&get(port, "/static/docs"), &["200 OK", "text/html", "<h1>docs</h1>"]);
    has(&get(port, "/static/css/"), &["404 Not Found"]);
    has(
        &get(port, "/static/blob.bin"),
        &["application/octet-stream", "Content-Length: 4\r\n"],
    );
    has(&get(port, "/static/missing.txt"), &["404 Not Found"]);
    // What is served is what is really in the directory: a link is followed while it
    // stays inside, and leads to nothing when it leads out.
    has(&get(port, "/static/styles/site.css"), &["200 OK", "body { margin: 0 }"]);
    has(
        &get(port, "/static/up/public/index.html"),
        &["200 OK", "<h1>index</h1>"],
    );
    for leading_out in ["/static/leak.txt", "/static/up/secret.txt"] {
        let answer = get(port, leading_out);
        assert!(
            answer.starts_with("HTTP/1.1 404 ") && !answer.contains("not to be served"),
            "{leading_out}: {answer}"
        );
    }
    for climbing in [
        "/static/../secret.txt",
        "/static/%2e%2e/secret.txt",
        "/static/css/..%2f..%2fsecret.txt",
    ] {
        let answer = get(port, climbing);
        assert!(!answer.contains("not to be served"), "{climbing}:\n{answer}");
        has(&answer, &["404 Not Found"]);
    }
    // A form and a cookie.
    has(
        &http(
            port,
            "POST /form HTTP/1.1\r\nConnection: close\r\nContent-Type: application/x-www-form-urlencoded\r\n\
             Content-Length: 20\r\n\r\nname=Ada+L%21&age=36",
        ),
        &["name is Ada L!, age is 36"],
    );
    has(
        &get(port, "/cookie"),
        &[
            "seen never",
            "Set-Cookie: visit=once%20more; Path=/; HttpOnly; SameSite=Lax; Max-Age=60",
        ],
    );
    has(
        &http(
            port,
            "GET /cookie HTTP/1.1\r\nConnection: close\r\nCookie: a=1; visit=twice\r\n\r\n",
        ),
        &["seen twice"],
    );
    has(
        &http(port, "POST /echo HTTP/1.1\r\nContent-Length: 99999999\r\n\r\n"),
        &["413 Content Too Large"],
    );
    has(
        &http(port, &format!("GET / HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(20000))),
        &["431 Request Header Fields Too Large"],
    );
    has(
        &http(port, "GET /hello/%ff%fe HTTP/1.1\r\nConnection: close\r\n\r\n"),
        &["200 OK", "\u{fffd}"],
    );
    stop(child, port);

    // The same server on three processes answers the same, and all three stop with it.
    let (child, port) = serve(&executable, 3, &files);
    for i in 0..60 {
        has(&get(port, &format!("/hello/{i}")), &[&format!("Hello, {i}!")]);
    }
    // An actor is one, however many processes ask it: the count misses nothing.
    for i in 1..=40 {
        has(&get(port, "/count"), &[&format!("\r\n\r\ncount {i}")]);
    }
    // A handler that panics fails its own request, with 500, and nothing else: the
    // next request is answered at once, by the same workers.
    for _ in 0..40 {
        let answer = get(port, "/panic");
        assert!(answer.starts_with("HTTP/1.1 500 "), "{answer}");
    }
    for i in 0..60 {
        has(&get(port, &format!("/hello/{i}")), &[&format!("Hello, {i}!")]);
    }
    stop(child, port);
    let _ = std::fs::remove_dir_all(&dir);
}
