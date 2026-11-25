//! The `koo` command.
//!
//! Section 12.1 of the specification lists the commands `koo` will have. The ones that
//! exist so far are `build`, `run`, `check` and `parse`.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use koo_syntax::{Diagnostic, SourceMap, dump, parse_file};

/// The support library, built by `build.rs` and linked into every program.
static RUNTIME: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/libkoo_rt.a"));

const USAGE: &str = "\
Usage: koo <command> [options] <file.koo | directory>

A directory is a package: its `.koo` files are one program.

Commands:
  run       Build a program and run it; arguments after `--` go to the program
  build     Build a program; -o <path> names the executable
  test      Run the tests of a file: its functions named `test_...`
  check     Check a program without building it
  parse     Check the syntax of source files; --tree also prints the syntax tree

Options for run and build:
  --release      Optimize (clang -O3); the default is a quick unoptimized build
  --emit-llvm    Write the LLVM IR next to the source, as <file>.ll, and stop
  --sanitize     Build with AddressSanitizer, to check the compiler's own output;
                 --sanitize=thread builds with ThreadSanitizer, to check that
                 nothing is shared between threads that should not be
  --static       Link everything in, so that the program runs on any Linux of its
                 architecture, including an empty container

  -h, --help       Print this help
  -V, --version    Print the version
";

#[cfg(unix)]
unsafe extern "C" {
    fn signal(signum: i32, handler: usize) -> usize;
}

fn main() -> ExitCode {
    // The checker and the code generator follow the shape of the program, so a deeply
    // nested one needs a deep stack. The main thread's is whatever the system gives; this
    // one is the compiler's to choose, and the memory is used only if it is needed.
    const STACK: usize = 1 << 30;
    match std::thread::Builder::new().stack_size(STACK).spawn(run) {
        Ok(thread) => thread.join().unwrap_or(ExitCode::FAILURE),
        Err(_) => run(),
    }
}

fn run() -> ExitCode {
    // Writing to a pipe that was closed (`koo run prog.koo | head`) ends the process
    // quietly, as it does for any command-line tool, and the same goes for the programs
    // `koo run` starts. Rust's start-up code ignores the signal instead.
    #[cfg(unix)]
    unsafe {
        const SIGPIPE: i32 = 13;
        const SIG_DFL: usize = 0;
        signal(SIGPIPE, SIG_DFL);
    }
    // The program's own arguments, after `--`, go to it exactly as they came, whatever
    // bytes they hold; everything before is the compiler's and is read as text.
    let raw: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let split = raw.iter().position(|arg| arg == "--").unwrap_or(raw.len());
    let args: Vec<String> = raw[..split]
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let program_args: &[std::ffi::OsString] = if split < raw.len() { &raw[split + 1..] } else { &[] };
    let passes_args = split < raw.len();
    match args.first().map(String::as_str) {
        Some("parse") => parse(&args[1..]),
        Some("check") => check(&args[1..]),
        Some("run") => build(&args[1..], Mode::Run, program_args),
        Some(command) if passes_args => {
            eprintln!("koo {command}: `--` and the arguments after it are for `koo run`");
            ExitCode::from(2)
        }
        Some("build") => build(&args[1..], Mode::Build, &[]),
        Some("test") => build(&args[1..], Mode::Test, &[]),
        Some("-h" | "--help" | "help") => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some("-V" | "--version" | "version") => {
            println!("koo {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("koo: unknown command `{other}`\n\n{USAGE}");
            ExitCode::from(2)
        }
        None => {
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// Whether standard error takes colors: it is a terminal, and the environment does not say
/// otherwise (`NO_COLOR` turns them off, `CLICOLOR_FORCE` turns them on).
fn colors() -> bool {
    use std::io::IsTerminal;
    let set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty() && value != "0");
    if set("NO_COLOR") {
        return false;
    }
    set("CLICOLOR_FORCE")
        || (std::io::stderr().is_terminal() && std::env::var_os("TERM").is_none_or(|term| term != "dumb"))
}

fn report(sources: &SourceMap, diagnostics: &[Diagnostic]) {
    let color = colors();
    for diagnostic in diagnostics {
        eprintln!("{}", sources.render_with(diagnostic, color));
    }
    let count = diagnostics.len();
    eprintln!("{count} error{}", if count == 1 { "" } else { "s" });
}

fn read(path: &str) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(source) => Some(source),
        Err(err) => {
            eprintln!("koo: cannot read {path}: {err}");
            None
        }
    }
}

/// The source files a path names: the file, or every `.koo` file of the directory, which
/// is a package (Section 8.1). Test files, `*_test.koo`, belong to `koo test` alone.
fn sources(path: &str, tests: bool) -> Option<SourceMap> {
    let location = Path::new(path);
    if !location.is_dir() {
        return Some(SourceMap::single(path, &read(path)?));
    }
    let entries = match std::fs::read_dir(location) {
        Ok(entries) => entries,
        Err(err) => {
            eprintln!("koo: cannot read {path}: {err}");
            return None;
        }
    };
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.ends_with(".koo") && !name.starts_with('.'))
        .filter(|name| tests || !name.ends_with("_test.koo"))
        .collect();
    names.sort();
    if names.is_empty() {
        eprintln!("koo: there are no `.koo` files in {path}");
        return None;
    }
    let mut files = Vec::new();
    for name in names {
        let file = location.join(&name).to_string_lossy().into_owned();
        let source = read(&file)?;
        files.push((file, source));
    }
    Some(SourceMap::new(files))
}

fn parse(args: &[String]) -> ExitCode {
    let tree = args.iter().any(|a| a == "--tree");
    let paths: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    if let Some(unknown) = args.iter().find(|a| a.starts_with("--") && *a != "--tree") {
        eprintln!("koo parse: unknown option `{unknown}`");
        return ExitCode::from(2);
    }
    if paths.is_empty() {
        eprintln!("koo parse: no files given\n\n{USAGE}");
        return ExitCode::from(2);
    }
    let mut failed = false;
    for path in paths {
        let Some(source) = read(path) else {
            failed = true;
            continue;
        };
        let parsed = parse_file(&source);
        if !parsed.diagnostics.is_empty() {
            report(&SourceMap::single(path, &source), &parsed.diagnostics);
            failed = true;
        }
        if tree {
            print!("{}", dump::file(&parsed.file));
        }
    }
    if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

fn check(args: &[String]) -> ExitCode {
    let [path] = args else {
        eprintln!("koo check: expected one file or directory\n\n{USAGE}");
        return ExitCode::from(2);
    };
    let Some(sources) = sources(path, false) else {
        return ExitCode::FAILURE;
    };
    match koo_compiler::compile_sources(&sources, false) {
        Ok(_) => ExitCode::SUCCESS,
        Err(diagnostics) => {
            report(&sources, &diagnostics);
            ExitCode::FAILURE
        }
    }
}

/// A directory for the files of one build, which is removed when the build is over: a
/// build leaves nothing behind but the program it was asked for. Each build has its own,
/// so that two builds at once do not write over each other.
struct WorkDir {
    path: PathBuf,
    /// Set when the files are wanted after all, to look into what went wrong.
    keep: bool,
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

/// The program that `koo run` is running, for the signals that are passed on to it.
static RUNNING: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

unsafe extern "C" {
    fn kill(process: i32, signal: i32) -> i32;
}

extern "C" fn pass_on(stop: i32) {
    let running = RUNNING.load(std::sync::atomic::Ordering::Relaxed);
    if running > 0 {
        unsafe { kill(running, stop) };
    }
}

fn work_dir() -> std::io::Result<WorkDir> {
    let temp = std::env::temp_dir();
    // A build that was killed could not remove its directory: what is left of builds
    // of more than a day ago is removed now.
    if let Ok(entries) = std::fs::read_dir(&temp) {
        for entry in entries.flatten() {
            let old = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age.as_secs() > 24 * 60 * 60);
            if old && entry.file_name().to_string_lossy().starts_with("koo-build-") {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    let started = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    let mut attempt = 0;
    loop {
        let path = temp.join(format!("koo-build-{}-{started}-{attempt}", std::process::id()));
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(WorkDir { path, keep: false }),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists && attempt < 100 => attempt += 1,
            Err(err) => return Err(err),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Build,
    Run,
    Test,
}

fn build(args: &[String], mode: Mode, program_args: &[std::ffi::OsString]) -> ExitCode {
    let run = mode != Mode::Build;
    let command = match mode {
        Mode::Build => "build",
        Mode::Run => "run",
        Mode::Test => "test",
    };
    let mut release = false;
    let mut emit_llvm = false;
    // The sanitizer to build with: "address" or "thread".
    let mut sanitize: Option<&str> = None;
    let mut fixed = false;
    let mut output: Option<PathBuf> = None;
    let mut path: Option<&String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--release" => release = true,
            "--emit-llvm" => emit_llvm = true,
            "--sanitize" | "--sanitize=address" => sanitize = Some("address"),
            "--sanitize=thread" => sanitize = Some("thread"),
            "--static" => fixed = true,
            "-o" if !run && i + 1 < args.len() => {
                i += 1;
                output = Some(PathBuf::from(&args[i]));
            }
            option if option.starts_with('-') => {
                eprintln!("koo {command}: unknown option `{option}`");
                return ExitCode::from(2);
            }
            _ if path.is_none() => path = Some(&args[i]),
            other => {
                eprintln!("koo {command}: unexpected argument `{other}`");
                return ExitCode::from(2);
            }
        }
        i += 1;
    }
    let Some(path) = path else {
        eprintln!("koo {command}: no file given\n\n{USAGE}");
        return ExitCode::from(2);
    };
    let Some(sources) = sources(path, mode == Mode::Test) else {
        return ExitCode::FAILURE;
    };
    let ir = match koo_compiler::compile_sources(&sources, mode == Mode::Test) {
        Ok(ir) => ir,
        Err(diagnostics) => {
            report(&sources, &diagnostics);
            return ExitCode::FAILURE;
        }
    };
    let source_path = Path::new(path);
    // A package is named after its directory.
    let stem = if source_path.is_dir() {
        source_path
            .canonicalize()
            .ok()
            .and_then(|full| full.file_name().map(|name| name.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "program".to_string())
    } else {
        source_path
            .file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "program".to_string())
    };
    let stem = stem.as_str();
    if emit_llvm {
        let out = if source_path.is_dir() {
            source_path.join(stem).with_extension("ll")
        } else {
            source_path.with_extension("ll")
        };
        if let Err(err) = std::fs::write(&out, ir) {
            eprintln!("koo: cannot write {}: {err}", out.display());
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }

    // AddressSanitizer instruments only functions that ask for it.
    let ir = if let Some(sanitizer) = sanitize {
        let mut ir: String = ir
            .lines()
            .map(|line| match line.strip_suffix(" {") {
                Some(head) if line.starts_with("define ") => format!("{head} sanitize_{sanitizer} {{\n"),
                _ => format!("{line}\n"),
            })
            .collect();
        if sanitizer == "thread" {
            // The support library is not instrumented, so it tells the sanitizer where
            // it hands something from one thread to another (`koo-rt/src/actor.rs`).
            // And a count is read in a way the sanitizer takes for the hand-over it is.
            ir = ir.replace(" unordered, align 8", " acquire, align 8").replacen(
                "  call void @koo_init(i32 %argc, ptr %argv)\n",
                "  call void @koo_init(i32 %argc, ptr %argv)\n  \
                 call void @koo_sync_hooks(ptr @__tsan_acquire, ptr @__tsan_release)\n",
                1,
            );
            ir.push_str(
                "declare void @koo_sync_hooks(ptr, ptr)\ndeclare void @__tsan_acquire(ptr)\n\
                 declare void @__tsan_release(ptr)\n",
            );
        }
        ir
    } else {
        ir
    };
    // How a program loads a library, which the support library does for TLS
    // (`koo-rt/src/tls.rs`): by the system's loader, or, in a program that is linked
    // statically and has none, not at all.
    let mut ir = ir;
    ir.push_str(if fixed {
        "define ptr @koo_dlopen(ptr %name) {\n  ret ptr null\n}\n\
         define ptr @koo_dlsym(ptr %library, ptr %name) {\n  ret ptr null\n}\n"
    } else {
        "declare ptr @dlopen(ptr, i32)\ndeclare ptr @dlsym(ptr, ptr)\n\
         define ptr @koo_dlopen(ptr %name) {\n  %library = call ptr @dlopen(ptr %name, i32 2)\n  ret ptr %library\n}\n\
         define ptr @koo_dlsym(ptr %library, ptr %name) {\n  %found = call ptr @dlsym(ptr %library, ptr %name)\n  ret ptr %found\n}\n"
    });
    let prepared = work_dir().and_then(|dir| {
        let ir_path = dir.path.join("program.ll");
        let runtime = dir.path.join("libkoo_rt.a");
        std::fs::write(&ir_path, &ir)?;
        std::fs::write(&runtime, RUNTIME)?;
        Ok((dir, ir_path, runtime))
    });
    let (mut dir, ir_path, runtime) = match prepared {
        Ok(paths) => paths,
        Err(err) => {
            eprintln!("koo: cannot prepare the build directory: {err}");
            return ExitCode::FAILURE;
        }
    };
    let executable = match (&output, run) {
        (Some(output), _) => output.clone(),
        (None, true) => dir.path.join(stem),
        (None, false) => PathBuf::from(stem),
    };
    // LLVM does the optimizing and the linking, through the system's clang (Section 13.2).
    if fixed && cfg!(target_os = "macos") {
        eprintln!("koo {command}: macOS does not link a program statically; `--static` is for Linux");
        return ExitCode::from(2);
    }
    // `KOO_CLANG` names another clang than the one on the path.
    let compiler = std::env::var("KOO_CLANG").unwrap_or_else(|_| "clang".to_string());
    let mut clang = Command::new(&compiler);
    if let Some(sanitizer) = sanitize {
        clang.arg(format!("-fsanitize={sanitizer}")).arg("-g");
    }
    if fixed {
        clang.arg("-static");
    }
    // The actors of a program run on threads (`koo-rt/src/thread.rs`).
    if !cfg!(target_os = "macos") {
        clang.arg("-pthread");
    }
    let loads = !fixed && !cfg!(target_os = "macos");
    let status = clang
        .arg(if release { "-O3" } else { "-O0" })
        .arg("-Wno-override-module")
        .arg(&ir_path)
        .arg(&runtime)
        .args(loads.then_some("-ldl"))
        .arg("-o")
        .arg(&executable)
        .status();
    match status {
        Ok(status) if status.success() => {}
        Ok(_) => {
            // The code is written for LLVM 15 and later, and an older clang cannot read it.
            let version = Command::new(&compiler).arg("--version").output().ok();
            let version = version.map(|output| String::from_utf8_lossy(&output.stdout).to_string());
            let major = version.as_deref().and_then(|text| {
                let number = text.split("version ").nth(1)?;
                number.split('.').next()?.trim().parse::<u32>().ok()
            });
            match major {
                Some(major) if major < 15 && !version.unwrap_or_default().contains("Apple") => eprintln!(
                    "koo: `{compiler}` is clang {major}, and Koo needs clang 15 or newer. Install a newer one, \
                     and if it has another name, such as `clang-19`, say so with KOO_CLANG=clang-19."
                ),
                _ if fixed => eprintln!(
                    "koo: the program could not be linked statically. That needs the static C library: \
                     `libc6-dev` on Debian and Ubuntu, `glibc-static` on Fedora, `musl-dev` on Alpine."
                ),
                _ if sanitize.is_some() => eprintln!(
                    "koo: the program could not be built with the sanitizer. That needs clang's sanitizer \
                     libraries: `libclang-rt-dev` on Debian and Ubuntu, `compiler-rt` on Fedora and Alpine."
                ),
                _ => {
                    dir.keep = true;
                    eprintln!(
                        "koo: clang rejected the generated code. This is a bug in the compiler; the IR is at {}",
                        ir_path.display()
                    )
                }
            }
            return ExitCode::FAILURE;
        }
        Err(err) => {
            let install = if cfg!(target_os = "macos") {
                "xcode-select --install"
            } else {
                "your package manager: `apt install clang`, `dnf install clang` or `apk add clang`"
            };
            eprintln!("koo: cannot run `{compiler}`, which builds the program: {err}");
            eprintln!("     Koo needs clang 15 or newer. Install it with {install}.");
            return ExitCode::FAILURE;
        }
    }
    if !run {
        return ExitCode::SUCCESS;
    }
    if mode == Mode::Test {
        return run_tests(&executable);
    }
    // While the program runs, a signal to stop is passed on to it and does not end this
    // process, which has the files of the build to remove when the program has ended.
    // (They cannot be removed sooner: macOS kills a program whose file is gone.)
    let ended = Command::new(&executable)
        .args(program_args)
        .spawn()
        .and_then(|mut child| {
            RUNNING.store(child.id() as i32, std::sync::atomic::Ordering::Relaxed);
            for stop in [1, 2, 15] {
                unsafe { signal(stop, pass_on as extern "C" fn(i32) as usize) };
            }
            child.wait()
        });
    drop(dir);
    match ended {
        Ok(status) => match status.code() {
            Some(code) => ExitCode::from(code as u8),
            // Stopped from outside, by an interrupt, a hang-up, a closed pipe or a
            // request to end: that is no fault of the program's, and is not reported.
            None if matches!(
                std::os::unix::process::ExitStatusExt::signal(&status),
                Some(1 | 2 | 13 | 15)
            ) =>
            {
                ExitCode::from(128 + std::os::unix::process::ExitStatusExt::signal(&status).unwrap_or(0) as u8)
            }
            None => {
                // No exit status: the program was stopped by a signal. The one way a
                // Koo program is known to crash is by running out of stack; anything
                // else is a bug in the compiler.
                eprintln!(
                    "koo: the program crashed ({status}). If it calls itself without end, or builds a value \
                     nested many thousands deep, it ran out of stack; otherwise this is a bug in the compiler."
                );
                ExitCode::from(134)
            }
        },
        Err(err) => {
            eprintln!("koo: cannot run {}: {err}", executable.display());
            ExitCode::FAILURE
        }
    }
}

/// Runs each test of a test program in a process of its own, so that a test that panics
/// fails alone, and reports the results (Section 12.4).
fn run_tests(executable: &Path) -> ExitCode {
    let listed = match Command::new(executable).env_remove("KOO_TEST").output() {
        Ok(output) => output,
        Err(err) => {
            eprintln!("koo: cannot run {}: {err}", executable.display());
            return ExitCode::FAILURE;
        }
    };
    let names: Vec<String> = String::from_utf8_lossy(&listed.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    let mut failed = 0;
    for (index, name) in names.iter().enumerate() {
        let output = Command::new(executable).env("KOO_TEST", index.to_string()).output();
        let ok = matches!(&output, Ok(output) if output.status.success());
        println!("{} {name}", if ok { "ok    " } else { "FAILED" });
        if !ok {
            failed += 1;
            if let Ok(output) = output {
                for line in String::from_utf8_lossy(&output.stdout).lines() {
                    println!("       | {line}");
                }
                for line in String::from_utf8_lossy(&output.stderr).lines() {
                    println!("       {line}");
                }
            }
        }
    }
    let total = names.len();
    println!();
    if failed == 0 {
        println!("{total} test{} passed", if total == 1 { "" } else { "s" });
        ExitCode::SUCCESS
    } else {
        println!("{failed} of {total} tests failed");
        ExitCode::FAILURE
    }
}
