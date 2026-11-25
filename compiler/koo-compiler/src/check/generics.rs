//! Generic types and functions (Section 4.10) and interfaces as constraints (Section 4.9).
//!
//! Generics are compiled by instantiation: a generic declaration is kept as written, and
//! each set of type arguments it is used with makes one ordinary type or function, checked
//! with the parameters bound to those arguments. This is step 6 of the pipeline in Section
//! 13.1 done early, in the checker, which keeps everything after it free of type
//! parameters. Its cost is that a generic body is checked when it is first used, not when
//! it is declared; the specification asks for the latter, and a later version will do both.

use std::collections::HashMap;

use koo_syntax::Span;
use koo_syntax::ast;

use super::{Checker, FnAt, FnSig, Owner, ParamSig, TypeName, param_kind};
use crate::hir::*;

pub(super) enum GenericDecl<'a> {
    Struct(&'a ast::TypeDecl),
    Enum(&'a ast::EnumDecl),
}

pub(super) struct GenericType<'a> {
    pub params: Vec<String>,
    pub decl: GenericDecl<'a>,
    /// Whether the prelude declares the type.
    pub prelude: bool,
}

/// What a generic method or type-level function belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum OwnerKey {
    /// A generic type declared in the program, by name.
    Generic(String),
    List,
    Optional,
}

pub(super) struct TemplateParam<'a> {
    pub name: String,
    pub constraints: &'a [ast::Type],
    pub resource: bool,
}

/// A generic function, as declared.
pub(super) struct Template<'a> {
    pub decl: &'a ast::FnDecl,
    /// The type parameters: those the receiver's type binds, those declared in `<...>`,
    /// and one for each parameter whose type is an interface.
    pub params: Vec<TemplateParam<'a>>,
    /// For a parameter whose declared type is an interface: the type parameter it stands for.
    pub implicit: HashMap<usize, String>,
    pub display: String,
    /// Whether the function is part of the prelude, whose errors are reported at the call.
    pub prelude: bool,
}

/// An instantiation whose body is still to be checked.
pub(super) struct Pending<'a> {
    pub id: FnId,
    pub decl: &'a ast::FnDecl,
    pub env: HashMap<String, Ty>,
    /// For a prelude function: the call that asked for it.
    pub origin: Option<(Span, String)>,
}

/// The errors a type parameter stands for when it is written after a `!`: those of the
/// function that was passed where the parameter is, kept as a type. No errors at all is
/// `Never`, one is the error's type, and several are a result that has only errors.
pub(super) fn errors_as_ty(set: Option<&ErrSet>) -> Ty {
    match set {
        None => Ty::Never,
        Some(ErrSet::Any) => Ty::Failure,
        Some(ErrSet::Of(ids)) if ids.len() == 1 => Ty::Error(ids[0]),
        Some(set) => Ty::Result(Box::new(Ty::Never), set.clone()),
    }
}

/// The reverse of `errors_as_ty`; nothing for a type that is not errors.
pub(super) fn errors_of(ty: &Ty) -> Option<ErrSet> {
    match ty {
        Ty::Never | Ty::Unknown => Some(ErrSet::Of(Vec::new())),
        Ty::Failure => Some(ErrSet::Any),
        Ty::Error(id) => Some(ErrSet::Of(vec![*id])),
        Ty::Result(ok, set) if **ok == Ty::Never => Some(set.clone()),
        _ => None,
    }
}

/// The names in a list of error types, as `free_names` finds them in a type.
fn free_error_names(errors: &[ast::Type], known: &dyn Fn(&str) -> bool, out: &mut Vec<String>) {
    for error in errors {
        free_names(error, known, out);
    }
}

/// The names in a type that are not known types: the type parameters a receiver binds.
fn free_names(ty: &ast::Type, known: &dyn Fn(&str) -> bool, out: &mut Vec<String>) {
    match &ty.kind {
        ast::TypeKind::Named { path, args } => {
            if path.len() == 1 && args.is_empty() && !known(&path[0].name) && !out.contains(&path[0].name) {
                out.push(path[0].name.clone());
            }
            for arg in args {
                free_names(arg, known, out);
            }
        }
        ast::TypeKind::List(inner) | ast::TypeKind::Optional(inner) => free_names(inner, known, out),
        ast::TypeKind::Map(key, value) => {
            free_names(key, known, out);
            free_names(value, known, out);
        }
        ast::TypeKind::Tuple(types) => types.iter().for_each(|t| free_names(t, known, out)),
        ast::TypeKind::Result { ok, errors } => {
            free_names(ok, known, out);
            free_error_names(errors, known, out);
        }
        ast::TypeKind::Fn {
            params, ret, errors, ..
        } => {
            params.iter().for_each(|(_, t)| free_names(t, known, out));
            if let Some(ret) = ret {
                free_names(ret, known, out);
            }
            free_error_names(errors, known, out);
        }
        ast::TypeKind::Array(inner, _) => free_names(inner, known, out),
        ast::TypeKind::Struct(_) => {}
    }
}

/// Gives the type parameters written in an error set the errors of an actual function.
fn bind_errors(errors: &[ast::Type], actual: Option<&ErrSet>, params: &[String], env: &mut HashMap<String, Ty>) {
    for error in errors {
        if let ast::TypeKind::Named { path, args } = &error.kind
            && path.len() == 1
            && args.is_empty()
            && params.iter().any(|p| *p == path[0].name)
        {
            env.entry(path[0].name.clone()).or_insert_with(|| errors_as_ty(actual));
        }
    }
}

/// Whether the errors written after a type mention a type parameter with no argument yet.
pub(super) fn errors_unbound(errors: &[ast::Type], params: &[String], env: &HashMap<String, Ty>) -> bool {
    let mut names = Vec::new();
    free_error_names(errors, &|name| !params.iter().any(|p| p == name), &mut names);
    names.iter().any(|name| !env.contains_key(name))
}

/// Whether a type as written mentions a type parameter that has no argument yet.
fn mentions_unbound(ty: &ast::Type, params: &[String], env: &HashMap<String, Ty>) -> bool {
    let mut names = Vec::new();
    free_names(ty, &|name| !params.iter().any(|p| p == name), &mut names);
    names.iter().any(|name| !env.contains_key(name))
}

impl<'a> Checker<'a> {
    /// Whether this is a function with a type parameter for errors that stands, this
    /// time, for none (`fn retry<T, E>(body: fn() -> T ! E)` given a function that cannot
    /// fail). What the function does with the errors of `body` is then written for
    /// something that cannot happen, and is left out.
    pub(super) fn without_errors(&self) -> bool {
        self.type_env.values().any(|ty| *ty == Ty::Never)
    }

    pub(super) fn is_type_name(&self, name: &str) -> bool {
        self.type_names.contains_key(name)
            || self.generic_types.contains_key(name)
            || self.interfaces.contains_key(name)
            || super::is_builtin_type(name)
            || koo_syntax::token::is_primitive(name)
            || matches!(name, "Ordered" | "Hashable" | "Equatable" | "Decimal")
    }

    pub(super) fn declare_generic_type(&mut self, name: &ast::Ident, params: &[ast::TypeParam], decl: GenericDecl<'a>) {
        // The prelude is where the built-in names `Map` and `Set` are declared.
        let taken = if self.in_prelude {
            self.generic_types.contains_key(&name.name)
        } else {
            self.is_type_name(&name.name)
        };
        if taken {
            self.already_declared(name.span, &name.name, &format!("type {}", name.name));
            return;
        }
        for param in params {
            if !matches!(param.constraint, ast::Constraint::None) {
                // Constraints on a type's parameters are checked where its functions use them.
            }
        }
        self.generic_types.insert(
            name.name.clone(),
            GenericType {
                params: params.iter().map(|p| p.name.name.clone()).collect(),
                decl,
                prelude: self.in_prelude,
            },
        );
    }

    /// The type `Name<args>`, made on first use.
    pub(super) fn instantiate_type(&mut self, name: &str, args: Vec<Ty>, span: Span) -> Ty {
        let key = (name.to_string(), args.clone());
        if let Some(ty) = self.type_instances.get(&key) {
            return ty.clone();
        }
        if args.iter().any(|arg| !self.ty_fits(arg)) {
            self.error_help(
                span,
                format!("`{name}` is needed for a type nested more than {MAX_TYPE_DEPTH} deep, or of more than {MAX_TYPE_SIZE} parts"),
                "a generic type that contains itself with a larger type argument has no end",
            );
            return Ty::Unknown;
        }
        if self.generic_types[name].prelude && args.contains(&Ty::Unit) {
            self.unsupported(span, "a map or set of `()` is");
            return Ty::Unknown;
        }
        let generic = &self.generic_types[name];
        if generic.params.len() != args.len() {
            let expected = generic.params.len();
            let params = generic.params.join(", ");
            self.error(
                span,
                format!(
                    "`{name}` takes {expected} type argument{}, found {}",
                    if expected == 1 { "" } else { "s" },
                    args.len()
                ),
            );
            self.help(format!(
                "write `{name}<{params}>` with a type for each: for example `{name}<{}>`",
                vec!["int"; expected].join(", ")
            ));
            return Ty::Unknown;
        }
        if args.iter().any(|arg| arg.is_unknown()) {
            return Ty::Unknown;
        }
        // The constraints on the type's parameters: `Map<K: Hashable, V>`.
        let declared: &'a [ast::TypeParam] = match generic.decl {
            GenericDecl::Struct(decl) => &decl.type_params,
            GenericDecl::Enum(decl) => &decl.type_params,
        };
        let mut satisfied = true;
        for (param, arg) in declared.iter().zip(&args) {
            if let ast::Constraint::Interfaces(constraints) = &param.constraint {
                for constraint in constraints {
                    if let Err(why) = self.satisfies(arg, constraint) {
                        let (found, wanted) = (self.ty_name(arg), type_text(constraint));
                        self.error_help(
                            span,
                            format!("`{found}` does not satisfy `{wanted}`"),
                            format!("`{name}` requires it: {why}"),
                        );
                        satisfied = false;
                    }
                }
            }
        }
        if !satisfied {
            return Ty::Unknown;
        }
        let param_names = self.generic_types[name].params.clone();
        let env: HashMap<String, Ty> = param_names.into_iter().zip(args.iter().cloned()).collect();
        let display = format!(
            "{name}<{}>",
            args.iter().map(|arg| self.ty_name(arg)).collect::<Vec<_>>().join(", ")
        );
        let generic = &self.generic_types[name];

        match generic.decl {
            GenericDecl::Struct(decl) => {
                let id = self.program.structs.len();
                self.program.structs.push(StructDef {
                    name: display,
                    fields: Vec::new(),
                });
                self.defaults.push(Vec::new());
                let ty = Ty::Struct(id);
                // Registered before the fields are resolved, so that a field may name
                // this same type.
                self.type_instances.insert(key.clone(), ty.clone());
                self.instance_of.insert(ty.clone(), key);
                let saved = std::mem::replace(&mut self.type_env, env);
                if let ast::TypeBody::Struct(fields) = &decl.body {
                    let (defs, defaults) = self.fields(fields, true);
                    self.program.structs[id].fields = defs;
                    self.defaults[id] = defaults;
                }
                self.type_env = saved;
                if self.contains_itself(Owner::Struct(id)) {
                    self.program.structs[id].fields.clear();
                    self.defaults[id].clear();
                    self.error_help(
                        span,
                        format!("`{name}` contains itself, which is not supported by this compiler yet"),
                        "hold the inner values in a list (`[T]`), which this compiler does support",
                    );
                }
                ty
            }
            GenericDecl::Enum(decl) => {
                let id = self.program.enums.len();
                self.program.enums.push(EnumDef {
                    name: display,
                    variants: Vec::new(),
                });
                let ty = Ty::Enum(id);
                self.type_instances.insert(key.clone(), ty.clone());
                self.instance_of.insert(ty.clone(), key);
                let saved = std::mem::replace(&mut self.type_env, env);
                let variants = self.variants(decl);
                self.program.enums[id].variants = variants;
                self.type_env = saved;
                if self.contains_itself(Owner::Enum(id)) {
                    self.program.enums[id]
                        .variants
                        .iter_mut()
                        .for_each(|v| v.fields.clear());
                    self.error_help(
                        span,
                        format!("`{name}` contains itself, which is not supported by this compiler yet"),
                        "hold the inner values in a list (`[T]`), which this compiler does support",
                    );
                }
                ty
            }
        }
    }

    /// Whether a struct or enum holds itself by value, which has no finite size.
    pub(super) fn contains_itself(&self, start: Owner) -> bool {
        fn reaches(checker: &Checker<'_>, ty: &Ty, start: Owner, seen: &mut Vec<Owner>) -> bool {
            let owner = match ty {
                Ty::Optional(inner) | Ty::Result(inner, _) => return reaches(checker, inner, start, seen),
                Ty::Tuple(types) => return types.iter().any(|t| reaches(checker, t, start, seen)),
                Ty::Struct(id) => Owner::Struct(*id),
                Ty::Enum(id) => Owner::Enum(*id),
                _ => return false,
            };
            if owner == start {
                return true;
            }
            if seen.contains(&owner) {
                return false;
            }
            seen.push(owner);
            fields_of(checker, owner)
                .iter()
                .any(|ty| reaches(checker, ty, start, seen))
        }
        fn fields_of(checker: &Checker<'_>, owner: Owner) -> Vec<Ty> {
            match owner {
                Owner::Struct(id) => checker.program.structs[id]
                    .fields
                    .iter()
                    .map(|f| f.ty.clone())
                    .collect(),
                Owner::Enum(id) => checker.program.enums[id]
                    .variants
                    .iter()
                    .flat_map(|v| v.fields.iter().map(|f| f.ty.clone()))
                    .collect(),
                _ => Vec::new(),
            }
        }
        let mut seen = Vec::new();
        fields_of(self, start)
            .iter()
            .any(|ty| reaches(self, ty, start, &mut seen))
    }

    /// The owner a receiver or target type names, if it is one a generic function can be
    /// declared on: a generic type of the program, or (in the prelude) a built-in one.
    fn generic_owner(&self, ty: &ast::Type) -> Option<OwnerKey> {
        match &ty.kind {
            ast::TypeKind::Named { path, .. } if path.len() == 1 => {
                let name = path[0].name.as_str();
                match name {
                    "List" if self.in_prelude => Some(OwnerKey::List),
                    _ if self.generic_types.contains_key(name) => Some(OwnerKey::Generic(name.to_string())),
                    _ => None,
                }
            }
            ast::TypeKind::List(_) if self.in_prelude => Some(OwnerKey::List),
            ast::TypeKind::Optional(_) if self.in_prelude => Some(OwnerKey::Optional),
            ast::TypeKind::Map(..) if self.generic_types.contains_key("Map") => {
                Some(OwnerKey::Generic("Map".to_string()))
            }
            _ => None,
        }
    }

    /// Records a function as a template if it is generic. Returns whether it was.
    pub(super) fn declare_template(&mut self, decl: &'a ast::FnDecl) -> bool {
        let sig = &decl.sig;
        let target = match &sig.target {
            ast::FnTarget::Method { ty: Some(ty), .. } => Some((ty, true)),
            ast::FnTarget::TypeLevel(ty) => Some((ty, false)),
            _ => None,
        };
        let owner = target.and_then(|(ty, _)| self.generic_owner(ty));
        let mut implicit = HashMap::new();
        for (index, param) in sig.params.iter().enumerate() {
            if let ast::TypeKind::Named { path, args } = &param.ty.kind
                && path.len() == 1
                && args.is_empty()
                && self.interfaces.contains_key(&path[0].name)
            {
                implicit.insert(index, format!("{} of `{}`", path[0].name, param.name.name));
            }
        }
        if sig.type_params.is_empty() && owner.is_none() && implicit.is_empty() {
            return false;
        }

        let mut params: Vec<TemplateParam<'a>> = Vec::new();
        if let (Some((ty, _)), Some(_)) = (target, &owner) {
            // The receiver names the type parameters of its type (Section 4.10).
            let mut names = Vec::new();
            free_names(ty, &|name| self.is_type_name(name), &mut names);
            for name in names {
                params.push(TemplateParam {
                    name,
                    constraints: &[],
                    resource: false,
                });
            }
        }
        for param in &sig.type_params {
            if params.iter().any(|p| p.name == param.name.name) || self.is_type_name(&param.name.name) {
                self.error(
                    param.name.span,
                    format!("the type parameter `{}` is already declared", param.name.name),
                );
            }
            let (constraints, resource): (&'a [ast::Type], bool) = match &param.constraint {
                ast::Constraint::None => (&[], false),
                ast::Constraint::Resource => (&[], true),
                ast::Constraint::Interfaces(types) => (types.as_slice(), false),
            };
            params.push(TemplateParam {
                name: param.name.name.clone(),
                constraints,
                resource,
            });
        }
        let mut by_position: Vec<(&usize, &String)> = implicit.iter().collect();
        by_position.sort();
        for (&index, name) in by_position {
            params.push(TemplateParam {
                name: name.clone(),
                constraints: std::slice::from_ref(&sig.params[index].ty),
                resource: false,
            });
        }

        let id = self.templates.len();
        // A function is known by the type it is of, whether or not the type is generic:
        // `http.Request.json` and `http.Reply.json` are two functions.
        let display = match target {
            Some((ty, _)) => {
                let on = type_text(ty);
                let on = if self.in_prelude {
                    self.shown_name(&on, true)
                } else {
                    on
                };
                format!("{on}.{}", sig.name.name)
            }
            None => self.shown_fn(&sig.name.name),
        };
        let name = sig.name.name.clone();
        let duplicate = match (&owner, target) {
            (Some(owner), Some((_, is_method))) => {
                let key = (owner.clone(), name.clone());
                let taken = self.generic_methods.contains_key(&key) || self.generic_type_fns.contains_key(&key);
                if !taken {
                    if is_method {
                        self.generic_methods.insert(key, id);
                    } else {
                        self.generic_type_fns.insert(key, id);
                    }
                }
                taken
            }
            (None, Some((ty, is_method))) => {
                // A generic function on a type that is not generic: `fn (self: Request) json<T>()`.
                let resolved = self.resolve_type(ty);
                match self.owner_of(&resolved) {
                    Some(plain) => {
                        let key = (plain, name.clone());
                        let taken = self.methods.contains_key(&key)
                            || self.type_fns.contains_key(&key)
                            || self.plain_generic_methods.contains_key(&key);
                        if !taken {
                            self.plain_generic_methods.insert(key, (id, is_method));
                        }
                        taken
                    }
                    None => {
                        if !resolved.is_unknown() {
                            self.error_help(
                                sig.name.span,
                                format!("a function cannot be declared on `{}`", self.ty_name(&resolved)),
                                "methods and type-level functions are declared on the types of this package (Sections 4.6 and 6.1)",
                            );
                        }
                        return true;
                    }
                }
            }
            (_, None) => {
                let taken = self.generic_fns.contains_key(&name)
                    || self.free_fns.contains_key(&name)
                    || self.consts.contains_key(&name)
                    || matches!(name.as_str(), "print" | "panic" | "all" | "race");
                if !taken {
                    self.generic_fns.insert(name.clone(), id);
                }
                taken
            }
        };
        if duplicate {
            self.already_declared(sig.name.span, &display, &super::fn_key(sig));
            self.note("there is no overloading: one name, one function (Section 6.1)");
        }
        self.templates.push(Template {
            decl,
            params,
            implicit,
            display,
            prelude: self.in_prelude,
        });
        true
    }

    /// Matches a type as written against an actual type, binding the type parameters it
    /// mentions. A mismatch binds nothing; the conversion of the argument reports it.
    pub(super) fn bind(&mut self, pattern: &ast::Type, actual: &Ty, params: &[String], env: &mut HashMap<String, Ty>) {
        if actual.is_unknown() || *actual == Ty::Never {
            return;
        }
        match &pattern.kind {
            ast::TypeKind::Named { path, args } if path.len() == 1 => {
                let name = path[0].name.as_str();
                if args.is_empty() && params.iter().any(|p| p == name) {
                    env.entry(name.to_string()).or_insert_with(|| actual.clone());
                    return;
                }
                if name == "List" && args.len() == 1 {
                    if let Ty::List(element) = actual {
                        self.bind(&args[0], element, params, env);
                    }
                    return;
                }
                if let Some((generic, actual_args)) = self.instance_of.get(actual).cloned()
                    && generic == name
                    && actual_args.len() == args.len()
                {
                    for (arg, actual_arg) in args.iter().zip(&actual_args) {
                        self.bind(arg, actual_arg, params, env);
                    }
                }
            }
            ast::TypeKind::List(element) => {
                if let Ty::List(actual) = actual {
                    self.bind(element, actual, params, env);
                }
            }
            ast::TypeKind::Map(key, value) => {
                if let Some((generic, actual_args)) = self.instance_of.get(actual).cloned()
                    && generic == "Map"
                    && actual_args.len() == 2
                {
                    self.bind(key, &actual_args[0], params, env);
                    self.bind(value, &actual_args[1], params, env);
                }
            }
            // A `T` is accepted where a `T?` is expected; inference prefers the exact
            // match (Section 4).
            ast::TypeKind::Optional(inner) => match actual {
                Ty::Optional(actual) => self.bind(inner, actual, params, env),
                other => self.bind(inner, other, params, env),
            },
            ast::TypeKind::Result { ok, errors } => match actual {
                Ty::Result(actual, set) => {
                    self.bind(ok, actual, params, env);
                    bind_errors(errors, Some(set), params, env);
                }
                other => self.bind(ok, other, params, env),
            },
            ast::TypeKind::Tuple(types) => {
                if let Ty::Tuple(actual) = actual
                    && actual.len() == types.len()
                {
                    for (ty, actual) in types.iter().zip(actual) {
                        self.bind(ty, actual, params, env);
                    }
                }
            }
            ast::TypeKind::Fn {
                params: fn_params,
                ret,
                errors,
                ..
            } => {
                if let Ty::Fn(actual) = actual
                    && actual.params.len() == fn_params.len()
                {
                    for ((_, ty), (_, actual)) in fn_params.iter().zip(&actual.params) {
                        self.bind(ty, actual, params, env);
                    }
                    if let Some(ret) = ret {
                        // `fn(T) -> U` takes the whole of what the function returns, a
                        // result included; `fn(T) -> U ! E` takes its success type.
                        let actual_ret = match &actual.ret {
                            Ty::Result(ok, _) if !errors.is_empty() => (**ok).clone(),
                            other => other.clone(),
                        };
                        self.bind(ret, &actual_ret, params, env);
                    }
                    // `fn(T) -> U ! E`: `E` is what the function fails with, which may
                    // be nothing.
                    match &actual.ret {
                        Ty::Result(_, set) => bind_errors(errors, Some(set), params, env),
                        Ty::Unknown => {}
                        _ => bind_errors(errors, None, params, env),
                    }
                }
            }
            _ => {}
        }
    }

    /// Resolves a type of a template with the arguments found so far, if they are enough.
    pub(super) fn resolve_bound(&mut self, ty: &ast::Type, params: &[String], env: &HashMap<String, Ty>) -> Option<Ty> {
        if mentions_unbound(ty, params, env) {
            return None;
        }
        let saved = std::mem::replace(&mut self.type_env, env.clone());
        let resolved = self.resolve_type(ty);
        self.type_env = saved;
        Some(resolved)
    }

    /// The function a template makes for the given type arguments, made on first use.
    pub(super) fn instantiate(&mut self, template: usize, args: Vec<Ty>, span: Span) -> Option<FnId> {
        if let Some(&id) = self.fn_instances.get(&(template, args.clone())) {
            return Some(id);
        }
        if args.iter().any(|arg| !self.ty_fits(arg)) {
            let name = self.templates[template].display.clone();
            self.error_help(
                span,
                format!("`{name}` is needed for a type nested more than {MAX_TYPE_DEPTH} deep, or of more than {MAX_TYPE_SIZE} parts"),
                "a generic function that calls itself with a larger type each time never stops being made; call it with the same type",
            );
            return None;
        }
        let t = &self.templates[template];
        let decl = t.decl;
        let prelude = t.prelude;
        let names: Vec<String> = t.params.iter().map(|p| p.name.clone()).collect();
        let constraints: Vec<(&'a [ast::Type], bool)> = t.params.iter().map(|p| (p.constraints, p.resource)).collect();
        let implicit_names = t.implicit.clone();
        let base = t.display.clone();

        let mut ok = true;
        for ((constraints, _), arg) in constraints.iter().zip(&args) {
            for constraint in constraints.iter() {
                if let Err(why) = self.satisfies(arg, constraint) {
                    let (found, wanted) = (self.ty_name(arg), type_text(constraint));
                    self.error_help(
                        span,
                        format!("`{found}` does not satisfy `{wanted}`"),
                        format!("`{base}` requires it: {why}"),
                    );
                    ok = false;
                }
            }
        }
        if !ok {
            return None;
        }

        let env: HashMap<String, Ty> = names.iter().cloned().zip(args.iter().cloned()).collect();
        let implicit: HashMap<usize, Ty> = implicit_names
            .iter()
            .map(|(index, name)| (*index, env[name].clone()))
            .collect();
        let shown: Vec<String> = names
            .iter()
            .zip(&args)
            .filter(|(name, _)| !name.contains(' '))
            .map(|(_, arg)| self.ty_name(arg))
            .chain({
                // In the order of the parameters, so that the name is the same every time.
                let mut by_index: Vec<(&usize, &Ty)> = implicit.iter().collect();
                by_index.sort_by_key(|(index, _)| **index);
                by_index.into_iter().map(|(_, ty)| self.ty_name(ty))
            })
            .collect();
        let display = format!("{base}<{}>", shown.join(", "));

        let saved = std::mem::replace(&mut self.type_env, env.clone());
        let (mut params, ret) = self.params_and_ret(decl, &implicit);
        if prelude {
            params.iter_mut().for_each(|param| param.at = None);
        }
        let receiver = match &decl.sig.target {
            ast::FnTarget::Method { kind, ty: Some(ty) } => Some(ParamSig {
                name: "self".to_string(),
                kind: param_kind(*kind),
                ty: self.resolve_type(ty),
                default: None,
                lent: false,
                at: None,
            }),
            _ => None,
        };
        self.type_env = saved;

        let id = self.sigs.len();
        // What the compiler writes for a program's types has no generic function, so
        // one of the prelude's is the library's own.
        let located = prelude && super::takes_location(&params);
        if located {
            self.located.insert(id);
        }
        self.sigs.push(FnSig {
            name: display.clone(),
            receiver,
            params,
            ret: ret.clone(),
            at: (!prelude).then(|| FnAt::of(&decl.sig)),
        });
        self.program.functions.push(Function {
            name: display.clone(),
            params: 0,
            locals: Vec::new(),
            ret,
            body: Block::default(),
            span: decl.sig.span,
            prelude,
            located,
            ..Function::default()
        });
        self.fn_instances.insert((template, args), id);
        self.pending.push(Pending {
            id,
            decl,
            env,
            // A prelude function that another one needs is reported where the program
            // called the first.
            origin: prelude.then_some(match &self.origin {
                Some((outer, _)) => (*outer, display),
                None => (span, display),
            }),
        });
        Some(id)
    }

    /// Whether a type satisfies an interface (Section 4.9); if not, why.
    pub(super) fn satisfies(&mut self, ty: &Ty, constraint: &ast::Type) -> Result<(), String> {
        let ast::TypeKind::Named { path, args } = &constraint.kind else {
            return Err("the constraint is not an interface".to_string());
        };
        if path.len() != 1 || !args.is_empty() {
            return Err("generic interfaces are not supported by this compiler yet".to_string());
        }
        if ty.is_unknown() {
            return Ok(());
        }
        let name = path[0].name.as_str();
        match name {
            "Any" | "Equatable" => {
                if self.has_equality(ty) {
                    Ok(())
                } else {
                    Err("a function value has no `==`".to_string())
                }
            }
            "Ordered" => {
                if self.is_ordered(ty) {
                    Ok(())
                } else {
                    Err("it has no order; give the type a method `fn (self: T) compare(other: T) -> int`".to_string())
                }
            }
            "Hashable" => {
                if self.is_hashable(ty) {
                    Ok(())
                } else {
                    Err(
                        "it cannot be hashed; a `float`, or a type that holds one, has no hash that agrees with `==`"
                            .to_string(),
                    )
                }
            }
            _ => {
                let Some(interface) = self.interfaces.get(name).copied() else {
                    return Err(format!("`{name}` is not an interface"));
                };
                let owner = self.owner_of(ty);
                let mut env = HashMap::new();
                env.insert("Self".to_string(), ty.clone());
                for member in &interface.members {
                    let method = member.sig.name.name.clone();
                    let type_level = matches!(member.sig.target, ast::FnTarget::TypeLevel(_));
                    let found = if type_level {
                        owner.and_then(|owner| self.type_fns.get(&(owner, method.clone())).copied())
                    } else {
                        self.method_of(ty, &method)
                    };
                    let Some(found) = found else {
                        return Err(format!(
                            "it has no {} `{method}`",
                            if type_level { "function" } else { "method" }
                        ));
                    };
                    // The same parameter types, and a return type that is the interface's.
                    let saved = std::mem::replace(&mut self.type_env, env.clone());
                    let wanted_params: Vec<(ParamKind, Ty)> = member
                        .sig
                        .params
                        .iter()
                        .map(|p| (param_kind(p.kind), self.resolve_type(&p.ty)))
                        .collect();
                    let wanted_ok = member
                        .sig
                        .ret
                        .as_ref()
                        .map(|t| self.resolve_type(t))
                        .unwrap_or(Ty::Unit);
                    let wanted_ret = match &member.sig.errors {
                        ast::ErrorSpec::Set(types) => self.result_of(wanted_ok, types),
                        _ => wanted_ok,
                    };
                    self.type_env = saved;
                    let sig = self.sigs[found].clone();
                    // The place a function of the library is told is no parameter of
                    // the method as a program sees it.
                    let seen = sig.params.len() - usize::from(self.located.contains(&found));
                    let same_params = seen == wanted_params.len()
                        && sig.params[..seen].iter().zip(&wanted_params).all(|(have, (kind, ty))| {
                            have.ty == *ty && (have.kind == ParamKind::Var) == (*kind == ParamKind::Var)
                        });
                    if !same_params || !self.can_coerce(&sig.ret, &wanted_ret) {
                        return Err(format!("its `{method}` has a different signature from the interface's"));
                    }
                    if let ast::FnTarget::Method { kind, .. } = &member.sig.target {
                        let wanted_var = param_kind(*kind) == ParamKind::Var;
                        let have_var = sig.receiver.as_ref().is_some_and(|r| r.kind == ParamKind::Var);
                        if wanted_var != have_var {
                            return Err(format!("its `{method}` and the interface's disagree on `var`"));
                        }
                    }
                }
                Ok(())
            }
        }
    }

    /// Whether `==` is defined for the type: every type has it but function values.
    pub(super) fn has_equality(&self, ty: &Ty) -> bool {
        self.has_equality_in(ty, &mut Vec::new())
    }

    /// `within` holds the structs and enums being looked into: a type that holds itself
    /// has `==` if the rest of it does.
    fn has_equality_in(&self, ty: &Ty, within: &mut Vec<Ty>) -> bool {
        // A map or a set is equal to another by a function of the prelude's: comparing
        // its fields would compare how it happens to be laid out, not what it holds.
        if self.is_prelude_type(ty) {
            return match self.prelude_equality(ty) {
                Some(args) => {
                    if within.contains(ty) {
                        return true;
                    }
                    within.push(ty.clone());
                    let all = args.iter().all(|arg| self.has_equality_in(arg, within));
                    within.pop();
                    all
                }
                None => false,
            };
        }
        match ty {
            Ty::Fn(_) => false,
            Ty::List(inner) | Ty::Optional(inner) => self.has_equality_in(inner, within),
            Ty::Result(ok, _) => self.has_equality_in(ok, within),
            Ty::Tuple(types) => types.iter().all(|t| self.has_equality_in(t, within)),
            Ty::Struct(_) | Ty::Enum(_) => {
                if within.contains(ty) {
                    return true;
                }
                within.push(ty.clone());
                let fields: Vec<&Ty> = match ty {
                    Ty::Struct(id) => self.program.structs[*id].fields.iter().map(|f| &f.ty).collect(),
                    Ty::Enum(id) => self.program.enums[*id]
                        .variants
                        .iter()
                        .flat_map(|v| v.fields.iter().map(|f| &f.ty))
                        .collect(),
                    _ => Vec::new(),
                };
                let all = fields.into_iter().all(|field| self.has_equality_in(field, within));
                within.pop();
                all
            }
            _ => true,
        }
    }

    /// Whether the type has `<`: a built-in ordered type, or one with a `compare` method.
    pub(super) fn is_ordered(&mut self, ty: &Ty) -> bool {
        match ty {
            Ty::Int | Ty::Float | Ty::Str | Ty::Byte | Ty::Bool => true,
            Ty::Enum(id) if self.program.enums[*id].variants.iter().all(|v| v.fields.is_empty()) => true,
            _ => self.compare_method(ty).is_some(),
        }
    }

    /// The hand-written `compare(other: Self) -> int` of a type, if it has one.
    pub(super) fn compare_method(&mut self, ty: &Ty) -> Option<FnId> {
        let id = self.method_of(ty, "compare")?;
        let sig = &self.sigs[id];
        // `<` is not written as a call, so nothing at it says `var`: the method may only
        // read the two values.
        let reads = sig.receiver.as_ref().is_some_and(|r| r.kind == ParamKind::Borrow)
            && sig.params.iter().all(|param| param.kind == ParamKind::Borrow);
        (reads && sig.params.len() == 1 && sig.params[0].ty == *ty && sig.ret == Ty::Int).then_some(id)
    }

    /// The method `name` of a type, for an interface or an operator to use. A method
    /// declared on a generic type (`fn (self: Cell<T>) name()`) is made for this instance
    /// of the type the first time it is asked for.
    pub(super) fn method_of(&mut self, ty: &Ty, name: &str) -> Option<FnId> {
        let owner = self.owner_of(ty)?;
        let key = (owner, name.to_string());
        if let Some(&id) = self.methods.get(&key) {
            return Some(id);
        }
        let (_, args) = self.instance_of.get(ty)?.clone();
        let template = self.generic_method(ty, name)?;
        // A method with type parameters of its own is not one function.
        if self.templates[template].params.len() != args.len() {
            return None;
        }
        let diags = self.diags.len();
        let id = self.instantiate(template, args, Span::default());
        self.diags.truncate(diags);
        let id = id?;
        self.methods.insert(key, id);
        Some(id)
    }

    pub(super) fn is_hashable(&self, ty: &Ty) -> bool {
        self.is_hashable_in(ty, &mut Vec::new())
    }

    /// As `has_equality_in`: a type that holds itself can be hashed if the rest of it can.
    fn is_hashable_in(&self, ty: &Ty, within: &mut Vec<Ty>) -> bool {
        if self.is_prelude_type(ty) {
            return false;
        }
        match ty {
            Ty::Int | Ty::Str | Ty::Byte | Ty::Bool | Ty::Unit => true,
            Ty::List(inner) | Ty::Optional(inner) => self.is_hashable_in(inner, within),
            Ty::Tuple(types) => types.iter().all(|t| self.is_hashable_in(t, within)),
            Ty::Struct(_) | Ty::Enum(_) => {
                if within.contains(ty) {
                    return true;
                }
                within.push(ty.clone());
                let fields: Vec<&Ty> = match ty {
                    Ty::Struct(id) => self.program.structs[*id].fields.iter().map(|f| &f.ty).collect(),
                    Ty::Enum(id) => self.program.enums[*id]
                        .variants
                        .iter()
                        .flat_map(|v| v.fields.iter().map(|f| &f.ty))
                        .collect(),
                    _ => Vec::new(),
                };
                let all = fields.into_iter().all(|field| self.is_hashable_in(field, within));
                within.pop();
                all
            }
            _ => false,
        }
    }

    /// The template for a method of the type, with the type arguments its receiver gives.
    pub(super) fn generic_method(&self, ty: &Ty, name: &str) -> Option<usize> {
        // A method that begins with `_` belongs to the prelude alone.
        if name.starts_with('_') && !self.body_in_prelude && self.is_prelude_type(ty) {
            return None;
        }
        let key = match ty {
            Ty::List(_) => OwnerKey::List,
            Ty::Optional(_) => OwnerKey::Optional,
            other => match self.instance_of.get(other) {
                Some((generic, _)) => OwnerKey::Generic(generic.clone()),
                None => {
                    let owner = self.owner_of(other)?;
                    return match self.plain_generic_methods.get(&(owner, name.to_string())) {
                        Some((id, true)) => Some(*id),
                        _ => None,
                    };
                }
            },
        };
        self.generic_methods.get(&(key, name.to_string())).copied()
    }

    /// The template for a type-level function `Name.function` of a generic type.
    pub(super) fn generic_type_fn(&self, type_name: &str, name: &str) -> Option<usize> {
        // A function that begins with `_` belongs to the prelude alone.
        if name.starts_with('_')
            && !self.body_in_prelude
            && self.generic_types.get(type_name).is_some_and(|generic| generic.prelude)
        {
            return None;
        }
        let key = match type_name {
            "List" => OwnerKey::List,
            other if self.generic_types.contains_key(other) => OwnerKey::Generic(other.to_string()),
            other => {
                let owner = match self.type_names.get(other)? {
                    TypeName::Struct(id) => Owner::Struct(*id),
                    TypeName::Enum(id) => Owner::Enum(*id),
                    TypeName::Error(id) => Owner::Error(*id),
                    _ => return None,
                };
                return match self.plain_generic_methods.get(&(owner, name.to_string())) {
                    Some((id, false)) => Some(*id),
                    _ => None,
                };
            }
        };
        self.generic_type_fns.get(&(key, name.to_string())).copied()
    }
}

/// A type as written, for messages.
pub(super) fn type_text(ty: &ast::Type) -> String {
    koo_syntax::dump::ty(ty)
}

/// How deep the types a generic function or type is made for may nest. Deeper than this
/// is a generic that asks for itself with a larger type each time, which has no end.
pub(super) const MAX_TYPE_DEPTH: usize = 24;

/// How many parts such a type may have in all. A generic that asks for itself with a tuple
/// of its own type doubles the type at each step, and would fill memory long before it
/// was 24 deep.
const MAX_TYPE_SIZE: usize = 512;

impl<'a> Checker<'a> {
    /// Whether a generic function or type may be made for the type: it is neither too
    /// deep nor too large.
    pub(super) fn ty_fits(&self, ty: &Ty) -> bool {
        let mut room = MAX_TYPE_SIZE;
        self.ty_depth(ty) <= MAX_TYPE_DEPTH && self.ty_room(ty, &mut room)
    }

    /// Takes the parts of `ty` from `room`, and says whether there was enough.
    fn ty_room(&self, ty: &Ty, room: &mut usize) -> bool {
        if *room == 0 {
            return false;
        }
        *room -= 1;
        match ty {
            Ty::List(inner) | Ty::Optional(inner) | Ty::Result(inner, _) => self.ty_room(inner, room),
            Ty::Tuple(parts) => parts.iter().all(|part| self.ty_room(part, room)),
            Ty::Fn(function) => {
                function.params.iter().all(|(_, ty)| self.ty_room(ty, room)) && self.ty_room(&function.ret, room)
            }
            Ty::Struct(_) | Ty::Enum(_) => match self.instance_of.get(ty) {
                Some((_, args)) => args.iter().all(|arg| self.ty_room(arg, room)),
                None => true,
            },
            _ => true,
        }
    }

    /// How many types are nested inside one another in `ty`.
    pub(super) fn ty_depth(&self, ty: &Ty) -> usize {
        let deepest = |types: &mut dyn Iterator<Item = &Ty>| types.map(|ty| self.ty_depth(ty)).max().unwrap_or(0);
        1 + match ty {
            Ty::List(inner) | Ty::Optional(inner) | Ty::Result(inner, _) => self.ty_depth(inner),
            Ty::Tuple(parts) => deepest(&mut parts.iter()),
            Ty::Fn(function) => deepest(&mut function.params.iter().map(|(_, ty)| ty).chain([&function.ret])),
            Ty::Struct(_) | Ty::Enum(_) => match self.instance_of.get(ty) {
                Some((_, args)) => deepest(&mut args.iter()),
                None => 0,
            },
            _ => 0,
        }
    }
}
