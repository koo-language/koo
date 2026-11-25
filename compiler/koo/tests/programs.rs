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
    stop(server, port);
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
        let got = get(port, "/gone");
        assert!(got.ends_with("\r\n\r\nfalse\n"), "with {workers} workers: {got}");
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
    std::fs::write(files.join("blob.bin"), [0u8, 159, 146, 150]).unwrap();
    std::fs::write(dir.join("secret.txt"), "not to be served").unwrap();

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
    has(
        &get(port, "/static/blob.bin"),
        &["application/octet-stream", "Content-Length: 4\r\n"],
    );
    has(&get(port, "/static/missing.txt"), &["404 Not Found"]);
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
