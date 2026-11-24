//! The Koo compiler after parsing: type checking and code generation.
//!
//! `compile` takes the text of a program and returns LLVM IR, or the errors found. The
//! `koo` command hands the IR to the system's `clang`, with the support library `koo-rt`.

pub mod check;
pub mod hir;
pub mod intrinsics;
pub mod llvm;

use koo_syntax::{Diagnostic, SourceMap};

/// The standard prelude: the part of the standard library that is in scope in every
/// program (Section 4.1), written in Koo.
pub const PRELUDE: &str = include_str!("prelude.koo");

/// Compiles one source file to LLVM IR.
pub fn compile(src: &str, file_name: &str) -> Result<String, Vec<Diagnostic>> {
    compile_sources(&SourceMap::single(file_name, src), false)
}

/// Compiles one source file. With `tests`, the program is its test functions (Section
/// 12.4): it runs the one whose number it is given, and with no number lists them.
pub fn compile_as(src: &str, file_name: &str, tests: bool) -> Result<String, Vec<Diagnostic>> {
    compile_sources(&SourceMap::single(file_name, src), tests)
}

/// Compiles a program: one file, or the files of a package, which share one namespace
/// (Section 8.1). The spans of the diagnostics count from the start of the package, and
/// `sources` renders them.
pub fn compile_sources(sources: &SourceMap, tests: bool) -> Result<String, Vec<Diagnostic>> {
    let prelude = koo_syntax::parse_file(PRELUDE);
    assert!(
        prelude.diagnostics.is_empty(),
        "the prelude does not parse: {:?}",
        prelude.diagnostics
    );
    let mut package = koo_syntax::ast::File {
        imports: Vec::new(),
        items: Vec::new(),
    };
    let mut diagnostics = Vec::new();
    for file in sources.files() {
        let mut parsed = koo_syntax::parse_file_at(&file.src, file.base);
        diagnostics.append(&mut parsed.diagnostics);
        package.imports.append(&mut parsed.file.imports);
        package.items.append(&mut parsed.file.items);
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    let bases: Vec<u32> = sources.files().iter().map(|file| file.base).collect();
    let extra = std::cell::OnceCell::new();
    let (program, mut diagnostics) = check::check(&prelude.file, &package, &extra, &bases, tests);
    if !diagnostics.is_empty() {
        diagnostics.sort_by_key(|d| d.span.start);
        return Err(diagnostics);
    }
    Ok(llvm::emit(&program, sources, PRELUDE))
}
