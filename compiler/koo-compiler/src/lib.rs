//! The Koo compiler after parsing: type checking and code generation.
//!
//! `compile` takes the text of a program and returns LLVM IR, or the errors found. The
//! `koo` command hands the IR to the system's `clang`, with the support library `koo-rt`.

pub mod check;
pub mod derive;
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
    let prelude = koo_syntax::parse_library(PRELUDE);
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
    // The packages a program imports from directories (Section 8.2) are compiled with it
    // as one program. What keeps their names apart is the name of each package, set
    // before the names of its types, functions and constants as the standard packages
    // have theirs (`Http__Request`, `http__get`): for the types here, as the package's
    // files are read, and for the functions and constants just below. Inside a package
    // its own names are written plain, and the checker looks for them so.
    use koo_syntax::ast::{FnTarget, ItemKind, TypeKind};
    use std::collections::{HashMap, HashSet};
    let several = sources.files().iter().any(|file| !file.package.is_empty());
    // The types of each package, by the names it writes them with; those it does not
    // offer; and what each file's imports of packages bind.
    let mut types: HashMap<&str, HashMap<String, String>> = HashMap::new();
    let mut kept: HashSet<String> = HashSet::new();
    let mut bound: Vec<Vec<(String, &str)>> = Vec::new();
    // And the names of the types of the program's own files.
    let mut programs_types: Vec<String> = Vec::new();
    for file in sources.files().iter().filter(|_| several) {
        let read = koo_syntax::parse_file_at(&file.src, file.base).file;
        let mut names = Vec::new();
        for import in &read.imports {
            if let koo_syntax::ast::ImportKind::Package { path, .. } = &import.kind
                && let Some((_, package)) = file.imports.iter().find(|(written, _)| written == path)
            {
                names.push((import.binding.clone(), package.as_str()));
            }
        }
        bound.push(names);
        let own = types.entry(file.package.as_str()).or_default();
        for item in &read.items {
            let name = match &item.kind {
                ItemKind::Type(decl) => &decl.name,
                ItemKind::Enum(decl) => &decl.name,
                ItemKind::Error(decl) => &decl.name,
                ItemKind::Interface(decl) => &decl.name,
                ItemKind::Actor(decl) => &decl.name,
                _ => continue,
            };
            if file.package.is_empty() {
                programs_types.push(name.name.clone());
                continue;
            }
            let full = check::type_in_package(&file.package, &name.name);
            if !item.public {
                kept.insert(full.clone());
            }
            own.insert(name.name.clone(), full);
        }
    }
    let mut packages = check::Packages::default();
    for (number, file) in sources.files().iter().enumerate() {
        packages.of.push(file.package.clone());
        packages.imports.push(file.imports.clone());
        let own = types.get(file.package.as_str()).filter(|_| !file.package.is_empty());
        let mut parsed = if several {
            // A package does not see the types of the program that imports it: where it
            // writes the name of one, the name is read as one of its own that it has
            // not got, and is unknown.
            let mut read_as = own.cloned().unwrap_or_default();
            if !file.package.is_empty() {
                for name in &programs_types {
                    read_as
                        .entry(name.clone())
                        .or_insert_with(|| check::type_in_package(&file.package, name));
                }
            }
            let renames = koo_syntax::Renames {
                own: read_as,
                imported: bound[number]
                    .iter()
                    .filter_map(|(binding, package)| Some((binding.clone(), types.get(package)?.clone())))
                    .collect(),
                kept: kept.clone(),
                used: Default::default(),
            };
            let parsed = koo_syntax::parse_file_renamed(&file.src, file.base, &renames);
            packages.used.push(renames.used.into_inner().unwrap());
            parsed
        } else {
            packages.used.push(Vec::new());
            koo_syntax::parse_file_at(&file.src, file.base)
        };
        // The type a method or a type-level function is of, which is one of its
        // package's own: a method is declared in the package of its type (Section 4.6).
        let of_type = |ty: &koo_syntax::ast::Type| match &ty.kind {
            TypeKind::Named { path, .. } if path.len() == 1 => Some(path[0].name.clone()),
            _ => None,
        };
        if several && file.package.is_empty() {
            for item in &parsed.file.items {
                if let ItemKind::Fn(decl) = &item.kind
                    && let FnTarget::Method { ty: Some(ty), .. } | FnTarget::TypeLevel(ty) = &decl.sig.target
                    && of_type(ty).is_some_and(|on| check::package_of_name(&on).is_some())
                {
                    diagnostics.push(
                        Diagnostic::new(ty.span, "a program gives methods to its own types, not to a package's")
                            .with_note("a method is declared in the package that declares its type (Section 4.6)")
                            .with_help("make it a function that takes the value as a parameter"),
                    );
                }
            }
        }
        if let Some(own) = own {
            let prefix = check::type_in_package(&file.package, "");
            for item in &mut parsed.file.items {
                let name = match &mut item.kind {
                    ItemKind::Fn(decl) => match &decl.sig.target {
                        FnTarget::Free => {
                            decl.sig.name.name = format!("{}__{}", file.package, decl.sig.name.name);
                            decl.sig.name.name.clone()
                        }
                        FnTarget::Method { ty: Some(ty), .. } | FnTarget::TypeLevel(ty) => match of_type(ty) {
                            Some(on) if own.values().any(|full| *full == on) => {
                                format!("{on}.{}", decl.sig.name.name)
                            }
                            _ => {
                                diagnostics.push(
                                    Diagnostic::new(ty.span, "a package gives methods to its own types alone")
                                        .with_note(
                                            "a method is declared in the package that declares its type (Section 4.6)",
                                        )
                                        .with_help("make it a function that takes the value as a parameter"),
                                );
                                continue;
                            }
                        },
                        FnTarget::Method { ty: None, .. } => continue,
                    },
                    ItemKind::Const(decl) => {
                        decl.name.name = format!("{}__{}", file.package, decl.name.name);
                        decl.name.name.clone()
                    }
                    ItemKind::Enum(decl) => {
                        if item.attrs.iter().any(|attribute| attribute.name.name == "closed") {
                            packages.closed.insert(decl.name.name.clone());
                        }
                        // A variant is known by the name it was given, also where a
                        // type of the package has the same name.
                        for variant in &mut decl.variants {
                            if let Some(plain) = variant.name.name.strip_prefix(&prefix) {
                                variant.name.name = plain.to_string();
                            }
                        }
                        decl.name.name.clone()
                    }
                    ItemKind::Type(decl) => decl.name.name.clone(),
                    ItemKind::Error(decl) => decl.name.name.clone(),
                    ItemKind::Interface(decl) => decl.name.name.clone(),
                    ItemKind::Actor(decl) => {
                        // A method of an actor is offered like any other.
                        for method in decl.methods.iter().filter(|method| !method.public) {
                            packages
                                .private
                                .insert(format!("{}.{}", decl.name.name, method.decl.sig.name.name));
                        }
                        decl.name.name.clone()
                    }
                    ItemKind::Stmt(_) => continue,
                };
                // What a package does not offer with `pub` is its own (Section 4.6).
                if !item.public {
                    packages.private.insert(name);
                }
            }
        }
        diagnostics.append(&mut parsed.diagnostics);
        package.imports.append(&mut parsed.file.imports);
        package.items.append(&mut parsed.file.items);
    }
    if !diagnostics.is_empty() {
        return Err(shown(diagnostics, sources, &bound));
    }
    let bases: Vec<u32> = sources.files().iter().map(|file| file.base).collect();
    let extra = std::cell::OnceCell::new();
    // What the compiler writes for the program (`derive.rs`): the JSON functions of its
    // types, if it says `json`, and what carries messages to its actors.
    let json = sources.files().iter().any(|file| file.src.contains("json"));
    let rows = sources.files().iter().any(|file| file.src.contains("from_row"));
    let envs = sources.files().iter().any(|file| file.src.contains("from_env"));
    let written = derive::derive(&package, json, rows, envs);
    let derived = koo_syntax::parse_library(&written.source);
    assert!(
        derived.diagnostics.is_empty(),
        "what was written for the program does not parse: {:?}\n{}",
        derived.diagnostics,
        written.source
    );
    let unwritten = written.unwritten;
    let problems = written
        .problems
        .into_iter()
        .map(|(span, message, help)| Diagnostic::new(span, message).with_help(help))
        .collect();
    let (program, mut diagnostics) = check::check(
        &prelude.file,
        &package,
        &derived.file,
        problems,
        &extra,
        &bases,
        packages,
        tests,
    );

    // An actor that could not be written has no `start`, and a method that could not be
    // carried is not one of its handle's, which the place that uses it need not also be told.
    diagnostics.retain(|diagnostic| {
        !unwritten
            .iter()
            .any(|missing| diagnostic.message.ends_with(missing.as_str()))
    });
    if !diagnostics.is_empty() {
        diagnostics.sort_by_key(|d| d.span.start);
        return Err(shown(diagnostics, sources, &bound));
    }
    Ok(llvm::emit(&program, sources, PRELUDE))
}

/// What is said of a program names the things of its packages as the program writes
/// them where the mistake is: `orders.Order` and `orders.place`, by the name the file
/// has given the package, or the directory's where the file does not import it, and
/// with no package at all in a file of the package itself; and not the names that keep
/// one package's apart from another's inside the compiler.
fn shown(mut diagnostics: Vec<Diagnostic>, sources: &SourceMap, bound: &[Vec<(String, &str)>]) -> Vec<Diagnostic> {
    if sources.files().iter().all(|file| file.package.is_empty()) {
        return diagnostics;
    }
    for diagnostic in &mut diagnostics {
        let file = sources.file_index(diagnostic.span.start);
        // (A name with `__` that the program wrote itself is said as it wrote it.)
        let at = &sources.files()[file];
        let (from, to) = (
            diagnostic.span.start.saturating_sub(at.base) as usize,
            diagnostic.span.end.saturating_sub(at.base) as usize,
        );
        if at.src.get(from..to).is_some_and(|written| written.contains("__")) {
            continue;
        }
        let here = sources.files()[file].package.as_str();
        let written = |package: &str| -> String {
            if package == here {
                return String::new();
            }
            let named = bound
                .get(file)
                .and_then(|names| names.iter().find(|(_, of)| *of == package));
            match named {
                Some((binding, _)) => format!("{binding}."),
                None => format!("{}.", check::package_shown(package)),
            }
        };
        let say = |text: &mut String| *text = check::rewrite_names(text, &written);
        say(&mut diagnostic.message);
        if let Some(label) = &mut diagnostic.label {
            say(label);
        }
        for (_, related) in &mut diagnostic.related {
            say(related);
        }
        diagnostic.notes.iter_mut().for_each(say);
        diagnostic.help.iter_mut().for_each(say);
        if let Some(fix) = &mut diagnostic.fix {
            say(&mut fix.message);
            for (_, text) in &mut fix.edits {
                say(text);
            }
        }
    }
    diagnostics
}
