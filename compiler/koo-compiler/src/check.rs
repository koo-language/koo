//! Name resolution and type checking: the syntax tree in, the typed program out.
//!
//! This is steps 2 to 4 of the pipeline in Section 13.1, for the part of the language the
//! compiler implements so far. A construct outside that part is reported as not supported
//! yet; nothing is accepted and then compiled wrongly.

use std::cell::OnceCell;
use std::collections::HashMap;

use koo_syntax::ast;
use koo_syntax::{Diagnostic, Span};

use crate::hir::*;

mod calls;
mod exprs;
mod fuse;
mod generics;
mod lambdas;
mod maps;
mod patterns;
mod suggest;

/// Checks a program. `prelude` is the standard prelude, whose declarations are in scope in
/// every program (Section 4.1).
///
/// `extra` is somewhere to keep the functions the checker writes for itself (see `fuse`),
/// which must live as long as the two files do.
pub fn check<'a>(
    prelude: &'a ast::File,
    file: &'a ast::File,
    extra: &'a OnceCell<ast::File>,
    bases: &[u32],
    test_mode: bool,
) -> (Program, Vec<Diagnostic>) {
    let mut checker = Checker::new(extra);
    checker.program.test_mode = test_mode;
    checker.file_bases = bases.to_vec();
    checker.in_prelude = true;
    let prelude_bodies = checker.declare_file(prelude);
    checker.in_prelude = false;
    let bodies = checker.declare_file(file);
    checker.finish(file, prelude_bodies, bodies);
    // One mistake in a generic function is found once for each way it is instantiated.
    let mut seen = std::collections::HashSet::new();
    checker
        .diags
        .retain(|d| seen.insert((d.span.start, d.span.end, d.message.clone())));
    (checker.program, checker.diags)
}

#[derive(Debug, Clone)]
enum TypeName {
    Struct(StructId),
    Enum(EnumId),
    Error(ErrorId),
    Alias(Ty),
    ErrorSet(ErrSet),
}

/// The type a method or type-level function belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Owner {
    Struct(StructId),
    Enum(EnumId),
    Error(ErrorId),
    // The built-in types, whose methods the prelude declares.
    Str,
    Int,
    Float,
    Bool,
    Byte,
}

#[derive(Debug, Clone)]
struct ParamSig {
    name: String,
    kind: ParamKind,
    ty: Ty,
    /// The value of the parameter when a call leaves it out: an index into `param_defaults`.
    default: Option<usize>,
    /// Whether the parameter is a function that is only lent: it is borrowed and declared
    /// with a function type (Section 5.7). A parameter of type `T` is not, whatever `T` is.
    lent: bool,
}

#[derive(Debug, Clone)]
struct FnSig {
    name: String,
    /// The receiver, if the function is a method. It is passed as the first argument.
    receiver: Option<ParamSig>,
    params: Vec<ParamSig>,
    ret: Ty,
}

pub(crate) struct Checker<'a> {
    // Generics (see `check/generics.rs`).
    generic_types: HashMap<String, generics::GenericType<'a>>,
    type_instances: HashMap<(String, Vec<Ty>), Ty>,
    /// For a type made from a generic one: the generic type's name and its arguments.
    instance_of: HashMap<Ty, (String, Vec<Ty>)>,
    templates: Vec<generics::Template<'a>>,
    generic_fns: HashMap<String, usize>,
    generic_methods: HashMap<(generics::OwnerKey, String), usize>,
    generic_type_fns: HashMap<(generics::OwnerKey, String), usize>,
    fn_instances: HashMap<(usize, Vec<Ty>), FnId>,
    pending: Vec<generics::Pending<'a>>,
    /// Where the functions for fused chains are kept once they are parsed.
    extra: &'a OnceCell<ast::File>,
    interfaces: HashMap<String, &'a ast::InterfaceDecl>,
    /// Generic functions declared on a type that is not itself generic, and whether each
    /// is a method.
    plain_generic_methods: HashMap<(Owner, String), (usize, bool)>,
    /// Whether the declarations being read are the prelude's.
    in_prelude: bool,
    // Lambdas (see `check/lambdas.rs`).
    /// The functions around the lambda being checked, outermost first.
    enclosing: Vec<lambdas::Enclosing>,
    /// Set while a lambda's body is checked.
    lambda: Option<lambdas::LambdaState>,
    /// Whether the lambda about to be checked is passed to a borrowed function parameter.
    next_lambda_is_block: bool,
    /// The locals of the current function that are function values only lent to it: they
    /// may be called and passed on, and nothing else (Section 5.3).
    borrowed_fns: std::collections::HashSet<LocalId>,
    /// Lent functions that a closure names, which it may not: it could outlive them.
    lent_in_closure: std::collections::HashSet<LocalId>,
    /// Whether a borrowed function value may be named here: as the callee of a call, or
    /// as an argument to another borrowed function parameter.
    lent_fn_ok: bool,
    lambda_count: usize,
    /// Whether the body being checked belongs to the prelude, which may call intrinsics.
    body_in_prelude: bool,
    /// The kind of the parameter that the argument being inferred is for.
    argument_kind: ParamKind,
    /// The type arguments of the instantiation being checked.
    type_env: HashMap<String, Ty>,
    /// Set while a body from the prelude is checked: the call that asked for it, where its
    /// errors are reported.
    origin: Option<(Span, String)>,
    program: Program,
    diags: Vec<Diagnostic>,
    type_names: HashMap<String, TypeName>,
    /// Default values of struct fields, by struct and field.
    defaults: Vec<Vec<Option<Expr>>>,
    sigs: Vec<FnSig>,
    free_fns: HashMap<String, FnId>,
    methods: HashMap<(Owner, String), FnId>,
    type_fns: HashMap<(Owner, String), FnId>,
    consts: HashMap<String, Expr>,
    // The function being checked.
    locals: Vec<Local>,
    /// For each local: whether its value is read anywhere, and where it was declared.
    used: Vec<bool>,
    declared_at: Vec<Span>,
    scopes: Vec<HashMap<String, LocalId>>,
    ret: Ty,
    loops: usize,
    // Prelude errors.
    overflow: ErrorId,
    parse_error: ErrorId,
    /// The first error type that is not predeclared.
    first_declared_error: ErrorId,
    /// The default values of parameters (Section 5.3).
    param_defaults: Vec<&'a ast::Expr>,
    /// Statements that take apart the tuple a condition bound: `if (a, b) := e`. They
    /// go first in the block the condition guards.
    cond_stmts: Vec<Stmt>,
    /// The standard packages each file imports, and whether the file uses each. An
    /// import belongs to its file (Section 8.2).
    modules: HashMap<(usize, String), (Span, bool)>,
    /// Where each file of the package begins: a span says which file it is in.
    file_bases: Vec<u32>,
    /// The file of the function being checked.
    file: usize,
}

impl<'a> Checker<'a> {
    fn new(extra: &'a OnceCell<ast::File>) -> Checker<'a> {
        let mut checker = Checker {
            generic_types: HashMap::new(),
            type_instances: HashMap::new(),
            instance_of: HashMap::new(),
            templates: Vec::new(),
            generic_fns: HashMap::new(),
            generic_methods: HashMap::new(),
            generic_type_fns: HashMap::new(),
            fn_instances: HashMap::new(),
            pending: Vec::new(),
            extra,
            interfaces: HashMap::new(),
            plain_generic_methods: HashMap::new(),
            in_prelude: false,
            enclosing: Vec::new(),
            lambda: None,
            next_lambda_is_block: false,
            borrowed_fns: std::collections::HashSet::new(),
            lent_in_closure: std::collections::HashSet::new(),
            lent_fn_ok: false,
            lambda_count: 0,
            body_in_prelude: false,
            argument_kind: ParamKind::Borrow,
            type_env: HashMap::new(),
            origin: None,
            program: Program::default(),
            diags: Vec::new(),
            type_names: HashMap::new(),
            defaults: Vec::new(),
            sigs: Vec::new(),
            free_fns: HashMap::new(),
            methods: HashMap::new(),
            type_fns: HashMap::new(),
            consts: HashMap::new(),
            locals: Vec::new(),
            used: Vec::new(),
            declared_at: Vec::new(),
            scopes: Vec::new(),
            ret: Ty::Unit,
            loops: 0,
            overflow: 0,
            parse_error: 0,
            first_declared_error: 0,
            param_defaults: Vec::new(),
            cond_stmts: Vec::new(),
            modules: HashMap::new(),
            file_bases: vec![0],
            file: 0,
        };
        // The prelude error types this compiler knows (Section 4.1).
        checker.overflow = checker.add_error("Overflow", Vec::new());
        checker.parse_error = checker.add_error(
            "ParseError",
            vec![FieldDef {
                name: "input".to_string(),
                ty: Ty::Str,
            }],
        );
        checker.add_error(
            "IoError",
            vec![FieldDef {
                name: "message".to_string(),
                ty: Ty::Str,
            }],
        );
        checker.first_declared_error = checker.program.errors.len();
        checker
    }

    /// The number of the file a span is in.
    fn file_of(&self, span: Span) -> usize {
        self.file_bases
            .iter()
            .rposition(|base| *base <= span.start)
            .unwrap_or(0)
    }

    fn add_error(&mut self, name: &str, fields: Vec<FieldDef>) -> ErrorId {
        let id = self.program.errors.len();
        self.program.errors.push(ErrorDef {
            name: name.to_string(),
            fields,
        });
        self.type_names.insert(name.to_string(), TypeName::Error(id));
        id
    }

    // ---- diagnostics --------------------------------------------------------------------

    fn error(&mut self, span: Span, message: impl Into<String>) {
        match &self.origin {
            // A mistake found inside a prelude function is reported at the call that
            // needed the function, which is the code the programmer can change.
            Some((origin, function)) => {
                let message = format!("in `{function}`: {}", message.into());
                self.diags.push(Diagnostic::new(*origin, message));
            }
            None => self.diags.push(Diagnostic::new(span, message)),
        }
    }

    fn error_help(&mut self, span: Span, message: impl Into<String>, help: impl Into<String>) {
        match &self.origin {
            Some((origin, function)) => {
                let message = format!("in `{function}`: {}", message.into());
                self.diags.push(Diagnostic::new(*origin, message).with_help(help));
            }
            None => self.diags.push(Diagnostic::new(span, message).with_help(help)),
        }
    }

    /// Reports a part of the language that the compiler does not implement yet.
    fn unsupported(&mut self, span: Span, what: &str) {
        self.error_help(
            span,
            format!("{what} not supported by this compiler yet"),
            "the compiler implements the core of the language so far; see the README for what is in",
        );
    }

    fn unknown(&self, span: Span) -> Expr {
        Expr {
            kind: ExprKind::Unit,
            ty: Ty::Unknown,
            span,
        }
    }

    pub(crate) fn ty_name(&self, ty: &Ty) -> String {
        match ty {
            Ty::Int => "int".to_string(),
            Ty::Float => "float".to_string(),
            Ty::Bool => "bool".to_string(),
            Ty::Byte => "byte".to_string(),
            Ty::Str => "string".to_string(),
            Ty::Unit => "()".to_string(),
            Ty::Never => "a value that never arrives".to_string(),
            Ty::List(element) => format!("[{}]", self.ty_name(element)),
            Ty::Optional(inner) => match **inner {
                Ty::Result(..) => format!("({})?", self.ty_name(inner)),
                _ => format!("{}?", self.ty_name(inner)),
            },
            Ty::Result(ok, set) => {
                let set = match set {
                    ErrSet::Any => "Failure".to_string(),
                    ErrSet::Of(ids) => ids
                        .iter()
                        .map(|&id| self.program.errors[id].name.clone())
                        .collect::<Vec<_>>()
                        .join(" | "),
                };
                match **ok {
                    Ty::Unit => format!("() ! {set}"),
                    _ => format!("{} ! {set}", self.ty_name(ok)),
                }
            }
            Ty::Struct(_) if self.map_types(ty).is_some() => {
                let (key, value) = self.map_types(ty).unwrap();
                format!("[{}: {}]", self.ty_name(&key), self.ty_name(&value))
            }
            Ty::Struct(id) => self.program.structs[*id].name.clone(),
            Ty::Enum(id) => self.program.enums[*id].name.clone(),
            Ty::Error(id) => self.program.errors[*id].name.clone(),
            Ty::Failure => "Failure".to_string(),
            Ty::Tuple(types) => format!(
                "({})",
                types.iter().map(|t| self.ty_name(t)).collect::<Vec<_>>().join(", ")
            ),
            Ty::Fn(function) => {
                let params: Vec<String> = function
                    .params
                    .iter()
                    .map(|(kind, ty)| {
                        let kind = match kind {
                            ParamKind::Borrow => "",
                            ParamKind::Var => "var ",
                            ParamKind::Take => "take ",
                        };
                        format!("{kind}{}", self.ty_name(ty))
                    })
                    .collect();
                match &function.ret {
                    Ty::Unit => format!("fn({})", params.join(", ")),
                    ret => format!("fn({}) -> {}", params.join(", "), self.ty_name(ret)),
                }
            }
            Ty::Unknown => "_".to_string(),
        }
    }

    // ---- declarations -------------------------------------------------------------------

    /// Reads the declarations of one source: its types, constants and function
    /// signatures. Returns the functions whose bodies are to be checked.
    fn declare_file(&mut self, file: &'a ast::File) -> Vec<(FnId, &'a ast::FnDecl)> {
        for import in &file.imports {
            // The standard packages that exist so far are part of the prelude; an import
            // brings one's name into scope (Section 8.2).
            match &import.kind {
                ast::ImportKind::Package { path, .. } if matches!(path.as_str(), "std/os" | "std/io" | "std/test") => {
                    let key = (self.file_of(import.span), import.binding.clone());
                    if self.modules.insert(key, (import.span, false)).is_some() {
                        self.error(import.span, format!("`{}` is imported twice", import.binding));
                    }
                }
                ast::ImportKind::Package { path, .. } if path.starts_with("std/") => {
                    self.error_help(
                        import.span,
                        format!("the package `{path}` is not part of this compiler yet"),
                        "the standard packages so far are `std/os`, `std/io` and `std/test`",
                    );
                }
                _ => self.unsupported(import.span, "imports of other packages are"),
            }
        }

        // 1. Every type name, so that declarations may refer to one another in any order.
        for item in &file.items {
            let (name, kind) = match &item.kind {
                ast::ItemKind::Type(decl) if !decl.type_params.is_empty() => {
                    self.declare_generic_type(&decl.name, &decl.type_params, generics::GenericDecl::Struct(decl));
                    continue;
                }
                ast::ItemKind::Enum(decl) if !decl.type_params.is_empty() => {
                    self.declare_generic_type(&decl.name, &decl.type_params, generics::GenericDecl::Enum(decl));
                    continue;
                }
                ast::ItemKind::Interface(decl) => {
                    if !decl.type_params.is_empty() || decl.resource {
                        self.unsupported(decl.name.span, "generic and resource interfaces are");
                    } else if self.interfaces.insert(decl.name.name.clone(), decl).is_some() {
                        self.error(decl.name.span, format!("`{}` is already declared", decl.name.name));
                    }
                    continue;
                }
                ast::ItemKind::Type(decl) => match &decl.body {
                    ast::TypeBody::Struct(_) if !decl.resource => {
                        let id = self.program.structs.len();
                        self.program.structs.push(StructDef {
                            name: decl.name.name.clone(),
                            fields: Vec::new(),
                        });
                        self.defaults.push(Vec::new());
                        (&decl.name, Some(TypeName::Struct(id)))
                    }
                    _ => (&decl.name, None),
                },
                ast::ItemKind::Enum(decl) => {
                    let id = self.program.enums.len();
                    self.program.enums.push(EnumDef {
                        name: decl.name.name.clone(),
                        variants: Vec::new(),
                    });
                    (&decl.name, Some(TypeName::Enum(id)))
                }
                ast::ItemKind::Error(decl) => match &decl.body {
                    ast::ErrorBody::Fields(_) => {
                        if self.type_names.contains_key(&decl.name.name) {
                            (&decl.name, None)
                        } else {
                            let id = self.program.errors.len();
                            self.program.errors.push(ErrorDef {
                                name: decl.name.name.clone(),
                                fields: Vec::new(),
                            });
                            (&decl.name, Some(TypeName::Error(id)))
                        }
                    }
                    ast::ErrorBody::Set(_) => (&decl.name, None),
                },
                _ => continue,
            };
            if self.type_names.contains_key(&name.name)
                || self.generic_types.contains_key(&name.name)
                || self.interfaces.contains_key(&name.name)
                || is_builtin_type(&name.name)
            {
                self.error(name.span, format!("`{}` is already declared", name.name));
            } else if let Some(kind) = kind {
                self.type_names.insert(name.name.clone(), kind);
            }
        }

        // 2. Aliases and named error sets, which may name the types above.
        for item in &file.items {
            match &item.kind {
                ast::ItemKind::Type(decl)
                    if !decl.type_params.is_empty()
                        && (!matches!(decl.body, ast::TypeBody::Struct(_)) || decl.resource) =>
                {
                    self.unsupported(decl.name.span, "generic aliases, distinct types and resources are");
                }
                ast::ItemKind::Type(decl) => match &decl.body {
                    ast::TypeBody::Struct(_) if decl.resource => self.unsupported(decl.name.span, "resources are"),
                    ast::TypeBody::Struct(_) => {}
                    ast::TypeBody::Distinct(_) => self.unsupported(decl.name.span, "distinct types are"),
                    ast::TypeBody::Alias(aliased) => {
                        let ty = self.resolve_type(aliased);
                        if !self.type_names.contains_key(&decl.name.name) {
                            self.type_names.insert(decl.name.name.clone(), TypeName::Alias(ty));
                        }
                    }
                },
                ast::ItemKind::Error(decl) => {
                    if let ast::ErrorBody::Set(types) = &decl.body {
                        let set = self.resolve_error_set(types);
                        if !self.type_names.contains_key(&decl.name.name) {
                            self.type_names.insert(decl.name.name.clone(), TypeName::ErrorSet(set));
                        }
                    }
                }
                _ => {}
            }
        }

        // 3. Fields.
        for item in &file.items {
            match &item.kind {
                ast::ItemKind::Type(decl) if decl.type_params.is_empty() => {
                    let (ast::TypeBody::Struct(fields), Some(TypeName::Struct(id))) =
                        (&decl.body, self.type_names.get(&decl.name.name).cloned())
                    else {
                        continue;
                    };
                    if self.program.structs[id].fields.is_empty() {
                        let (defs, defaults) = self.fields(fields, true);
                        self.program.structs[id].fields = defs;
                        self.defaults[id] = defaults;
                    }
                }
                ast::ItemKind::Enum(decl) if decl.type_params.is_empty() => {
                    let Some(TypeName::Enum(id)) = self.type_names.get(&decl.name.name).cloned() else {
                        continue;
                    };
                    let variants = self.variants(decl);
                    if self.program.enums[id].variants.is_empty() {
                        self.program.enums[id].variants = variants;
                    }
                }
                ast::ItemKind::Error(decl) => {
                    let (ast::ErrorBody::Fields(fields), Some(TypeName::Error(id))) =
                        (&decl.body, self.type_names.get(&decl.name.name).cloned())
                    else {
                        continue;
                    };
                    if id >= self.first_declared_error && self.program.errors[id].fields.is_empty() {
                        self.program.errors[id].fields = self.fields(fields, false).0;
                    }
                }
                ast::ItemKind::Actor(decl) => self.unsupported(decl.name.span, "actors are"),
                _ => {}
            }
        }
        self.reject_recursive_types(file);

        // 4. Constants and function signatures.
        for item in &file.items {
            if let ast::ItemKind::Const(decl) = &item.kind {
                self.constant(decl);
            }
        }
        let mut bodies: Vec<(FnId, &ast::FnDecl)> = Vec::new();
        for item in &file.items {
            if let ast::ItemKind::Fn(decl) = &item.kind {
                if !item.attrs.is_empty() {
                    self.unsupported(item.attrs[0].span, "attributes on functions are");
                }
                if let Some(id) = self.signature(decl) {
                    bodies.push((id, decl));
                }
            }
        }

        bodies
    }

    /// Checks the bodies of the generic functions that are waiting, and of those they need.
    fn check_pending(&mut self) {
        loop {
            while let Some(pending) = self.pending.pop() {
                self.type_env = pending.env;
                self.body_in_prelude = pending.origin.is_some();
                self.origin = pending.origin;
                let function = self.function(pending.id, pending.decl);
                self.program.functions[pending.id] = function;
            }
            self.type_env.clear();
            self.origin = None;
            self.body_in_prelude = false;
            if !self.generic_to_strings() {
                break;
            }
        }
    }

    /// Checks every body, after every declaration of the prelude and the program is known.
    fn finish(
        &mut self,
        file: &'a ast::File,
        prelude_bodies: Vec<(FnId, &'a ast::FnDecl)>,
        bodies: Vec<(FnId, &'a ast::FnDecl)>,
    ) {
        for ((owner, name), &id) in &self.methods {
            let sig = &self.sigs[id];
            let borrows = sig.receiver.as_ref().is_some_and(|r| r.kind == ParamKind::Borrow);
            if name == "to_string" && borrows && sig.params.is_empty() && sig.ret == Ty::Str {
                let ty = match *owner {
                    Owner::Struct(id) => Ty::Struct(id),
                    Owner::Enum(id) => Ty::Enum(id),
                    Owner::Error(id) => Ty::Error(id),
                    _ => continue,
                };
                self.program.to_string.insert(ty, id);
            }
        }

        // 5. Bodies, and then the bodies of the generic functions they use.
        for (id, decl) in prelude_bodies {
            // A mistake in the prelude itself is a mistake in the compiler.
            self.body_in_prelude = true;
            self.origin = Some((Span::default(), format!("the prelude's {}", self.sigs[id].name)));
            let function = self.function(id, decl);
            self.program.functions[id] = function;
        }
        self.origin = None;
        self.body_in_prelude = false;
        for (id, decl) in bodies {
            let function = self.function(id, decl);
            self.program.functions[id] = function;
        }
        self.check_pending();
        // 6. Chains of list methods, where one pass can do the work of several.
        // `KOO_NO_FUSE` leaves them as written, to compare what the two forms print.
        if self.diags.is_empty() && std::env::var_os("KOO_NO_FUSE").is_none() {
            self.fuse_chains();
        }

        let unused: Vec<(String, Span)> = self
            .modules
            .iter()
            .filter(|(_, (_, used))| !used)
            .map(|((_, name), (span, _))| (name.clone(), *span))
            .collect();
        for (name, span) in unused {
            self.error_help(
                span,
                format!("the import `{name}` is not used"),
                "remove it: an unused import is an error (Section 8.2)",
            );
        }
        if self.program.test_mode {
            // `koo test`: the functions named `test_*` are the program (Section 12.4).
            let mut tests: Vec<(String, FnId)> = self
                .free_fns
                .iter()
                .filter(|(name, _)| name.starts_with("test_") && !name.contains("__"))
                .map(|(name, id)| (name.clone(), *id))
                .collect();
            tests.sort();
            for (name, id) in &tests {
                let sig = &self.sigs[*id];
                let unit = match &sig.ret {
                    Ty::Result(ok, _) => **ok == Ty::Unit,
                    other => *other == Ty::Unit,
                };
                if !sig.params.is_empty() || !unit {
                    let span = self.program.functions[*id].span;
                    self.error_help(
                        span,
                        format!("the test `{name}` has the wrong signature"),
                        "a test is `fn test_name()` or `fn test_name() ! E`",
                    );
                }
            }
            if tests.is_empty() {
                let span = file.items.first().map(|i| i.span).unwrap_or_default();
                self.error_help(
                    Span {
                        start: span.start,
                        end: span.start,
                    },
                    "this file has no tests",
                    "a test is a function named `test_...`: `fn test_addition() { ... }`",
                );
            }
            self.program.tests = tests;
            return;
        }
        match self.free_fns.get("main") {
            Some(&id) => {
                let sig = &self.sigs[id];
                let ok = sig.params.is_empty() && matches!(&sig.ret, Ty::Unit | Ty::Result(_, _));
                let unit = match &sig.ret {
                    Ty::Result(ok, _) => **ok == Ty::Unit,
                    other => *other == Ty::Unit,
                };
                if !ok || !unit {
                    let span = self.program.functions[id].span;
                    self.error_help(
                        span,
                        "`main` has the wrong signature",
                        "`main` is `fn main()` or `fn main() ! E` (Section 6.1)",
                    );
                }
                self.program.main = Some(id);
            }
            None => {
                let span = file.items.first().map(|i| i.span).unwrap_or_default();
                self.error_help(
                    Span {
                        start: span.start,
                        end: span.start,
                    },
                    "the program has no `main` function",
                    "add `fn main() { ... }`",
                );
            }
        }
    }

    fn variants(&mut self, decl: &'a ast::EnumDecl) -> Vec<VariantDef> {
        let mut variants: Vec<VariantDef> = Vec::new();
        for variant in &decl.variants {
            if variants.iter().any(|v| v.name == variant.name.name) {
                self.error(
                    variant.name.span,
                    format!("the variant `{}` is declared twice", variant.name.name),
                );
                continue;
            }
            let fields = match &variant.fields {
                Some(fields) => self.fields(fields, false).0,
                None => Vec::new(),
            };
            variants.push(VariantDef {
                name: variant.name.name.clone(),
                fields,
            });
        }
        variants
    }

    fn fields(&mut self, fields: &'a [ast::Field], allow_defaults: bool) -> (Vec<FieldDef>, Vec<Option<Expr>>) {
        let mut defs: Vec<FieldDef> = Vec::new();
        let mut defaults = Vec::new();
        for field in fields {
            if defs.iter().any(|f| f.name == field.name.name) {
                self.error(
                    field.name.span,
                    format!("the field `{}` is declared twice", field.name.name),
                );
                continue;
            }
            if !field.attrs.is_empty() {
                self.unsupported(field.attrs[0].span, "attributes on fields are");
            }
            let ty = self.resolve_type(&field.ty);
            let default = match &field.default {
                Some(default) if allow_defaults => self.const_expr(default, Some(&ty)).map(|e| self.coerce(e, &ty)),
                Some(default) => {
                    self.unsupported(default.span, "default values here are");
                    None
                }
                None => None,
            };
            defs.push(FieldDef {
                name: field.name.name.clone(),
                ty,
            });
            defaults.push(default);
        }
        (defs, defaults)
    }

    /// A struct or enum that holds itself by value has no finite size. The specification
    /// places such occurrences on the heap (Section 4.7); this compiler does so only
    /// through a list, and reports the other cases.
    fn reject_recursive_types(&mut self, file: &ast::File) {
        #[derive(Clone, Copy, PartialEq)]
        enum State {
            Visiting,
            Done,
        }
        fn contains(checker: &Checker<'_>, ty: &Ty, states: &mut HashMap<Owner, State>) -> bool {
            match ty {
                Ty::Optional(inner) | Ty::Result(inner, _) => contains(checker, inner, states),
                Ty::Tuple(parts) => parts.iter().any(|part| contains(checker, part, states)),
                Ty::Struct(id) => visit(checker, Owner::Struct(*id), states),
                Ty::Enum(id) => visit(checker, Owner::Enum(*id), states),
                _ => false,
            }
        }
        fn visit(checker: &Checker<'_>, owner: Owner, states: &mut HashMap<Owner, State>) -> bool {
            match states.get(&owner) {
                Some(State::Visiting) => return true,
                Some(State::Done) => return false,
                None => {}
            }
            states.insert(owner, State::Visiting);
            let types: Vec<&Ty> = match owner {
                Owner::Struct(id) => checker.program.structs[id].fields.iter().map(|f| &f.ty).collect(),
                Owner::Enum(id) => checker.program.enums[id]
                    .variants
                    .iter()
                    .flat_map(|v| v.fields.iter().map(|f| &f.ty))
                    .collect(),
                _ => Vec::new(),
            };
            let cyclic = types.into_iter().any(|ty| contains(checker, ty, states));
            states.insert(owner, State::Done);
            cyclic
        }
        for item in &file.items {
            let (name, owner) = match &item.kind {
                ast::ItemKind::Type(decl) => match self.type_names.get(&decl.name.name) {
                    Some(TypeName::Struct(id)) => (&decl.name, Owner::Struct(*id)),
                    _ => continue,
                },
                ast::ItemKind::Enum(decl) => match self.type_names.get(&decl.name.name) {
                    Some(TypeName::Enum(id)) => (&decl.name, Owner::Enum(*id)),
                    _ => continue,
                },
                _ => continue,
            };
            let mut states = HashMap::new();
            if visit(self, owner, &mut states) {
                self.error_help(
                    name.span,
                    format!(
                        "`{}` contains itself, which is not supported by this compiler yet",
                        name.name
                    ),
                    "hold the inner values in a list (`[T]`), which this compiler does support",
                );
                // Break the cycle so that the layout of the type can still be computed.
                match owner {
                    Owner::Struct(id) => self.program.structs[id].fields.clear(),
                    Owner::Enum(id) => self.program.enums[id]
                        .variants
                        .iter_mut()
                        .for_each(|v| v.fields.clear()),
                    _ => {}
                }
            }
        }
    }

    pub(crate) fn resolve_type(&mut self, ty: &ast::Type) -> Ty {
        match &ty.kind {
            ast::TypeKind::Named { path, args } => {
                if path.len() != 1 {
                    self.unsupported(ty.span, "types from other packages are");
                    return Ty::Unknown;
                }
                let name = path[0].name.as_str();
                if args.is_empty()
                    && let Some(bound) = self.type_env.get(name)
                {
                    return bound.clone();
                }
                if self.generic_types.contains_key(name) {
                    let args: Vec<Ty> = args.iter().map(|arg| self.resolve_type(arg)).collect();
                    return self.instantiate_type(name, args, ty.span);
                }
                if self.interfaces.contains_key(name) {
                    self.error_help(
                        ty.span,
                        format!("the interface `{name}` cannot be used as a type here yet"),
                        "this compiler supports an interface as a constraint (`T: Name`) and as a parameter's type; interface values are not supported yet",
                    );
                    return Ty::Unknown;
                }
                if name == "List" {
                    if args.len() != 1 {
                        self.error(ty.span, "`List` takes one type argument: `List<T>`");
                        return Ty::Unknown;
                    }
                    return Ty::list(self.resolve_type(&args[0]));
                }
                if !args.is_empty() {
                    let mut known: Vec<String> = self.generic_types.keys().cloned().collect();
                    known.push("List".to_string());
                    let habit = suggest::type_habit(name);
                    self.error_near(ty.span, format!("unknown generic type `{name}`"), name, known, habit);
                    return Ty::Unknown;
                }
                match name {
                    "int" | "i64" => Ty::Int,
                    "float" | "f64" => Ty::Float,
                    "bool" => Ty::Bool,
                    "string" => Ty::Str,
                    "byte" | "u8" => Ty::Byte,
                    "Failure" => Ty::Failure,
                    "char" | "i8" | "i16" | "i32" | "u16" | "u32" | "u64" | "f32" => {
                        self.unsupported(ty.span, &format!("`{name}` is"));
                        Ty::Unknown
                    }
                    "Decimal" | "Duration" | "Time" | "Map" | "Set" | "Json" | "Channel" | "Shared" => {
                        self.unsupported(ty.span, &format!("`{name}` is"));
                        Ty::Unknown
                    }
                    _ => match self.type_names.get(name) {
                        Some(TypeName::Struct(id)) => Ty::Struct(*id),
                        Some(TypeName::Enum(id)) => Ty::Enum(*id),
                        Some(TypeName::Error(id)) => Ty::Error(*id),
                        Some(TypeName::Alias(ty)) => ty.clone(),
                        Some(TypeName::ErrorSet(_)) => {
                            self.error_help(
                                ty.span,
                                format!("`{name}` is an error set, not a type"),
                                "an error set is written after `!`",
                            );
                            Ty::Unknown
                        }
                        None => {
                            let mut known: Vec<String> = self.type_names.keys().cloned().collect();
                            known.extend(self.generic_types.keys().cloned());
                            known.extend(["int", "float", "bool", "string", "byte"].map(String::from));
                            let habit = suggest::type_habit(name);
                            self.error_near(ty.span, format!("unknown type `{name}`"), name, known, habit);
                            Ty::Unknown
                        }
                    },
                }
            }
            ast::TypeKind::List(element) => Ty::list(self.resolve_type(element)),
            ast::TypeKind::Optional(inner) => Ty::optional(self.resolve_type(inner)),
            ast::TypeKind::Result { ok, errors } => {
                let ok = self.resolve_type(ok);
                if matches!(ok, Ty::Result(..)) {
                    self.error(ty.span, "a result of a result cannot be written (Section 6.3)");
                }
                Ty::Result(Box::new(ok), self.resolve_error_set(errors))
            }
            ast::TypeKind::Tuple(types) if types.is_empty() => Ty::Unit,
            ast::TypeKind::Tuple(types) => {
                let types: Vec<Ty> = types.iter().map(|t| self.resolve_type(t)).collect();
                if types.iter().any(|t| t.is_unknown()) {
                    Ty::Unknown
                } else {
                    Ty::Tuple(types)
                }
            }
            ast::TypeKind::Map(key, value) => {
                // `[K: V]` is the prelude's `Map<K, V>` (Section 3.9).
                if !self.generic_types.contains_key("Map") {
                    self.unsupported(ty.span, "maps are");
                    return Ty::Unknown;
                }
                let args = vec![self.resolve_type(key), self.resolve_type(value)];
                self.instantiate_type("Map", args, ty.span)
            }
            ast::TypeKind::Array(..) => {
                self.unsupported(ty.span, "fixed arrays are");
                Ty::Unknown
            }
            ast::TypeKind::Fn {
                resource,
                params,
                ret,
                errors,
            } => {
                if *resource {
                    self.unsupported(ty.span, "`resource fn` types are");
                    return Ty::Unknown;
                }
                let params: Vec<(ParamKind, Ty)> = params
                    .iter()
                    .map(|(kind, ty)| (param_kind(*kind), self.resolve_type(ty)))
                    .collect();
                let ok = ret.as_ref().map(|ty| self.resolve_type(ty)).unwrap_or(Ty::Unit);
                let ret = if errors.is_empty() {
                    ok
                } else {
                    Ty::Result(Box::new(ok), self.resolve_error_set(errors))
                };
                Ty::Fn(Box::new(FnTy { params, ret }))
            }
            ast::TypeKind::Struct(_) => {
                self.unsupported(ty.span, "inline struct types are");
                Ty::Unknown
            }
        }
    }

    fn resolve_error_set(&mut self, types: &[ast::Type]) -> ErrSet {
        let mut ids = Vec::new();
        for ty in types {
            let ast::TypeKind::Named { path, args } = &ty.kind else {
                self.error(ty.span, "expected an error type");
                continue;
            };
            if path.len() != 1 || !args.is_empty() {
                self.unsupported(ty.span, "error types from other packages are");
                continue;
            }
            let name = path[0].name.as_str();
            if name == "Failure" {
                return ErrSet::Any;
            }
            match self.type_names.get(name) {
                Some(TypeName::Error(id)) => ids.push(*id),
                Some(TypeName::ErrorSet(ErrSet::Any)) => return ErrSet::Any,
                Some(TypeName::ErrorSet(ErrSet::Of(set))) => ids.extend(set),
                Some(_) => self.error_help(
                    ty.span,
                    format!("`{name}` is not an error type"),
                    "an error type is declared with `error` (Section 6.2)",
                ),
                None => self.error(ty.span, format!("unknown error type `{name}`")),
            }
        }
        ErrSet::of(ids)
    }

    /// A compile-time constant: a literal, possibly negated, or an empty list.
    fn const_expr(&mut self, expr: &ast::Expr, expected: Option<&Ty>) -> Option<Expr> {
        let literal = match &expr.kind {
            ast::ExprKind::Int(_)
            | ast::ExprKind::Float(_)
            | ast::ExprKind::Bool(_)
            | ast::ExprKind::RawStr(_)
            | ast::ExprKind::None => true,
            ast::ExprKind::Str(parts) => parts.iter().all(|p| matches!(p, ast::StrPart::Lit(_))),
            ast::ExprKind::List(items) => items.is_empty(),
            ast::ExprKind::Unary {
                op: ast::UnaryOp::Neg,
                expr: inner,
            } => {
                matches!(inner.kind, ast::ExprKind::Int(_) | ast::ExprKind::Float(_))
            }
            _ => false,
        };
        if !literal {
            self.unsupported(expr.span, "constant expressions other than literals are");
            return None;
        }
        Some(self.expr(expr, expected))
    }

    fn constant(&mut self, decl: &ast::ConstDecl) {
        let ty = decl.ty.as_ref().map(|ty| self.resolve_type(ty));
        let Some(mut value) = self.const_expr(&decl.value, ty.as_ref()) else {
            return;
        };
        if let Some(ty) = &ty {
            value = self.coerce(value, ty);
        }
        if !matches!(
            value.ty,
            Ty::Int | Ty::Float | Ty::Bool | Ty::Str | Ty::Byte | Ty::Unknown
        ) {
            self.unsupported(decl.value.span, "constants of this type are");
            return;
        }
        if self.consts.contains_key(&decl.name.name) || self.free_fns.contains_key(&decl.name.name) {
            self.error(decl.name.span, format!("`{}` is already declared", decl.name.name));
            return;
        }
        self.consts.insert(decl.name.name.clone(), value);
    }

    fn owner_of(&self, ty: &Ty) -> Option<Owner> {
        match ty {
            Ty::Struct(id) => Some(Owner::Struct(*id)),
            Ty::Enum(id) => Some(Owner::Enum(*id)),
            Ty::Error(id) => Some(Owner::Error(*id)),
            Ty::Str => Some(Owner::Str),
            Ty::Int => Some(Owner::Int),
            Ty::Float => Some(Owner::Float),
            Ty::Bool => Some(Owner::Bool),
            Ty::Byte => Some(Owner::Byte),
            _ => None,
        }
    }

    /// The parameters and return type of a function, resolved with the current type
    /// arguments. `implicit` gives the type of each parameter whose declared type is an
    /// interface, which makes it a type parameter (Section 4.9).
    fn params_and_ret(&mut self, decl: &'a ast::FnDecl, implicit: &HashMap<usize, Ty>) -> (Vec<ParamSig>, Ty) {
        let sig = &decl.sig;
        let mut params = Vec::new();
        for (index, param) in sig.params.iter().enumerate() {
            if param.variadic {
                self.unsupported(param.name.span, "variadic parameters are");
            }
            let mut default_index = None;
            if let Some(default) = &param.default {
                if param.kind != ast::ParamKind::Borrow {
                    self.error_help(
                        default.span,
                        "a `var` or `take` parameter cannot have a default",
                        "the caller gives up or lends a value of its own, so there must be one",
                    );
                } else if !is_constant(default) {
                    self.error_help(
                        default.span,
                        "a default must be a constant",
                        "a literal, `none`, `[]`, `[:]` or an enum variant; for anything else take a `T?` and use `else` in the body",
                    );
                } else {
                    default_index = Some(self.param_defaults.len());
                    self.param_defaults.push(default);
                }
            }
            if params.iter().any(|p: &ParamSig| p.name == param.name.name) {
                self.error(
                    param.name.span,
                    format!("the parameter `{}` is declared twice", param.name.name),
                );
            }
            let ty = match implicit.get(&index) {
                Some(ty) => ty.clone(),
                None => self.resolve_type(&param.ty),
            };
            let is_type_parameter = implicit.contains_key(&index)
                || matches!(&param.ty.kind, ast::TypeKind::Named { path, args }
                    if path.len() == 1 && args.is_empty() && self.type_env.contains_key(&path[0].name));
            let lent = param.kind == ast::ParamKind::Borrow && matches!(ty, Ty::Fn(_)) && !is_type_parameter;
            params.push(ParamSig {
                name: param.name.name.clone(),
                kind: param_kind(param.kind),
                ty,
                default: default_index,
                lent,
            });
        }
        let ok = sig.ret.as_ref().map(|ty| self.resolve_type(ty)).unwrap_or(Ty::Unit);
        let ret = match &sig.errors {
            ast::ErrorSpec::None => ok,
            ast::ErrorSpec::Set(types) => Ty::Result(Box::new(ok), self.resolve_error_set(types)),
            ast::ErrorSpec::Inferred => {
                self.unsupported(sig.name.span, "inferred error sets (`!` with nothing after it) are");
                ok
            }
        };
        (params, ret)
    }

    fn signature(&mut self, decl: &'a ast::FnDecl) -> Option<FnId> {
        let sig = &decl.sig;
        if self.declare_template(decl) {
            return None;
        }
        let (params, ret) = self.params_and_ret(decl, &HashMap::new());

        let id = self.sigs.len();
        let mut receiver = None;
        let display = match &sig.target {
            ast::FnTarget::Free => sig.name.name.clone(),
            ast::FnTarget::Method { .. } | ast::FnTarget::TypeLevel(_) => {
                let (ty, kind) = match &sig.target {
                    ast::FnTarget::Method { kind, ty } => (ty.as_ref(), Some(*kind)),
                    ast::FnTarget::TypeLevel(ty) => (Some(ty), None),
                    ast::FnTarget::Free => unreachable!(),
                };
                let ty = ty.map(|ty| self.resolve_type(ty)).unwrap_or(Ty::Unknown);
                let owner = self.owner_of(&ty).filter(|owner| {
                    self.in_prelude || matches!(owner, Owner::Struct(_) | Owner::Enum(_) | Owner::Error(_))
                });
                let Some(owner) = owner else {
                    if !ty.is_unknown() {
                        self.error_help(
                            sig.name.span,
                            format!("a function cannot be declared on `{}`", self.ty_name(&ty)),
                            "methods and type-level functions are declared on the types of this package (Sections 4.6 and 6.1)",
                        );
                    }
                    return None;
                };
                let display = format!("{}.{}", self.ty_name(&ty), sig.name.name);
                let key = (owner, sig.name.name.clone());
                if self.methods.contains_key(&key) || self.type_fns.contains_key(&key) {
                    self.error(sig.name.span, format!("`{display}` is already declared"));
                    return None;
                }
                match kind {
                    Some(kind) => {
                        // A map hashes its keys with `hash`, so a hand-written one has
                        // the signature a map expects.
                        if sig.name.name == "hash" && !self.in_prelude && (!params.is_empty() || ret != Ty::Int) {
                            self.error_help(
                                sig.name.span,
                                "a method named `hash` takes nothing and returns `int`",
                                "it is what a map or a set calls on a key (Section 4.9); give another meaning another name",
                            );
                        }
                        // `hash`, `compare` and `to_string` are called for the program by
                        // maps, by `<` and by text, on values it never offered to change.
                        let reads = param_kind(kind) == ParamKind::Borrow
                            && params.iter().all(|param| param.kind == ParamKind::Borrow);
                        if matches!(sig.name.name.as_str(), "hash" | "compare" | "to_string") && !reads {
                            self.error_help(
                                sig.name.span,
                                format!(
                                    "a method named `{}` only reads: it takes neither `var` nor `take`",
                                    sig.name.name
                                ),
                                "it is called where no change is expected: by `<`, by a map, or to make text (Sections 4.6 and 4.9)",
                            );
                        }
                        receiver = Some(ParamSig {
                            name: "self".to_string(),
                            kind: param_kind(kind),
                            ty,
                            default: None,
                            lent: false,
                        });
                        self.methods.insert(key, id);
                    }
                    None => {
                        self.type_fns.insert(key, id);
                    }
                }
                display
            }
        };
        if matches!(sig.target, ast::FnTarget::Free) {
            let name = &sig.name.name;
            if self.free_fns.contains_key(name) || self.consts.contains_key(name) {
                self.error_help(
                    sig.name.span,
                    format!("`{name}` is already declared"),
                    "there is no overloading: one name, one function (Section 6.1)",
                );
                return None;
            }
            if matches!(name.as_str(), "print" | "panic" | "all" | "race") {
                self.error(sig.name.span, format!("`{name}` is built in and cannot be declared"));
                return None;
            }
            if name.contains("__") && !self.in_prelude {
                self.error(
                    sig.name.span,
                    "a name with `__` in it is reserved for the standard library",
                );
                return None;
            }
            self.free_fns.insert(name.clone(), id);
        }
        self.sigs.push(FnSig {
            name: display.clone(),
            receiver,
            params,
            ret: ret.clone(),
        });
        self.program.functions.push(Function {
            name: display,
            params: 0,
            locals: Vec::new(),
            ret,
            body: Block::default(),
            span: sig.span,
            ..Function::default()
        });
        Some(id)
    }

    fn function(&mut self, id: FnId, decl: &'a ast::FnDecl) -> Function {
        let sig = self.sigs[id].clone();
        self.file = self.file_of(decl.sig.span);
        self.locals.clear();
        self.borrowed_fns.clear();
        self.lent_in_closure.clear();
        self.used.clear();
        self.declared_at.clear();
        self.scopes.clear();
        self.scopes.push(HashMap::new());
        self.ret = sig.ret.clone();
        self.loops = 0;
        // A default is checked where it is declared, whether or not a call uses it.
        for param in &sig.params {
            if let Some(default) = param.default {
                let default = self.param_defaults[default];
                let value = self.expr(default, Some(&param.ty));
                self.coerce(value, &param.ty);
            }
        }
        for param in sig.receiver.iter().chain(&sig.params) {
            let local = self.locals.len();
            self.locals.push(Local {
                name: param.name.clone(),
                ty: param.ty.clone(),
                mutable: param.kind == ParamKind::Var,
                kind: LocalKind::Param(param.kind),
            });
            self.used.push(true);
            self.declared_at.push(decl.sig.span);
            self.scopes[0].insert(param.name.clone(), local);
            if param.lent {
                self.borrowed_fns.insert(local);
            }
        }
        let params = self.locals.len();
        let body = match &decl.body {
            Some(body) => self.block(body),
            None => Block::default(),
        };
        let ok = match &sig.ret {
            Ty::Result(ok, _) => (**ok).clone(),
            other => other.clone(),
        };
        if !diverges(&body) && !matches!(ok, Ty::Unit | Ty::Unknown) {
            let end = decl.body.as_ref().map(|b| b.span.end).unwrap_or(decl.sig.span.end);
            self.error_help(
                Span {
                    start: end.saturating_sub(1),
                    end,
                },
                format!("`{}` can reach its end without returning a value", sig.name),
                format!(
                    "it returns `{}`: end every path with `return` or `fail`",
                    self.ty_name(&sig.ret)
                ),
            );
        }
        // A result may not be ignored, and binding it to a name that is never read ignores
        // it (Section 6.3).
        for local in params..self.locals.len() {
            if matches!(self.locals[local].ty, Ty::Result(..)) && !self.used[local] && self.locals[local].name != "_" {
                let name = self.locals[local].name.clone();
                self.error_help(
                    self.declared_at[local],
                    format!("the result in `{name}` is never handled"),
                    "propagate it with `?`, inspect it with `match`, or fall back with `else` (Section 6.4)",
                );
            }
        }
        Function {
            name: sig.name,
            params,
            locals: std::mem::take(&mut self.locals),
            ret: sig.ret,
            body,
            span: decl.sig.span,
            prelude: self.body_in_prelude,
            ..Function::default()
        }
    }

    // ---- scopes -------------------------------------------------------------------------

    fn declare(&mut self, name: &ast::Ident, ty: Ty, mutable: bool, kind: LocalKind) -> LocalId {
        let scope = self.scopes.last().unwrap();
        if scope.contains_key(&name.name) {
            self.error_help(
                name.span,
                format!("`{}` is already declared in this scope", name.name),
                "an inner scope may shadow a name; the same scope may not declare it twice (Section 3.5)",
            );
        }
        let id = self.declare_hidden(&name.name, ty, mutable, kind, true);
        self.declared_at[id] = name.span;
        id
    }

    fn declare_hidden(&mut self, name: &str, ty: Ty, mutable: bool, kind: LocalKind, visible: bool) -> LocalId {
        let id = self.locals.len();
        self.locals.push(Local {
            name: name.to_string(),
            ty,
            mutable,
            kind,
        });
        self.used.push(false);
        self.declared_at.push(Span::default());
        if visible {
            self.scopes.last_mut().unwrap().insert(name.to_string(), id);
        }
        id
    }

    fn lookup(&self, name: &str) -> Option<LocalId> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name).copied())
    }

    // ---- statements ---------------------------------------------------------------------

    /// A block of statements, which has no value.
    fn block(&mut self, block: &ast::Block) -> Block {
        self.scopes.push(HashMap::new());
        let mut stmts = Vec::new();
        for stmt in &block.stmts {
            self.stmt(stmt, &mut stmts);
        }
        self.scopes.pop();
        Block { stmts, value: None }
    }

    /// A block used for its value (Section 6.6): one expression, or statements that leave.
    fn value_block(&mut self, block: &ast::Block, expected: Option<&Ty>) -> Block {
        if let [
            ast::Stmt {
                kind: ast::StmtKind::Expr(expr),
                ..
            },
        ] = block.stmts.as_slice()
        {
            self.scopes.push(HashMap::new());
            let value = self.expr(expr, expected);
            self.scopes.pop();
            return Block {
                stmts: Vec::new(),
                value: Some(Box::new(value)),
            };
        }
        let checked = self.block(block);
        if !diverges(&checked) {
            self.error_help(
                Span {
                    start: block.span.end.saturating_sub(1),
                    end: block.span.end,
                },
                "this block has no value",
                "a block used for its value is one expression, or ends with `return`, `fail`, `break`, `continue` or `panic` (Section 6.6)",
            );
        }
        checked
    }

    /// Checks a statement, adding what it becomes to `out`. A tuple binding becomes several
    /// statements: one for the tuple, and one for each name it binds.
    pub(crate) fn stmt(&mut self, stmt: &ast::Stmt, out: &mut Vec<Stmt>) {
        if let ast::StmtKind::Let {
            mutable,
            pattern: pattern @ ast::BindPattern::Tuple(_, span),
            ty,
            value,
        } = &stmt.kind
        {
            let declared = ty.as_ref().map(|ty| self.resolve_type(ty));
            let mut init = self.expr(value, declared.as_ref());
            if let Some(declared) = &declared {
                init = self.coerce(init, declared);
            }
            let ty = init.ty.clone();
            let whole = self.declare_hidden("tuple", ty.clone(), false, LocalKind::Owned, false);
            self.used[whole] = true;
            out.push(Stmt::Let { local: whole, init });
            let source = Expr {
                kind: ExprKind::Local(whole),
                ty,
                span: *span,
            };
            self.destructure(pattern, source, *mutable, out);
            return;
        }
        if let ast::StmtKind::For {
            label,
            pattern: pattern @ ast::BindPattern::Tuple(_, span),
            take,
            iter,
            body,
        } = &stmt.kind
        {
            // `for (a, b) in pairs`: the loop variable is the pair, taken apart in the body.
            if let Some(label) = label {
                self.unsupported(label.span, "loop labels are");
            }
            if *take {
                self.unsupported(iter.span, "`for ... in take` loops are");
            }
            let list = self.expr(iter, None);
            let list = self.iterable(list);
            let element = match &list.ty {
                Ty::List(element) => (**element).clone(),
                Ty::Unknown => Ty::Unknown,
                other => {
                    let found = self.ty_name(other);
                    self.error(iter.span, format!("a `for` loop over `{found}` cannot bind a tuple"));
                    Ty::Unknown
                }
            };
            self.scopes.push(HashMap::new());
            let local = self.declare_hidden("pair", element.clone(), false, LocalKind::Element, false);
            self.used[local] = true;
            let mut stmts = Vec::new();
            let source = Expr {
                kind: ExprKind::Local(local),
                ty: element,
                span: *span,
            };
            self.destructure(pattern, source, false, &mut stmts);
            self.loops += 1;
            let checked = self.block(body);
            self.loops -= 1;
            self.scopes.pop();
            stmts.extend(checked.stmts);
            out.push(Stmt::ForList {
                local,
                list,
                body: Block { stmts, value: None },
            });
            return;
        }
        if let Some(checked) = self.stmt_one(stmt) {
            out.push(checked);
        }
    }

    /// Whether an expression is a map, found without reporting anything: the expression is
    /// checked again by whoever asked.
    fn is_map_expr(&self, expr: &ast::Expr) -> bool {
        self.peek_type(expr).is_some_and(|ty| self.map_types(&ty).is_some())
    }

    /// The type of a place (a variable, or a field or element of one), found by looking
    /// only: nothing is reported, and a lambda does not come to capture the variable.
    fn peek_type(&self, expr: &ast::Expr) -> Option<Ty> {
        match &expr.kind {
            ast::ExprKind::Paren(inner) => self.peek_type(inner),
            ast::ExprKind::Name(name) => match self.lookup(&name.name) {
                Some(local) => Some(self.locals[local].ty.clone()),
                // A variable of an enclosing function, when this is a lambda.
                None => self
                    .enclosing
                    .iter()
                    .rev()
                    .find_map(|outer| outer.local_type(&name.name)),
            },
            ast::ExprKind::Field { base, name } => match self.peek_type(base)? {
                ty @ Ty::Struct(id) if !self.is_prelude_type(&ty) || self.body_in_prelude => self.program.structs[id]
                    .fields
                    .iter()
                    .find(|field| field.name == name.name)
                    .map(|field| field.ty.clone()),
                _ => None,
            },
            ast::ExprKind::Index { base, .. } => match self.peek_type(base)? {
                Ty::List(element) => Some(*element),
                _ => None,
            },
            _ => None,
        }
    }

    /// Binds the names of a pattern to the parts of `source`.
    fn destructure(&mut self, pattern: &ast::BindPattern, source: Expr, mutable: bool, out: &mut Vec<Stmt>) {
        match pattern {
            ast::BindPattern::Name(name) => {
                let local = self.declare(name, source.ty.clone(), mutable, LocalKind::Owned);
                out.push(Stmt::Let { local, init: source });
            }
            ast::BindPattern::Wildcard(_) => {}
            ast::BindPattern::Tuple(parts, span) => match source.ty.clone() {
                Ty::Tuple(types) if types.len() == parts.len() => {
                    for (index, (part, ty)) in parts.iter().zip(types).enumerate() {
                        let field = Expr {
                            kind: ExprKind::Field(Box::new(source.clone()), index),
                            ty,
                            span: *span,
                        };
                        self.destructure(part, field, mutable, out);
                    }
                }
                Ty::Unknown | Ty::Never => {
                    for part in parts {
                        let unknown = self.unknown(*span);
                        self.destructure(part, unknown, mutable, out);
                    }
                }
                other => {
                    let found = self.ty_name(&other);
                    self.error_help(
                        *span,
                        format!(
                            "this pattern names {} elements, but the value is `{found}`",
                            parts.len()
                        ),
                        "a tuple is taken apart by a pattern with one name for each element (Section 4.5)",
                    );
                    // The names exist all the same, so that their uses are not errors too.
                    for part in parts {
                        let unknown = self.unknown(*span);
                        self.destructure(part, unknown, mutable, out);
                    }
                }
            },
        }
    }

    fn stmt_one(&mut self, stmt: &ast::Stmt) -> Option<Stmt> {
        match &stmt.kind {
            ast::StmtKind::Let {
                mutable,
                pattern,
                ty,
                value,
            } => {
                let declared = ty.as_ref().map(|ty| self.resolve_type(ty));
                let mut init = self.expr(value, declared.as_ref());
                if let Some(declared) = &declared {
                    init = self.coerce(init, declared);
                }
                let ty = declared.unwrap_or_else(|| init.ty.clone());
                let ty = if ty == Ty::Never { Ty::Unknown } else { ty };
                match pattern {
                    ast::BindPattern::Name(name) => {
                        let local = self.declare(name, ty, *mutable, LocalKind::Owned);
                        Some(Stmt::Let { local, init })
                    }
                    ast::BindPattern::Wildcard(span) => {
                        if matches!(init.ty, Ty::Result(..)) {
                            self.unhandled_result(&init, *span);
                        }
                        let local = self.declare_hidden("_", ty, false, LocalKind::Owned, false);
                        Some(Stmt::Let { local, init })
                    }
                    ast::BindPattern::Tuple(_, span) => {
                        self.unsupported(*span, "tuples are");
                        None
                    }
                }
            }
            ast::StmtKind::Assign {
                target:
                    ast::Expr {
                        kind: ast::ExprKind::Index { base, index },
                        ..
                    },
                op,
                value,
            } if self.is_map_expr(base) => {
                // `m[key] = value` adds the key or replaces its value (Section 4.5).
                let map = self.expr(base, None);
                let (key_ty, value_ty) = self.map_types(&map.ty).unwrap_or((Ty::Unknown, Ty::Unknown));
                let key = self.expr_as(index, &key_ty);
                let new = self.expr_as(value, &value_ty);
                if !matches!(op, ast::AssignOp::Set) {
                    self.error_help(
                        stmt.span,
                        "a map entry cannot be changed with a compound assignment",
                        "the key may be absent: use `m.update(key, start, value -> ...)` (Section 4.5)",
                    );
                    return None;
                }
                Some(Stmt::Expr(self.prelude_call("insert", vec![map, key, new], stmt.span)))
            }
            ast::StmtKind::Assign { target, op, value } => {
                // Assigning to a variable is not a use of the value it held.
                let assigned = match &target.kind {
                    ast::ExprKind::Name(name) => self.lookup(&name.name).map(|local| (local, self.used[local])),
                    _ => None,
                };
                let target_expr = self.expr(target, None);
                if let (Some((local, was_used)), true) = (assigned, matches!(op, ast::AssignOp::Set)) {
                    self.used[local] = was_used;
                }
                if !target_expr.ty.is_unknown() {
                    self.require_mutable_place(&target_expr, "assign to");
                }
                let op = match op {
                    ast::AssignOp::Set => None,
                    ast::AssignOp::Add => Some(BinOp::Add),
                    ast::AssignOp::Sub => Some(BinOp::Sub),
                    ast::AssignOp::Mul => Some(BinOp::Mul),
                    ast::AssignOp::Div => Some(BinOp::Div),
                };
                let value = self.expr(value, Some(&target_expr.ty));
                let value = self.coerce(value, &target_expr.ty);
                if let Some(op) = op {
                    let ok = match target_expr.ty {
                        Ty::Int | Ty::Float | Ty::Unknown => true,
                        Ty::Str => op == BinOp::Add,
                        _ => false,
                    };
                    if !ok {
                        self.error(
                            stmt.span,
                            format!("this operator is not defined for `{}`", self.ty_name(&target_expr.ty)),
                        );
                    }
                }
                Some(Stmt::Assign {
                    target: target_expr,
                    op,
                    value,
                    span: stmt.span,
                })
            }
            ast::StmtKind::Expr(expr) => {
                let checked = match &expr.kind {
                    ast::ExprKind::If(if_expr) => self.if_expr(if_expr, expr.span, false, None),
                    ast::ExprKind::Match { scrutinee, arms } => {
                        self.match_expr(scrutinee, arms, expr.span, false, None)
                    }
                    _ => self.expr(expr, None),
                };
                if matches!(checked.ty, Ty::Result(..)) {
                    self.unhandled_result(&checked, expr.span);
                }
                Some(Stmt::Expr(checked))
            }
            ast::StmtKind::Return(_)
            | ast::StmtKind::Fail(_)
            | ast::StmtKind::Break(_)
            | ast::StmtKind::Continue(_) => Some(Stmt::Expr(self.jump(stmt))),
            ast::StmtKind::While { label, cond, body } => {
                if let Some(label) = label {
                    self.unsupported(label.span, "loop labels are");
                }
                self.scopes.push(HashMap::new());
                let cond = self.cond(cond);
                let parts = std::mem::take(&mut self.cond_stmts);
                self.loops += 1;
                let mut body = self.block(body);
                body.stmts.splice(0..0, parts);
                self.loops -= 1;
                self.scopes.pop();
                Some(Stmt::While { cond, body })
            }
            ast::StmtKind::For {
                label,
                pattern,
                take,
                iter,
                body,
            } => {
                if let Some(label) = label {
                    self.unsupported(label.span, "loop labels are");
                }
                if *take {
                    self.unsupported(iter.span, "`for ... in take` loops are");
                }
                self.for_stmt(pattern, iter, body)
            }
            ast::StmtKind::Scope(_) | ast::StmtKind::Spawn(_) => {
                self.unsupported(stmt.span, "tasks are");
                None
            }
            ast::StmtKind::Defer(_) => {
                self.unsupported(stmt.span, "`defer` is");
                None
            }
            ast::StmtKind::Unsafe(_) => {
                self.unsupported(stmt.span, "`unsafe` is");
                None
            }
            ast::StmtKind::Select(_) => {
                self.unsupported(stmt.span, "`select` is");
                None
            }
        }
    }

    fn unhandled_result(&mut self, expr: &Expr, span: Span) {
        let Ty::Result(_, set) = &expr.ty else { return };
        let errors = match set {
            ErrSet::Any => "Failure".to_string(),
            ErrSet::Of(ids) => ids
                .iter()
                .map(|&id| self.program.errors[id].name.clone())
                .collect::<Vec<_>>()
                .join(", "),
        };
        self.error_help(
            span,
            format!("unhandled result: {errors}"),
            "propagate it with `?`, inspect it with `match`, or fall back with `else` (Section 6.4)",
        );
    }

    /// `return`, `fail`, `break` and `continue`, as an expression that never arrives.
    pub(crate) fn jump(&mut self, stmt: &ast::Stmt) -> Expr {
        let kind = match &stmt.kind {
            ast::StmtKind::Return(value) => {
                let ret = self.ret.clone();
                match value {
                    Some(value) => {
                        let hint = if ret.is_unknown() { None } else { Some(&ret) };
                        let value = self.expr(value, hint);
                        self.note_return(&value.ty);
                        ExprKind::Return(Some(Box::new(self.coerce(value, &ret))))
                    }
                    None => {
                        let unit = Expr {
                            kind: ExprKind::Unit,
                            ty: Ty::Unit,
                            span: stmt.span,
                        };
                        match &ret {
                            Ty::Unit | Ty::Unknown => ExprKind::Return(None),
                            _ => ExprKind::Return(Some(Box::new(self.coerce(unit, &ret)))),
                        }
                    }
                }
            }
            ast::StmtKind::Fail(value) => {
                let value = self.expr(value, None);
                match (&self.ret, &value.ty) {
                    (_, Ty::Unknown) => {}
                    (Ty::Result(_, set), Ty::Error(id)) => {
                        if !set.contains(*id) {
                            let name = self.program.errors[*id].name.clone();
                            self.error_help(
                                value.span,
                                format!("`{name}` is not in this function's error set"),
                                format!("add `{name}` after the `!` in the function's signature (Section 6.3)"),
                            );
                        }
                    }
                    (Ty::Result(_, ErrSet::Any), Ty::Failure) => {}
                    (Ty::Result(..), other) => {
                        let found = self.ty_name(other);
                        self.error_help(
                            value.span,
                            format!("`fail` takes an error, found `{found}`"),
                            "an error type is declared with `error` (Section 6.2)",
                        );
                    }
                    (Ty::Unknown, Ty::Error(id)) => {
                        // The return type of a lambda is being inferred.
                        self.note_errors(&ErrSet::of(vec![*id]));
                    }
                    (Ty::Unknown, _) => {}
                    (_, _) => self.error_help(
                        stmt.span,
                        "`fail` in a function that cannot fail",
                        "give the function an error set: `fn f() -> T ! E` (Section 6.1)",
                    ),
                }
                ExprKind::Fail(Box::new(value))
            }
            ast::StmtKind::Break(label) | ast::StmtKind::Continue(label) => {
                if let Some(label) = label {
                    self.unsupported(label.span, "loop labels are");
                }
                if self.loops == 0 {
                    self.error(stmt.span, "`break` and `continue` appear only inside a loop");
                }
                if matches!(stmt.kind, ast::StmtKind::Break(_)) {
                    ExprKind::Break
                } else {
                    ExprKind::Continue
                }
            }
            _ => unreachable!("not a jump"),
        };
        Expr {
            kind,
            ty: Ty::Never,
            span: stmt.span,
        }
    }

    /// The condition of `if` and `while`. A binding it makes is declared in the current scope.
    fn cond(&mut self, cond: &ast::Cond) -> Cond {
        match cond {
            ast::Cond::Expr(expr) => {
                let checked = self.expr(expr, Some(&Ty::Bool));
                Cond::Expr(self.coerce(checked, &Ty::Bool))
            }
            ast::Cond::Bind { pattern, value } => {
                let value = self.expr(value, None);
                let inner = match &value.ty {
                    Ty::Optional(inner) => (**inner).clone(),
                    Ty::Unknown => Ty::Unknown,
                    other => {
                        let found = self.ty_name(other);
                        self.error_help(
                            value.span,
                            format!("a binding condition takes an optional, found `{found}`"),
                            "`if x := e` runs when `e`, a `T?`, holds a value (Section 6.7)",
                        );
                        Ty::Unknown
                    }
                };
                let local = match pattern {
                    ast::BindPattern::Name(name) => self.declare(name, inner, false, LocalKind::Owned),
                    ast::BindPattern::Wildcard(_) => self.declare_hidden("_", inner, false, LocalKind::Owned, false),
                    ast::BindPattern::Tuple(_, span) => {
                        // `if (a, b) := e`: the condition binds the tuple, and the block
                        // begins by taking it apart.
                        let whole = self.declare_hidden("tuple", inner.clone(), false, LocalKind::Owned, false);
                        self.used[whole] = true;
                        let source = Expr {
                            kind: ExprKind::Local(whole),
                            ty: inner,
                            span: *span,
                        };
                        let mut parts = Vec::new();
                        self.destructure(pattern, source, false, &mut parts);
                        self.cond_stmts = parts;
                        whole
                    }
                };
                Cond::Bind { local, value }
            }
        }
    }

    fn for_stmt(&mut self, pattern: &ast::BindPattern, iter: &ast::Expr, body: &ast::Block) -> Option<Stmt> {
        let name = match pattern {
            ast::BindPattern::Name(name) => Some(name),
            ast::BindPattern::Wildcard(_) => None,
            ast::BindPattern::Tuple(_, span) => {
                self.unsupported(*span, "tuples are");
                None
            }
        };
        let range = match &iter.kind {
            ast::ExprKind::Binary {
                op: op @ (ast::BinaryOp::Range | ast::BinaryOp::RangeInclusive),
                lhs,
                rhs,
            } => Some((lhs, rhs, *op == ast::BinaryOp::RangeInclusive)),
            ast::ExprKind::Paren(inner) => match &inner.kind {
                ast::ExprKind::Binary {
                    op: op @ (ast::BinaryOp::Range | ast::BinaryOp::RangeInclusive),
                    lhs,
                    rhs,
                } => Some((lhs, rhs, *op == ast::BinaryOp::RangeInclusive)),
                _ => None,
            },
            _ => None,
        };
        if let Some((lo, hi, inclusive)) = range {
            let lo = self.expr(lo, Some(&Ty::Int));
            let lo = self.coerce(lo, &Ty::Int);
            let hi = self.expr(hi, Some(&Ty::Int));
            let hi = self.coerce(hi, &Ty::Int);
            self.scopes.push(HashMap::new());
            let local = match name {
                Some(name) => self.declare(name, Ty::Int, false, LocalKind::Owned),
                None => self.declare_hidden("_", Ty::Int, false, LocalKind::Owned, false),
            };
            self.loops += 1;
            let body = self.block(body);
            self.loops -= 1;
            self.scopes.pop();
            return Some(Stmt::ForRange {
                local,
                lo,
                hi,
                inclusive,
                body,
            });
        }
        let list = self.expr(iter, None);
        let list = self.iterable(list);
        let element = match &list.ty {
            Ty::List(element) => (**element).clone(),
            Ty::Unknown => Ty::Unknown,
            other => {
                let found = self.ty_name(other);
                self.error_help(
                    iter.span,
                    format!("a `for` loop over `{found}` is not supported by this compiler yet"),
                    "it loops over lists and integer ranges so far",
                );
                Ty::Unknown
            }
        };
        self.scopes.push(HashMap::new());
        let local = match name {
            Some(name) => self.declare(name, element, false, LocalKind::Element),
            None => self.declare_hidden("_", element, false, LocalKind::Element, false),
        };
        self.loops += 1;
        let body = self.block(body);
        self.loops -= 1;
        self.scopes.pop();
        Some(Stmt::ForList { local, list, body })
    }

    // ---- places -------------------------------------------------------------------------

    /// Reports an error unless `expr` is storage that may be changed: a `var` binding, or a
    /// field or element of one (Section 5.2).
    pub(crate) fn require_mutable_place(&mut self, expr: &Expr, action: &str) -> bool {
        let mut current = expr;
        loop {
            match &current.kind {
                ExprKind::Local(id) => {
                    let local = &self.locals[*id];
                    if local.mutable {
                        return true;
                    }
                    let name = local.name.clone();
                    let help = match local.kind {
                        LocalKind::Owned => format!("declare it with `var {name} := ...`"),
                        LocalKind::Param(_) => {
                            format!(
                                "declare the parameter as `{name}: var T`, and pass it as `var {name}` (Section 5.3)"
                            )
                        }
                        LocalKind::Element => {
                            "a loop variable names an element as it was; change the list through an index".to_string()
                        }
                        LocalKind::Capture { .. } => {
                            "a closure holds a copy of the variable, so assigning to it would change nothing outside; return the new value instead (Section 5.7)".to_string()
                        }
                    };
                    self.error_help(expr.span, format!("cannot {action} `{name}`, which is not `var`"), help);
                    return false;
                }
                ExprKind::Field(base, _)
                    if !matches!(base.ty, Ty::Error(_) | Ty::Failure)
                        && (self.body_in_prelude || !self.is_prelude_type(&base.ty)) =>
                {
                    current = base
                }
                ExprKind::Index(base, _) => current = base,
                _ => {
                    self.error_help(
                        expr.span,
                        format!("cannot {action} this expression"),
                        "only a `var` binding, or a field or element of one, can be changed (Section 5.2)",
                    );
                    return false;
                }
            }
        }
    }

    // ---- conversions --------------------------------------------------------------------

    /// Uses `expr` where `expected` is required, applying the widenings of Section 4.
    pub(crate) fn coerce(&mut self, expr: Expr, expected: &Ty) -> Expr {
        if expr.ty == *expected || expr.ty == Ty::Never || expr.ty.is_unknown() || expected.is_unknown() {
            return expr;
        }
        let span = expr.span;
        match expected {
            Ty::Failure if matches!(expr.ty, Ty::Error(_)) => {
                return Expr {
                    kind: ExprKind::Retype(Box::new(expr)),
                    ty: Ty::Failure,
                    span,
                };
            }
            Ty::Optional(inner) if self.can_coerce(&expr.ty, inner) => {
                let value = self.coerce(expr, inner);
                return Expr {
                    kind: ExprKind::WrapSome(Box::new(value)),
                    ty: expected.clone(),
                    span,
                };
            }
            Ty::Result(ok, set) => {
                if let Ty::Result(found_ok, found_set) = &expr.ty {
                    if found_ok == ok && found_set.within(set) {
                        return Expr {
                            kind: ExprKind::Retype(Box::new(expr)),
                            ty: expected.clone(),
                            span,
                        };
                    }
                } else if self.can_coerce(&expr.ty, ok) {
                    let value = self.coerce(expr, ok);
                    return Expr {
                        kind: ExprKind::WrapOk(Box::new(value)),
                        ty: expected.clone(),
                        span,
                    };
                }
            }
            _ => {}
        }
        let (want, found) = (self.ty_name(expected), self.ty_name(&expr.ty));
        let help = match (expected, &expr.ty) {
            (Ty::Float, Ty::Int) => {
                Some("there is no implicit conversion: write `float.from(x)`, or `1.0` for a literal")
            }
            (Ty::Int, Ty::Float) => Some("choose the rounding: `x.round()`, `x.floor()`, `x.ceil()` or `x.trunc()`"),
            (_, Ty::Optional(inner)) if **inner == *expected => {
                Some("the value may be absent: use `?`, `else` or `match` (Section 4.8)")
            }
            (_, Ty::Result(inner, _)) if **inner == *expected => {
                Some("the value may be an error: use `?`, `else` or `match` (Section 6.4)")
            }
            (Ty::Result(..), Ty::Result(..)) => Some("the errors of the value are not all in the expected error set"),
            _ => None,
        };
        let message = format!("expected `{want}`, found `{found}`");
        match help {
            Some(help) => self.error_help(span, message, help),
            None => self.error(span, message),
        }
        Expr {
            kind: expr.kind,
            ty: Ty::Unknown,
            span,
        }
    }

    fn can_coerce(&self, found: &Ty, expected: &Ty) -> bool {
        if found == expected || *found == Ty::Never || found.is_unknown() || expected.is_unknown() {
            return true;
        }
        match expected {
            Ty::Failure => matches!(found, Ty::Error(_)),
            Ty::Optional(inner) => self.can_coerce(found, inner),
            Ty::Result(ok, set) => match found {
                Ty::Result(found_ok, found_set) => found_ok == ok && found_set.within(set),
                _ => self.can_coerce(found, ok),
            },
            _ => false,
        }
    }
}

fn is_builtin_type(name: &str) -> bool {
    matches!(
        name,
        "List"
            | "Map"
            | "Set"
            | "Failure"
            | "Decimal"
            | "Duration"
            | "Time"
            | "Json"
            | "Channel"
            | "Shared"
            | "Any"
            | "Self"
    )
}

fn param_kind(kind: ast::ParamKind) -> ParamKind {
    match kind {
        ast::ParamKind::Borrow => ParamKind::Borrow,
        ast::ParamKind::Var => ParamKind::Var,
        ast::ParamKind::Take => ParamKind::Take,
    }
}

/// Whether control never reaches the end of the block.
pub(crate) fn diverges(block: &Block) -> bool {
    block.stmts.iter().any(|stmt| match stmt {
        Stmt::Expr(expr) => expr.ty == Ty::Never,
        Stmt::Let { init, .. } => init.ty == Ty::Never,
        // `while true` ends only by a `break` of its own.
        Stmt::While {
            cond: Cond::Expr(Expr {
                kind: ExprKind::Bool(true),
                ..
            }),
            body,
        } => !breaks(body),
        _ => false,
    }) || block.value.as_ref().is_some_and(|value| value.ty == Ty::Never)
}

/// Whether the block holds a `break` that leaves the loop the block is the body of: one
/// that is not inside a loop nested in it.
fn breaks(block: &Block) -> bool {
    fn in_expr(expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Break => true,
            ExprKind::If { cond, then, otherwise } => {
                let in_cond = match &**cond {
                    Cond::Expr(expr) => in_expr(expr),
                    Cond::Bind { value, .. } => in_expr(value),
                };
                in_cond || breaks(then) || otherwise.as_ref().is_some_and(breaks)
            }
            ExprKind::Match { scrutinee, arms } => {
                in_expr(scrutinee)
                    || arms
                        .iter()
                        .any(|arm| arm.guard.as_ref().is_some_and(in_expr) || breaks(&arm.body))
            }
            ExprKind::Else { expr, alt, .. } => in_expr(expr) || breaks(alt),
            ExprKind::Field(inner, _)
            | ExprKind::Unary(_, inner)
            | ExprKind::WrapSome(inner)
            | ExprKind::WrapOk(inner)
            | ExprKind::Retype(inner)
            | ExprKind::Try(inner)
            | ExprKind::Fail(inner)
            | ExprKind::Panic(inner)
            | ExprKind::Return(Some(inner)) => in_expr(inner),
            ExprKind::Index(a, b) | ExprKind::Binary(_, a, b) | ExprKind::And(a, b) | ExprKind::Or(a, b) => {
                in_expr(a) || in_expr(b)
            }
            ExprKind::Call { args, .. } => args.iter().any(|arg| in_expr(&arg.expr)),
            ExprKind::Builtin(_, values)
            | ExprKind::StructLit(_, values)
            | ExprKind::EnumLit(_, _, values)
            | ExprKind::ErrorLit(_, values)
            | ExprKind::ListLit(values)
            | ExprKind::TupleLit(values) => values.iter().any(in_expr),
            ExprKind::Interp(parts) => parts.iter().any(|part| match part {
                InterpPart::Lit(_) => false,
                InterpPart::Value(value, _) => in_expr(value),
            }),
            _ => false,
        }
    }
    block.stmts.iter().any(|stmt| match stmt {
        Stmt::Let { init, .. } => in_expr(init),
        Stmt::Assign { target, value, .. } => in_expr(target) || in_expr(value),
        Stmt::Expr(expr) => in_expr(expr),
        // A `break` inside a nested loop leaves that loop, not this one.
        Stmt::While { .. } | Stmt::ForRange { .. } | Stmt::ForList { .. } => false,
    }) || block.value.as_ref().is_some_and(|value| in_expr(value))
}

/// Whether an expression can be the default of a parameter: a value that is the same
/// wherever the function is called from (Section 5.3).
fn is_constant(expr: &ast::Expr) -> bool {
    match &expr.kind {
        ast::ExprKind::Int(_)
        | ast::ExprKind::Float(_)
        | ast::ExprKind::Bool(_)
        | ast::ExprKind::RawStr(_)
        | ast::ExprKind::None => true,
        ast::ExprKind::Str(parts) => parts.iter().all(|part| matches!(part, ast::StrPart::Lit(_))),
        ast::ExprKind::List(items) => items.iter().all(is_constant),
        ast::ExprKind::Map(entries) => entries
            .iter()
            .all(|(key, value)| is_constant(key) && is_constant(value)),
        ast::ExprKind::Unary {
            op: ast::UnaryOp::Neg,
            expr,
        } => matches!(expr.kind, ast::ExprKind::Int(_) | ast::ExprKind::Float(_)),
        // An enum variant without fields: `Level.Info`.
        ast::ExprKind::Field { base, name } => {
            matches!(&base.kind, ast::ExprKind::Name(ty) if is_type_like(&ty.name)) && is_type_like(&name.name)
        }
        _ => false,
    }
}

fn is_type_like(name: &str) -> bool {
    name.chars().next().is_some_and(char::is_uppercase)
}
