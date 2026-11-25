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

mod actors;
mod calls;
mod exprs;
mod fuse;
mod generics;
mod lambdas;
mod maps;
mod patterns;
mod suggest;
mod time;

/// Checks a program. `prelude` is the standard prelude, whose declarations are in scope in
/// every program (Section 4.1).
///
/// `extra` is somewhere to keep the functions the checker writes for itself (see `fuse`),
/// which must live as long as the two files do.
pub fn check<'a>(
    prelude: &'a ast::File,
    file: &'a ast::File,
    derived: &'a ast::File,
    problems: Vec<Diagnostic>,
    extra: &'a OnceCell<ast::File>,
    bases: &[u32],
    test_mode: bool,
) -> (Program, Vec<Diagnostic>) {
    let mut checker = Checker::new(extra);
    checker.program.test_mode = test_mode;
    // What could not be written for the program, such as a message that cannot be sent.
    checker.diags = problems;
    checker.file_bases = bases.to_vec();
    checker.in_prelude = true;
    let prelude_bodies = checker.declare_file(prelude);
    checker.prelude_types = (checker.program.structs.len(), checker.program.errors.len());
    checker.prelude_enums = checker.program.enums.len();
    checker.in_prelude = false;
    checker.note_declarations(file);
    let bodies = checker.declare_file(file);
    // What the compiler wrote for the program's types (`derive.rs`) is declared after
    // them, and stands with the prelude: it may use what only the prelude may, and a
    // mistake in it is the compiler's.
    checker.in_prelude = true;
    let mut prelude_bodies = prelude_bodies;
    prelude_bodies.extend(checker.declare_file(derived));
    checker.in_prelude = false;
    checker.finish(file, prelude_bodies, bodies);
    // What goes wrong in a function the compiler wrote for a type follows from a mistake
    // in the type, which is the one to report.
    let written = |d: &Diagnostic| d.message.starts_with("in `the prelude's ");
    if checker.diags.iter().any(|d| !written(d)) {
        checker.diags.retain(|d| !written(d));
    }
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
    /// Where the parameter is declared, if the program declares it.
    at: Option<Span>,
}

/// Where a function of the program is declared, for the errors that point at it. The
/// prelude's functions have none: an error never points into the prelude.
#[derive(Debug, Clone, Copy)]
struct FnAt {
    name: Span,
    /// The return type, if one is written.
    ret: Option<Span>,
    /// The end of the signature, where a return type or an error set would be added.
    end: u32,
    /// Whether an error set is written.
    fails: bool,
}

impl FnAt {
    fn of(sig: &ast::FnSig) -> FnAt {
        FnAt {
            name: sig.name.span,
            ret: sig.ret.as_ref().map(|ty| ty.span),
            end: sig.span.end,
            fails: !matches!(sig.errors, ast::ErrorSpec::None),
        }
    }

    /// The place just after the signature.
    fn after(&self) -> Span {
        Span {
            start: self.end,
            end: self.end,
        }
    }
}

#[derive(Debug, Clone)]
struct FnSig {
    name: String,
    /// The receiver, if the function is a method. It is passed as the first argument.
    receiver: Option<ParamSig>,
    params: Vec<ParamSig>,
    ret: Ty,
    at: Option<FnAt>,
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
    /// For a variable that holds an error caught from a result (`! e`, `else e =>`) of
    /// more than one kind: the errors it may be, so that `fail e` can pass it on.
    caught: HashMap<LocalId, ErrSet>,
    /// For the lambda about to be checked, whose return type is to be found: the errors
    /// the function it is passed to lets it fail with.
    lambda_errors: Option<ErrSet>,
    scopes: Vec<HashMap<String, LocalId>>,
    ret: Ty,
    loops: usize,
    /// Where the function being checked is declared, if it is one of the program's and
    /// not a lambda.
    here: Option<FnAt>,
    fn_name: String,
    /// The actor whose method each function is the body of, and the actor of the
    /// function being checked.
    actor_of: HashMap<FnId, &'a ast::ActorDecl>,
    actor: Option<&'a ast::ActorDecl>,
    /// The variables declared by a statement that `var` can be written before, by where
    /// their names are: `x := e` and `x: T = e`.
    plain_bindings: std::collections::HashSet<u32>,
    /// Where each name of the program is first declared: `type Name`, `fn name` and
    /// `fn Type.name`.
    first_at: HashMap<String, Span>,
    /// Where the fields of the program's types are declared, by type and field.
    field_at: HashMap<(String, String), Span>,
    // Prelude errors.
    overflow: ErrorId,
    parse_error: ErrorId,
    /// The first error type that is not predeclared.
    first_declared_error: ErrorId,
    /// How many struct types and how many error types the prelude declares: they come first.
    prelude_types: (StructId, ErrorId),
    prelude_enums: EnumId,
    /// The default values of parameters (Section 5.3).
    param_defaults: Vec<&'a ast::Expr>,
    /// Statements that take apart the tuple a condition bound: `if (a, b) := e`. They
    /// go first in the block the condition guards.
    cond_stmts: Vec<Stmt>,
    /// The standard packages each file imports, by the name the file knows each under:
    /// where it is imported, whether the file uses it, and the package's own name, which
    /// is another if the import says `as`. An import belongs to its file (Section 8.2).
    modules: HashMap<(usize, String), (Span, bool, String)>,
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
            caught: HashMap::new(),
            lambda_errors: None,
            declared_at: Vec::new(),
            here: None,
            fn_name: String::new(),
            actor_of: HashMap::new(),
            actor: None,
            plain_bindings: std::collections::HashSet::new(),
            first_at: HashMap::new(),
            field_at: HashMap::new(),
            scopes: Vec::new(),
            ret: Ty::Unit,
            loops: 0,
            overflow: 0,
            parse_error: 0,
            first_declared_error: 0,
            prelude_types: (0, 0),
            prelude_enums: 0,
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

    /// The error just reported, to say more about it. There is none while a body of the
    /// prelude is checked: such an error is reported at the call that needed the body,
    /// and a place inside the prelude would mean nothing there.
    fn reported(&mut self) -> Option<&mut Diagnostic> {
        if self.origin.is_some() {
            None
        } else {
            self.diags.last_mut()
        }
    }

    /// Says what the marks under the error just reported point out.
    fn label(&mut self, text: impl Into<String>) {
        if let Some(diagnostic) = self.reported() {
            diagnostic.label = Some(text.into());
        }
    }

    /// Points the error just reported at another place that explains it.
    fn related(&mut self, span: Option<Span>, text: impl Into<String>) {
        if let (Some(span), Some(diagnostic)) = (span, self.reported()) {
            diagnostic.related.push((span, text.into()));
        }
    }

    fn note(&mut self, text: impl Into<String>) {
        if let Some(diagnostic) = self.diags.last_mut() {
            diagnostic.notes.push(text.into());
        }
    }

    fn help(&mut self, text: impl Into<String>) {
        if let Some(diagnostic) = self.diags.last_mut() {
            diagnostic.help.push(text.into());
        }
    }

    /// Gives the error just reported its correction: `text` in place of `span`.
    fn fix(&mut self, message: impl Into<String>, span: Span, text: impl Into<String>) {
        self.fix_edits(message, vec![(span, text.into())]);
    }

    fn fix_edits(&mut self, message: impl Into<String>, edits: Vec<(Span, String)>) {
        if let Some(diagnostic) = self.reported() {
            diagnostic.fix = Some(koo_syntax::Fix {
                message: message.into(),
                edits,
            });
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
                        .map(|&id| self.ty_name(&Ty::Error(id)))
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
            Ty::Struct(id) => self.shown_name(&self.program.structs[*id].name, *id < self.prelude_types.0),
            Ty::Enum(id) => self.shown_name(&self.program.enums[*id].name, *id < self.prelude_enums),
            Ty::Error(id) => self.shown_name(&self.program.errors[*id].name, *id < self.prelude_types.1),
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

    /// A type's name as the program writes it: the prelude's `HttpRequest` is `http.Request`.
    /// The name of a function as a program writes it: the prelude's `http__json` is the
    /// `http.json` of a standard package. A function the prelude has for its own use
    /// (`j__float`) keeps its name, which no program sees.
    pub(crate) fn shown_fn(&self, name: &str) -> String {
        match name.split_once("__") {
            Some((package, function))
                if self.in_prelude && package.len() > 1 && !function.is_empty() && !function.contains("__") =>
            {
                format!("{package}.{function}")
            }
            _ => name.to_string(),
        }
    }

    pub(crate) fn shown_name(&self, name: &str, prelude: bool) -> String {
        for (prefix, package) in [("Http", "http"), ("Sql", "sql"), ("Log", "log")] {
            if let Some(rest) = name.strip_prefix(prefix)
                && prelude
                && rest.starts_with(char::is_uppercase)
            {
                return format!("{package}.{rest}");
            }
        }
        name.to_string()
    }

    /// The prelude's name for a type of a standard package that the file imports:
    /// `http.Request` is `HttpRequest`. The import counts as used.
    pub(crate) fn package_type(&mut self, path: &[ast::Ident]) -> Option<String> {
        let [package, name] = path else { return None };
        let file = self.file_of(package.span);
        let own = self.modules.get(&(file, package.name.clone()))?.2.clone();
        let mut letters = own.chars();
        let first = letters.next()?.to_ascii_uppercase();
        let full = format!("{first}{}{}", letters.as_str(), name.name);
        if !self.type_names.contains_key(&full) {
            return None;
        }
        self.modules.get_mut(&(file, package.name.clone()))?.1 = true;
        Some(full)
    }

    // ---- declarations -------------------------------------------------------------------

    /// Records where the program declares its names, for the errors that point there.
    fn note_declarations(&mut self, file: &ast::File) {
        for item in &file.items {
            let mut fields: Vec<(String, &[ast::Field])> = Vec::new();
            let name = match &item.kind {
                ast::ItemKind::Type(decl) => {
                    if let ast::TypeBody::Struct(list) = &decl.body {
                        fields.push((decl.name.name.clone(), list));
                    }
                    &decl.name
                }
                ast::ItemKind::Enum(decl) => {
                    for variant in &decl.variants {
                        if let Some(list) = &variant.fields {
                            fields.push((variant.name.name.clone(), list));
                            fields.push((format!("{}.{}", decl.name.name, variant.name.name), list));
                        }
                    }
                    &decl.name
                }
                ast::ItemKind::Error(decl) => {
                    if let ast::ErrorBody::Fields(list) = &decl.body {
                        fields.push((decl.name.name.clone(), list));
                    }
                    &decl.name
                }
                ast::ItemKind::Interface(decl) => &decl.name,
                ast::ItemKind::Actor(decl) => &decl.name,
                ast::ItemKind::Fn(decl) => {
                    self.first_at.entry(fn_key(&decl.sig)).or_insert(decl.sig.name.span);
                    continue;
                }
                ast::ItemKind::Const(decl) => {
                    self.first_at
                        .entry(format!("fn {}", decl.name.name))
                        .or_insert(decl.name.span);
                    continue;
                }
                _ => continue,
            };
            self.first_at.entry(format!("type {}", name.name)).or_insert(name.span);
            for (owner, list) in fields {
                for field in list {
                    self.field_at
                        .entry((owner.clone(), field.name.name.clone()))
                        .or_insert(field.name.span.to(field.ty.span));
                }
            }
        }
    }

    /// Reports a name that is declared a second time, and points at the first.
    fn already_declared(&mut self, span: Span, name: &str, key: &str) {
        self.error(span, format!("`{name}` is already declared"));
        self.label("declared a second time");
        let first = self.first_at.get(key).copied().filter(|first| *first != span);
        self.related(first, format!("`{name}` is first declared here"));
    }

    /// Reads the declarations of one source: its types, constants and function
    /// signatures. Returns the functions whose bodies are to be checked.
    fn declare_file(&mut self, file: &'a ast::File) -> Vec<(FnId, &'a ast::FnDecl)> {
        for import in &file.imports {
            // The standard packages that exist so far are part of the prelude; an import
            // brings one's name into scope (Section 8.2).
            match &import.kind {
                ast::ImportKind::Package { path, .. }
                    if matches!(
                        path.as_str(),
                        "std/os"
                            | "std/io"
                            | "std/test"
                            | "std/http"
                            | "std/time"
                            | "std/sql"
                            | "std/sql/postgres"
                            | "std/hash"
                            | "std/encoding"
                            | "std/crypto"
                            | "std/random"
                            | "std/log"
                    ) =>
                {
                    let key = (self.file_of(import.span), import.binding.clone());
                    let own = path.rsplit('/').next().unwrap_or(path).to_string();
                    if self.modules.insert(key, (import.span, false, own)).is_some() {
                        self.error(import.span, format!("`{}` is imported twice", import.binding));
                        self.fix("remove the second import", import.span, "");
                    }
                }
                ast::ImportKind::Package { path, .. } if path.starts_with("std/") => {
                    self.error_help(
                        import.span,
                        format!("the package `{path}` is not part of this compiler yet"),
                        "the standard packages so far are `std/os`, `std/io`, `std/time`, `std/http`, `std/sql`, `std/sql/postgres`, `std/hash`, `std/encoding`, `std/crypto`, `std/random`, `std/log` and `std/test`",
                    );
                }
                _ => self.unsupported(import.span, "imports of other packages are"),
            }
        }

        // 1. Every type name, so that declarations may refer to one another in any order.
        for item in &file.items {
            let (name, kind) = match &item.kind {
                ast::ItemKind::Actor(decl) => {
                    self.declare_actor(decl);
                    continue;
                }
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
                        let key = format!("type {}", decl.name.name);
                        self.already_declared(decl.name.span, &decl.name.name, &key);
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
                self.already_declared(name.span, &name.name, &format!("type {}", name.name));
                if is_builtin_type(&name.name) || matches!(name.name.as_str(), "Duration" | "Time") {
                    self.note(format!("`{}` is a type of the language", name.name));
                }
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
                ast::ItemKind::Actor(decl) => self.actor_fields(decl),
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
            if let ast::ItemKind::Actor(decl) = &item.kind {
                self.actor_methods(decl, &mut bodies);
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
            let texts = self.generic_to_strings();
            let equalities = self.generic_equalities();
            if !texts && !equalities {
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
            .filter(|(_, (_, used, _))| !used)
            .map(|((_, name), (span, _, _))| (name.clone(), *span))
            .collect();
        for (name, span) in unused {
            self.error(span, format!("the import `{name}` is not used"));
            self.note("an unused import is an error (Section 8.2)");
            self.fix("remove it", span, "");
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
                    let (has_params, at) = (!sig.params.is_empty(), sig.at);
                    self.error(span, "`main` has the wrong signature");
                    self.note(
                        "`main` is `fn main()` or `fn main() ! E`: it takes nothing and returns nothing (Section 6.1)",
                    );
                    if has_params {
                        self.help("the program's arguments come from `std/os`: `import std/os`, then `os.args()`");
                    }
                    if !unit {
                        self.related(at.and_then(|at| at.ret), "`main` returns nothing");
                        self.help("to end with a status, `import std/os` and call `os.exit(1)`; an error that `main` fails with ends the program with status 1");
                    }
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
                let first = decl.variants.iter().find(|v| v.name.name == variant.name.name);
                self.related(first.map(|v| v.name.span), "first declared here");
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
                let first = fields.iter().find(|f| f.name.name == field.name.name);
                self.related(first.map(|f| f.name.span), "first declared here");
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
                let in_package;
                let name = if path.len() == 1 {
                    path[0].name.as_str()
                } else {
                    // `http.Request`: a type of a standard package.
                    match self.package_type(path) {
                        Some(name) => {
                            in_package = name;
                            in_package.as_str()
                        }
                        None => {
                            self.unsupported(ty.span, "types from other packages are");
                            return Ty::Unknown;
                        }
                    }
                };
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
                        self.error(ty.span, format!("`{name}` is not supported by this compiler yet"));
                        match name {
                            "char" => self.help("a string gives its characters as strings, with `s.chars()`"),
                            "f32" => self.fix("use `float`, which is 64 bits", ty.span, "float"),
                            _ => self.fix("use `int`, which is 64 bits", ty.span, "int"),
                        }
                        Ty::Unknown
                    }
                    "Decimal" | "Map" | "Set" | "Json" | "Channel" | "Shared" => {
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
                self.result_of(ok, errors)
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
                    self.result_of(ok, errors)
                };
                Ty::Fn(Box::new(FnTy { params, ret }))
            }
            ast::TypeKind::Struct(_) => {
                self.unsupported(ty.span, "inline struct types are");
                Ty::Unknown
            }
        }
    }

    /// A type with an error set as written after it. The set is empty when it is a type
    /// parameter that stands for no error at all: then the type is all there is.
    pub(crate) fn result_of(&mut self, ok: Ty, errors: &[ast::Type]) -> Ty {
        match self.resolve_error_set(errors) {
            ErrSet::Of(ids) if ids.is_empty() => ok,
            set => Ty::Result(Box::new(ok), set),
        }
    }

    fn resolve_error_set(&mut self, types: &[ast::Type]) -> ErrSet {
        let mut ids = Vec::new();
        for ty in types {
            let ast::TypeKind::Named { path, args } = &ty.kind else {
                self.error(ty.span, "expected an error type");
                continue;
            };
            let in_package;
            let name = match (path.len(), args.is_empty()) {
                (1, true) => path[0].name.as_str(),
                // `http.Error`: an error of a standard package.
                (2, true) if let Some(name) = self.package_type(path) => {
                    in_package = name;
                    in_package.as_str()
                }
                _ => {
                    self.unsupported(ty.span, "error types from other packages are");
                    continue;
                }
            };
            if name == "Failure" {
                return ErrSet::Any;
            }
            // A type parameter in the place of an error stands for the errors of a
            // function that was passed (`check/generics.rs`).
            if let Some(bound) = self.type_env.get(name).cloned() {
                match generics::errors_of(&bound) {
                    Some(ErrSet::Any) => return ErrSet::Any,
                    Some(ErrSet::Of(set)) => ids.extend(set),
                    None => {
                        let found = self.ty_name(&bound);
                        self.error_help(
                            ty.span,
                            format!("`{name}` is `{found}` here, which is not an error type"),
                            "a type parameter written after `!` stands for errors (Section 6.3)",
                        );
                    }
                }
                continue;
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

    /// A compile-time constant: a literal, possibly negated, an empty list, or a variant
    /// of an enumeration that has no fields.
    fn const_expr(&mut self, expr: &ast::Expr, expected: Option<&Ty>) -> Option<Expr> {
        let literal = match &expr.kind {
            ast::ExprKind::Int(_)
            | ast::ExprKind::Float(_)
            | ast::ExprKind::Bool(_)
            | ast::ExprKind::RawStr(_)
            | ast::ExprKind::Duration(_)
            | ast::ExprKind::None => true,
            ast::ExprKind::Str(parts) => parts.iter().all(|p| matches!(p, ast::StrPart::Lit(_))),
            ast::ExprKind::List(items) => items.is_empty(),
            ast::ExprKind::Map(entries) => entries.is_empty(),
            ast::ExprKind::Unary {
                op: ast::UnaryOp::Neg,
                expr: inner,
            } => {
                matches!(inner.kind, ast::ExprKind::Int(_) | ast::ExprKind::Float(_))
            }
            // A variant of an enumeration, where one is expected: `Priority.Normal`.
            // What is wrong with it, if anything, is said as for any other value.
            ast::ExprKind::Field { base: on, .. } => {
                matches!((&on.kind, expected), (ast::ExprKind::Name(_), Some(Ty::Enum(_))))
            }
            _ => false,
        };
        if !literal {
            self.unsupported(expr.span, "constant expressions other than literals are");
            return None;
        }
        let value = self.expr(expr, expected);
        // Of an enumeration, only a variant without fields is a constant.
        if matches!(expr.kind, ast::ExprKind::Field { .. })
            && !value.ty.is_unknown()
            && !matches!(&value.kind, ExprKind::EnumLit(_, _, values) if values.is_empty())
        {
            self.unsupported(expr.span, "constant expressions other than literals are");
            return None;
        }
        Some(value)
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
        ) && !self.is_time_type(&value.ty)
        {
            self.unsupported(decl.value.span, "constants of this type are");
            return;
        }
        if self.consts.contains_key(&decl.name.name) || self.free_fns.contains_key(&decl.name.name) {
            self.already_declared(decl.name.span, &decl.name.name, &format!("fn {}", decl.name.name));
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
                let first = sig.params.iter().find(|p| p.name.name == param.name.name);
                self.related(first.map(|p| p.name.span), "first declared here");
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
                at: Some(param.name.span.to(param.ty.span)),
            });
        }
        let ok = sig.ret.as_ref().map(|ty| self.resolve_type(ty)).unwrap_or(Ty::Unit);
        let ret = match &sig.errors {
            ast::ErrorSpec::None => ok,
            ast::ErrorSpec::Set(types) => self.result_of(ok, types),
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
        let (mut params, ret) = self.params_and_ret(decl, &HashMap::new());
        if self.in_prelude {
            params.iter_mut().for_each(|param| param.at = None);
        }

        let id = self.sigs.len();
        let mut receiver = None;
        let display = match &sig.target {
            ast::FnTarget::Free => self.shown_fn(&sig.name.name),
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
                // The prelude's `HttpApp.get` is `http.App.get` to the program.
                let on = self.ty_name(&ty);
                let on = if self.in_prelude {
                    self.shown_name(&on, true)
                } else {
                    on
                };
                let display = format!("{on}.{}", sig.name.name);
                let key = (owner, sig.name.name.clone());
                if self.methods.contains_key(&key) || self.type_fns.contains_key(&key) {
                    self.already_declared(sig.name.span, &display, &fn_key(sig));
                    self.note("a type has one function of each name, whether it is a method or a type-level function");
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
                            at: None,
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
                self.already_declared(sig.name.span, name, &fn_key(sig));
                self.note("there is no overloading: one name, one function (Section 6.1)");
                self.help(
                    "give this function another name, or one function both jobs with a default value for a parameter",
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
            at: (!self.in_prelude).then(|| FnAt::of(sig)),
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
        self.caught.clear();
        self.scopes.clear();
        self.scopes.push(HashMap::new());
        self.ret = sig.ret.clone();
        self.here = sig.at;
        self.fn_name = sig.name.clone();
        self.actor = self.actor_of.get(&id).copied();
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
            self.declared_at.push(param.at.unwrap_or(decl.sig.span));
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
                "end every path with `return` or `fail`",
            );
            self.label("the function ends here");
            let returns = format!("it returns `{}`", self.ty_name(&ok));
            self.related(sig.at.and_then(|at| at.ret), returns);
        }
        // A result may not be ignored, and binding it to a name that is never read ignores
        // it (Section 6.3).
        for local in params..self.locals.len() {
            if matches!(self.locals[local].ty, Ty::Result(..)) && !self.used[local] && self.locals[local].name != "_" {
                let name = self.locals[local].name.clone();
                let found = self.ty_name(&self.locals[local].ty);
                self.error(
                    self.declared_at[local],
                    format!("the result in `{name}` is never handled"),
                );
                self.label(format!("bound to {}, and never looked at", a(&found)));
                self.note("a result is a value or an error, and the error may not be ignored (Section 6.4)");
                self.help("end the value with `?` to pass the error on, or with `else` to give a value in its place, or inspect it with `match`");
            }
        }
        Function {
            name: self.program.functions[id].name.clone(),
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
        if let Some(&first) = scope.get(&name.name) {
            self.error(name.span, format!("`{}` is already declared in this scope", name.name));
            self.label("declared a second time");
            let first_at = self.declared_at.get(first).copied().filter(|at| *at != Span::default());
            self.related(first_at, format!("`{}` is first declared here", name.name));
            if self.locals[first].mutable {
                self.help(format!(
                    "to give `{}` a new value, assign with `=`; `:=` declares a new variable",
                    name.name
                ));
            } else {
                self.help(format!(
                    "use another name, or declare the first as `var {}` and assign to it with `=`",
                    name.name
                ));
            }
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
                    let at = ty.as_ref().map(|ty| ty.span);
                    init = self.coerce_for(init, declared, at, "the binding declares this type");
                }
                let ty = declared.unwrap_or_else(|| init.ty.clone());
                let ty = if ty == Ty::Never { Ty::Unknown } else { ty };
                match pattern {
                    ast::BindPattern::Name(name) => {
                        let local = self.declare(name, ty, *mutable, LocalKind::Owned);
                        self.plain_bindings.insert(name.span.start);
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
            ast::StmtKind::Assign {
                target:
                    ast::Expr {
                        kind: ast::ExprKind::Name(name),
                        span: target_span,
                    },
                op: ast::AssignOp::Set,
                value,
            } if self.lambda.is_none()
                && self.lookup(&name.name).is_none()
                && !self.consts.contains_key(&name.name)
                && !self.free_fns.contains_key(&name.name)
                && suggest::name_habit(&name.name).is_none()
                && !suggest::has_near(&name.name, &self.names_in_scope()) =>
            {
                // `x = 5` where no `x` exists: the variable was meant to be declared.
                // It is declared, so that its uses below are not reported as well.
                let init = self.expr(value, None);
                self.error(name.span, format!("unknown name `{}`", name.name));
                self.label("no variable of this name is declared");
                self.note("`=` changes a variable that exists; `:=` declares one");
                let between = Span {
                    start: target_span.end,
                    end: value.span.start,
                };
                self.fix(format!("to declare `{}`, write `:=`", name.name), between, " := ");
                let ty = if init.ty == Ty::Never {
                    Ty::Unknown
                } else {
                    init.ty.clone()
                };
                let local = self.declare(name, ty, true, LocalKind::Owned);
                self.used[local] = true;
                Some(Stmt::Let { local, init })
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
                // `deadline += 5s`: the operators of durations and times are methods of
                // the prelude's, so the assignment is of what the method returns.
                if let Some(op) = op
                    && self.is_time_type(&target_expr.ty)
                {
                    let value = self.expr(value, None);
                    if value.ty.is_unknown() {
                        return None;
                    }
                    return match self.time_arithmetic(op, &target_expr, &value, stmt.span) {
                        Some(result) if result.ty == target_expr.ty => Some(Stmt::Assign {
                            target: target_expr,
                            op: None,
                            value: result,
                            span: stmt.span,
                        }),
                        Some(result) => {
                            let (found, wanted) = (self.ty_name(&result.ty), self.ty_name(&target_expr.ty));
                            self.error(
                                stmt.span,
                                format!("this gives a `{found}`, which cannot be assigned to a `{wanted}`"),
                            );
                            None
                        }
                        None => {
                            let (left, right) = (self.ty_name(&target_expr.ty), self.ty_name(&value.ty));
                            self.error_help(
                                stmt.span,
                                format!("this operator is not defined for `{left}` and `{right}`"),
                                "durations add and subtract, and are multiplied and divided by an `int`; a `Time` and a `Duration` add to a `Time` (Section 10.6)",
                            );
                            None
                        }
                    };
                }
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

    /// The errors of a set as they are written after `!`: `NotFound | Denied`.
    pub(crate) fn error_names(&self, set: &ErrSet) -> String {
        match set {
            ErrSet::Any => "Failure".to_string(),
            ErrSet::Of(ids) => ids
                .iter()
                .map(|&id| self.ty_name(&Ty::Error(id)))
                .collect::<Vec<_>>()
                .join(" | "),
        }
    }

    /// The errors of `set` that the function being checked does not declare.
    fn undeclared_errors(&self, set: &ErrSet) -> String {
        match (set, &self.ret) {
            (ErrSet::Of(ids), Ty::Result(_, ErrSet::Of(declared))) => ids
                .iter()
                .filter(|id| !declared.contains(id))
                .map(|&id| self.ty_name(&Ty::Error(id)))
                .collect::<Vec<_>>()
                .join(" | "),
            (_, Ty::Result(_, ErrSet::Any)) => String::new(),
            _ => self.error_names(set),
        }
    }

    /// Gives the error just reported the correction that lets the function being checked
    /// fail with `errors`, written as after `!`. A lambda has no signature to add them to.
    pub(crate) fn fix_error_set(&mut self, errors: &str) {
        let Some(here) = self.here else { return };
        let name = self.fn_name.clone();
        if errors.is_empty() {
            return;
        }
        if here.fails {
            self.fix(
                format!("or pass it on: add `{errors}` to the errors of `{name}`"),
                here.after(),
                format!(" | {errors}"),
            );
        } else {
            self.fix(
                format!("or pass it on: let `{name}` fail with `{errors}`"),
                here.after(),
                format!(" ! {errors}"),
            );
        }
    }

    fn unhandled_result(&mut self, expr: &Expr, span: Span) {
        let Ty::Result(_, set) = &expr.ty else { return };
        let set = set.clone();
        let errors = match &set {
            ErrSet::Any => "Failure".to_string(),
            ErrSet::Of(ids) => ids
                .iter()
                .map(|&id| self.ty_name(&Ty::Error(id)))
                .collect::<Vec<_>>()
                .join(", "),
        };
        let found = self.ty_name(&expr.ty);
        self.error(span, format!("unhandled result: {errors}"));
        self.label(format!("this is {}, which may be an error", a(&found)));
        self.note("a result is a value or an error, and the error may not be ignored (Section 6.4)");
        if let ExprKind::Local(local) = &expr.kind {
            // The result was bound to a name, and the name used as if it were the value.
            let name = self.locals[*local].name.clone();
            let at = self
                .declared_at
                .get(*local)
                .copied()
                .filter(|at| *at != Span::default());
            self.related(at, format!("`{name}` is bound to the result here"));
            self.help(format!(
                "handle the error where `{name}` is bound: end the value with `?` to pass the error on, or with `else` to give a value in its place"
            ));
            return;
        }
        let after = Span {
            start: span.end,
            end: span.end,
        };
        let undeclared = self.undeclared_errors(&set);
        if undeclared.is_empty() && matches!(self.ret, Ty::Result(..)) {
            self.help("to handle it here, give a value in its place with `else`, or inspect it with `match`");
            self.fix("or pass the error on to the caller with `?`", after, "?");
        } else {
            self.help("to handle it here, give a value in its place with `else`, or inspect it with `match`");
            if self.here.is_some() && !undeclared.is_empty() {
                // The two edits that pass it on: `?` here, and the error in the signature.
                let here = self.here.unwrap();
                let name = self.fn_name.clone();
                let add = if here.fails {
                    format!(" | {undeclared}")
                } else {
                    format!(" ! {undeclared}")
                };
                self.fix_edits(
                    format!("or pass the error on to the caller: write `?`, and let `{name}` fail with `{undeclared}`"),
                    vec![(here.after(), add), (after, "?".to_string())],
                );
            } else {
                self.help("or pass the error on to the caller: write `?` after the value");
            }
        }
    }

    /// A value as what the function returns. If it is not of the type the function
    /// declares, the error points at the declaration.
    pub(crate) fn returned(&mut self, value: Expr, ret: &Ty) -> Expr {
        let Some(here) = self.here else {
            return self.coerce(value, ret);
        };
        let ok = match ret {
            Ty::Result(ok, _) => (**ok).clone(),
            other => other.clone(),
        };
        if ok == Ty::Unit && !matches!(value.ty, Ty::Unit | Ty::Unknown | Ty::Never | Ty::Result(..)) {
            // The commonest form of this mistake: a function that was never given a
            // return type.
            let found = self.ty_name(&value.ty);
            let name = self.fn_name.clone();
            self.error(
                value.span,
                format!("`{name}` returns nothing, but this `return` has a value"),
            );
            self.label(format!("this is {}", a(&found)));
            self.related(Some(here.name), "no return type is declared");
            if !here.fails && !found.contains('{') {
                self.fix("declare the return type", here.after(), format!(" -> {found}"));
            }
            return self.unknown(value.span);
        }
        let why = format!("the function returns `{}`", self.ty_name(&ok));
        self.coerce_for(value, ret, here.ret, &why)
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
                        ExprKind::Return(Some(Box::new(self.returned(value, &ret))))
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
                // An error that was caught, of one of several kinds, is passed on: each
                // kind it may be must be one the function may fail with.
                let caught = match &value.kind {
                    ExprKind::Local(id) if value.ty == Ty::Failure => self.caught.get(id).cloned(),
                    _ => None,
                };
                if let Some(caught) = caught
                    && matches!(self.ret, Ty::Result(_, ErrSet::Of(_)) | Ty::Unknown)
                {
                    match (&self.ret, &caught) {
                        (Ty::Unknown, _) => self.note_errors(&caught),
                        (Ty::Result(_, set), _) if caught.within(set) => {}
                        (Ty::Result(_, set), ErrSet::Of(ids)) => {
                            let missing: Vec<String> = ids
                                .iter()
                                .filter(|id| !set.contains(**id))
                                .map(|id| format!("`{}`", self.ty_name(&Ty::Error(*id))))
                                .collect();
                            let declared = self.error_names(set);
                            self.error(
                                value.span,
                                format!(
                                    "this error may be {}, which is not in this function's error set",
                                    missing.join(" or ")
                                ),
                            );
                            self.label(format!("the function may fail only with `{declared}`"));
                            if self.here.is_none() {
                                self.help("add it after the `!` in the function's signature (Section 6.3)");
                            }
                        }
                        _ => {
                            let declared = match &self.ret {
                                Ty::Result(_, set) => self.error_names(set),
                                _ => String::new(),
                            };
                            self.error(value.span, "this error may be any error at all");
                            self.label(format!("the function may fail only with `{declared}`"));
                            self.help("match the errors the function can pass on by their types, or give the function the error set `Failure` (Section 6.3)");
                        }
                    }
                    return self.mk(ExprKind::Fail(Box::new(value)), Ty::Never, stmt.span);
                }
                match (&self.ret, &value.ty) {
                    (_, Ty::Unknown) => {}
                    (Ty::Result(_, set), Ty::Error(id)) => {
                        if !set.contains(*id) {
                            let name = self.program.errors[*id].name.clone();
                            let declared = self.error_names(set);
                            self.error(value.span, format!("`{name}` is not in this function's error set"));
                            self.label(format!("the function may fail only with `{declared}`"));
                            if self.here.is_none() {
                                self.help(format!(
                                    "add `{name}` after the `!` in the function's signature (Section 6.3)"
                                ));
                            }
                            self.fix_error_set(&name);
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
                    (_, found) => {
                        let error = match found {
                            Ty::Error(id) => self.program.errors[*id].name.clone(),
                            _ => String::new(),
                        };
                        self.error(stmt.span, "`fail` in a function that cannot fail");
                        self.related(self.here.map(|here| here.name), "no error is declared after a `!`");
                        if self.here.is_none() || error.is_empty() {
                            self.help("give the function an error set: `fn f() -> T ! E` (Section 6.1)");
                        }
                        self.fix_error_set(&error);
                    }
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
                let reported = self.diags.len();
                let checked = self.coerce(checked, &Ty::Bool);
                if self.diags.len() > reported {
                    self.help("a condition is a `bool`: compare the value, as in `n != 0` or `name != \"\"`");
                }
                Cond::Expr(checked)
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
                    let kind = local.kind;
                    let ty = self.ty_name(&local.ty);
                    let at = self.declared_at.get(*id).copied().filter(|at| *at != Span::default());
                    self.error(expr.span, format!("cannot {action} `{name}`, which is not `var`"));
                    match kind {
                        LocalKind::Owned => {
                            self.related(at, format!("`{name}` is declared here, without `var`"));
                            match at.filter(|at| self.plain_bindings.contains(&at.start)) {
                                Some(at) => {
                                    let before = Span {
                                        start: at.start,
                                        end: at.start,
                                    };
                                    self.fix("declare it with `var`", before, "var ");
                                }
                                None => self.help(format!(
                                    "a name bound by a pattern cannot change; copy it into a variable first: `var copy := {name}`"
                                )),
                            }
                        }
                        LocalKind::Param(_)
                            if self
                                .actor
                                .is_some_and(|actor| actor.fields.iter().any(|field| field.name.name == name)) =>
                        {
                            // A field of the actor whose method this is.
                            self.related(at, format!("the field `{name}` is declared here, without `var`"));
                            if let Some(at) = at {
                                let before = Span {
                                    start: at.start,
                                    end: at.start,
                                };
                                self.fix("declare the field with `var`", before, "var ");
                            }
                        }
                        LocalKind::Param(_) => {
                            self.related(at, format!("the parameter `{name}` is declared here"));
                            self.note("a parameter is lent to the function: it reads the caller's value and cannot change it (Section 5.3)");
                            self.help(format!(
                                "to change the caller's value, declare the parameter as `{name}: var {ty}` and pass `var {name}`"
                            ));
                            self.help(format!("to change a copy, make one: `var copy := {name}`"));
                        }
                        LocalKind::Element => {
                            self.related(at, format!("`{name}` is the loop's variable"));
                            self.help("a loop variable names an element as it was; change the list through an index: `for i in 0..xs.len { xs[i] = ... }`");
                        }
                        LocalKind::Capture { .. } => {
                            self.help("a closure holds a copy of the variable, so assigning to it would change nothing outside; return the new value instead (Section 5.7)");
                        }
                    }
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
                    if action == "pass as `var`" {
                        self.error(expr.span, "this value cannot be passed as `var`");
                        self.label("not a variable");
                        self.note("a `var` argument is a variable that the function may change, or a field or element of one (Section 5.3)");
                        self.help("put the value in a variable first: `var n := ...`, then pass `var n`");
                    } else {
                        self.error_help(
                            expr.span,
                            format!("cannot {action} this expression"),
                            "only a `var` binding, or a field or element of one, can be changed (Section 5.2)",
                        );
                    }
                    return false;
                }
            }
        }
    }

    // ---- conversions --------------------------------------------------------------------

    /// If an error since `reported` says that the expression at `span` has the wrong type,
    /// points it at `because`, the place that asks for the type.
    pub(crate) fn explain_mismatch(&mut self, reported: usize, span: Span, because: Option<Span>, why: String) {
        let mismatch = self.diags.len() == reported + 1
            && self.diags[reported].span == span
            && self.diags[reported].message.starts_with("expected `");
        if mismatch {
            self.related(because, why);
        }
    }

    /// As `coerce`. If the types do not agree, the error also points at `because`, the
    /// place that asks for the type, and says `why`.
    pub(crate) fn coerce_for(&mut self, expr: Expr, expected: &Ty, because: Option<Span>, why: &str) -> Expr {
        let reported = self.diags.len();
        let expr = self.coerce(expr, expected);
        if self.diags.len() > reported {
            self.related(because, why);
        }
        expr
    }

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
        let after = Span {
            start: span.end,
            end: span.end,
        };
        let before = Span {
            start: span.start,
            end: span.start,
        };
        // A correction, where there is exactly one thing that was meant.
        let fix: Option<(&str, Vec<(Span, String)>)> = match (expected, &expr.ty) {
            (Ty::Float, Ty::Int) if matches!(expr.kind, ExprKind::Int(_)) => {
                Some(("write the number as a `float`", vec![(after, ".0".to_string())]))
            }
            (Ty::Float, Ty::Int) => Some((
                "convert the `int` to a `float`",
                vec![(before, "float.from(".to_string()), (after, ")".to_string())],
            )),
            (_, Ty::Fn(function)) if function.ret == *expected && function.params.is_empty() => {
                Some(("call the function", vec![(after, "()".to_string())]))
            }
            (Ty::Str, Ty::Int | Ty::Float | Ty::Bool) => Some((
                "make it text",
                vec![(before, "\"{".to_string()), (after, "}\"".to_string())],
            )),
            _ => None,
        };
        let help = match (expected, &expr.ty) {
            (Ty::Float, Ty::Int) => Some("an `int` never becomes a `float` by itself (Section 4.2)"),
            (_, Ty::Fn(function)) if function.ret == *expected => {
                Some("this names the function; to use its result, call it")
            }
            (Ty::Int | Ty::Float, Ty::Str) if !matches!(expr.kind, ExprKind::Str(_)) => {
                Some("to read a number from text, write `int.parse(text)` or `float.parse(text)`, which give a result")
            }
            (Ty::Int, Ty::Float) => Some("choose the rounding: `x.round()`, `x.floor()`, `x.ceil()` or `x.trunc()`"),
            (_, Ty::Optional(inner)) if **inner == *expected => Some(
                "the value may be absent: give one to use when there is none, `x else fallback`, or test for it, `if value := x { ... }` (Section 4.8)",
            ),
            (_, Ty::Result(inner, _)) if **inner == *expected => Some(
                "the value may be an error: `x?` passes the error on to the caller, and `x else fallback` gives a value in its place (Section 6.4)",
            ),
            (Ty::Result(..), Ty::Result(..)) => Some("the errors of the value are not all in the expected error set"),
            (Ty::Struct(id), Ty::Str) if self.program.structs[*id].name == "LogMessage" && self.is_opaque(expected) => {
                Some(
                    "a line of the log is written as a string literal, so that the values in it are kept as fields: `log.info(\"{text}\")` logs a string that the program has (Section 3.3)",
                )
            }
            (Ty::Struct(id), Ty::Str) if self.program.structs[*id].name == "SqlQuery" && self.is_opaque(expected) => {
                Some(
                    "a query is written as a string literal where one is expected, so that the values in it are sent apart from its text; for text the program has put together itself there is `sql.Query.raw(text)` (Section 10.3)",
                )
            }
            _ => None,
        };
        let message = format!("expected `{want}`, found `{found}`");
        match help {
            Some(help) => self.error_help(span, message, help),
            None => self.error(span, message),
        }
        self.label(format!("this is {}", a(&found)));
        if let Some((message, edits)) = fix {
            self.fix_edits(message, edits);
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
        "List" | "Map" | "Set" | "Failure" | "Decimal" | "Json" | "Channel" | "Shared" | "Any" | "Self"
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
        | ast::ExprKind::Duration(_)
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

/// A type's name with its article: "an `int`", "a `string`".
pub(crate) fn a(name: &str) -> String {
    let letters: Vec<char> = name.chars().take(3).map(|c| c.to_ascii_lowercase()).collect();
    let is_vowel = |c: &char| matches!(c, 'a' | 'e' | 'i' | 'o' | 'u');
    let vowel = match letters.as_slice() {
        // "a user", "a unit": a `u` that is said as "you".
        ['u', second, third, ..] => is_vowel(second) || !is_vowel(third),
        [first, ..] => is_vowel(first),
        [] => false,
    };
    format!("{} `{name}`", if vowel { "an" } else { "a" })
}

/// The name a function is recorded under where the program's declarations are noted.
fn fn_key(sig: &ast::FnSig) -> String {
    let owner = match &sig.target {
        ast::FnTarget::Free => return format!("fn {}", sig.name.name),
        ast::FnTarget::Method { ty, .. } => ty.as_ref(),
        ast::FnTarget::TypeLevel(ty) => Some(ty),
    };
    let owner = match owner.map(|ty| &ty.kind) {
        Some(ast::TypeKind::Named { path, .. }) => path.last().map(|name| name.name.as_str()).unwrap_or(""),
        Some(ast::TypeKind::List(_)) => "List",
        _ => "",
    };
    format!("fn {owner}.{}", sig.name.name)
}
