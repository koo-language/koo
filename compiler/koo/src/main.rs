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
  --sanitize     Build with AddressSanitizer, to check the compiler's own output

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

fn report(sources: &SourceMap, diagnostics: &[Diagnostic]) {
    for diagnostic in diagnostics {
        eprintln!("{}", sources.render(diagnostic));
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

/// A directory for the files of one build.
fn work_dir(source: &Path) -> std::io::Result<PathBuf> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source
        .canonicalize()
        .unwrap_or_else(|_| source.to_path_buf())
        .hash(&mut hasher);
    let dir = std::env::temp_dir().join(format!("koo-build-{:016x}", hasher.finish()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
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
    let mut sanitize = false;
    let mut output: Option<PathBuf> = None;
    let mut path: Option<&String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--release" => release = true,
            "--emit-llvm" => emit_llvm = true,
            "--sanitize" => sanitize = true,
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
    let ir = if sanitize {
        ir.lines()
            .map(|line| match line.strip_suffix(" {") {
                Some(head) if line.starts_with("define ") => format!("{head} sanitize_address {{\n"),
                _ => format!("{line}\n"),
            })
            .collect()
    } else {
        ir
    };
    let prepared = work_dir(source_path).and_then(|dir| {
        let ir_path = dir.join("program.ll");
        let runtime = dir.join("libkoo_rt.a");
        std::fs::write(&ir_path, &ir)?;
        std::fs::write(&runtime, RUNTIME)?;
        Ok((dir, ir_path, runtime))
    });
    let (dir, ir_path, runtime) = match prepared {
        Ok(paths) => paths,
        Err(err) => {
            eprintln!("koo: cannot prepare the build directory: {err}");
            return ExitCode::FAILURE;
        }
    };
    let executable = match (&output, run) {
        (Some(output), _) => output.clone(),
        (None, true) => dir.join(stem),
        (None, false) => PathBuf::from(stem),
    };
    // LLVM does the optimizing and the linking, through the system's clang (Section 13.2).
    let mut clang = Command::new("clang");
    if sanitize {
        clang.arg("-fsanitize=address").arg("-g");
    }
    let status = clang
        .arg(if release { "-O3" } else { "-O0" })
        .arg("-Wno-override-module")
        .arg(&ir_path)
        .arg(&runtime)
        .arg("-o")
        .arg(&executable)
        .status();
    match status {
        Ok(status) if status.success() => {}
        Ok(_) => {
            eprintln!(
                "koo: clang rejected the generated code. This is a bug in the compiler; the IR is at {}",
                ir_path.display()
            );
            return ExitCode::FAILURE;
        }
        Err(err) => {
            eprintln!("koo: cannot run clang, which builds the program: {err}");
            return ExitCode::FAILURE;
        }
    }
    if !run {
        return ExitCode::SUCCESS;
    }
    if mode == Mode::Test {
        return run_tests(&executable);
    }
    match Command::new(&executable).args(program_args).status() {
        Ok(status) => match status.code() {
            Some(code) => ExitCode::from(code as u8),
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
