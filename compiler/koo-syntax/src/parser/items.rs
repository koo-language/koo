//! Files, imports and declarations.

use super::*;
use crate::token::StrPiece;

/// Where a function declaration appears, which decides the forms it may take.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FnCtx {
    Top,
    Actor,
    Interface,
}

impl Parser {
    pub(super) fn file(&mut self) -> File {
        let mut imports = Vec::new();
        let mut items = Vec::new();
        loop {
            self.skip_newlines();
            if self.at(&Tok::Eof) {
                break;
            }
            let start = self.pos;
            let depth = self.nl_sig.len();
            let result = if self.at_kw(Kw::Import) {
                if !items.is_empty() && !self.fragment {
                    let span = self.span();
                    self.report(span, "imports come before every declaration in a file");
                }
                self.import().map(|import| imports.push(import))
            } else {
                self.item().map(|item| items.push(item))
            };
            if result.is_ok() && !matches!(self.peek(), Tok::Newline | Tok::Eof) {
                let _ = self.expected::<()>("the end of the line");
                self.recover(depth, true);
            } else if result.is_err() {
                self.recover(depth, true);
            }
            if self.pos == start || (!self.at_newline() && !self.at(&Tok::Eof)) {
                // Recovery stopped at a stray closing bracket: step over it.
                self.bump();
            }
        }
        File { imports, items }
    }

    // ---- imports ------------------------------------------------------------------------

    fn import(&mut self) -> PResult<Import> {
        let start = self.bump().span;
        let (path, path_span) = match self.peek() {
            Tok::ImportPath(_) => {
                let token = self.bump();
                let Tok::ImportPath(path) = token.kind else {
                    unreachable!()
                };
                (path, token.span)
            }
            _ => return self.expected("a package path"),
        };

        if path == "c" && matches!(self.peek(), Tok::Str(_) | Tok::RawStr(_)) {
            let header = self.plain_string("a header name")?;
            let alias = if self.eat_kw(Kw::As) {
                Some(self.binding_name("import name")?)
            } else {
                None
            };
            let link = if *self.peek() == Tok::Ident("link".to_string()) {
                self.bump();
                Some(self.plain_string("a library name")?)
            } else {
                None
            };
            let binding = alias
                .as_ref()
                .map(|a| a.name.clone())
                .unwrap_or_else(|| "c".to_string());
            if !self.c_prefixes.contains(&binding) {
                self.c_prefixes.push(binding.clone());
            }
            let span = start.to(self.prev_span());
            return Ok(Import {
                kind: ImportKind::C { header, link },
                alias,
                binding,
                span,
            });
        }

        let registry = self.check_import_path(&path, path_span);
        let mut major = None;
        if let Tok::Ident(word) = self.peek()
            && let Some(n) = word.strip_prefix('v').and_then(|digits| digits.parse::<u32>().ok())
        {
            let span = self.bump().span;
            major = Some(n);
            if !registry {
                self.report(span, "only a registry package is imported with a major version");
            }
        }
        if registry && major.is_none() {
            let span = self.here();
            self.report_help(
                span,
                "a registry package is imported with its major version",
                format!("write `import {path} v1`"),
            );
        }
        let alias = if self.eat_kw(Kw::As) {
            Some(self.binding_name("import name")?)
        } else {
            None
        };
        let last = path.rsplit('/').next().unwrap_or("");
        let last_is_name = !last.is_empty()
            && last != "_"
            && !last.as_bytes()[0].is_ascii_digit()
            && last
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            && !is_primitive(last)
            && Kw::from_word(last).is_none();
        if alias.is_none() && !last_is_name {
            self.report_help(
                path_span,
                format!("`{last}` cannot be used as a name"),
                "give the import a name with `as`",
            );
        }
        let binding = alias
            .as_ref()
            .map(|a| a.name.clone())
            .unwrap_or_else(|| last.to_string());
        let span = start.to(self.prev_span());
        Ok(Import {
            kind: ImportKind::Package { path, major },
            alias,
            binding,
            span,
        })
    }

    /// Checks the shape of a package path. Returns whether it names a registry package.
    fn check_import_path(&mut self, path: &str, span: Span) -> bool {
        let segment_ok =
            |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        let relative = path.starts_with("./") || path.starts_with("../");
        let mut rest = path;
        if let Some(r) = rest.strip_prefix("./") {
            rest = r;
        } else {
            while let Some(r) = rest.strip_prefix("../") {
                rest = r;
            }
        }
        let segments: Vec<&str> = rest.split('/').collect();
        let registry = !relative && segments[0].contains('.');
        let ok = if relative {
            segments.iter().all(|s| segment_ok(s))
        } else if registry {
            let host_ok = segments[0].split('.').all(segment_ok);
            host_ok && segments.len() >= 3 && segments[1..].iter().all(|s| segment_ok(s))
        } else {
            segments[0] == "std" && segments.len() >= 2 && segments.iter().all(|s| segment_ok(s))
        };
        if !ok {
            self.report_help(
                span,
                format!("`{path}` is not a package path"),
                "a package is `std/name`, a relative path such as `./name`, or a registry path such as `pkg.koo.io/owner/name v1`",
            );
        }
        registry
    }

    /// A string literal with no interpolation, as an import or an attribute needs.
    pub(super) fn plain_string(&mut self, what: &str) -> PResult<String> {
        match self.peek() {
            Tok::Str(_) | Tok::RawStr(_) => {
                let token = self.bump();
                match token.kind {
                    Tok::RawStr(s) => Ok(s),
                    Tok::Str(pieces) => {
                        let mut text = String::new();
                        for piece in pieces {
                            match piece {
                                StrPiece::Lit(s) => text.push_str(&s),
                                StrPiece::Interp { span, .. } => {
                                    return self.fail(span, "an interpolation cannot be used here");
                                }
                            }
                        }
                        Ok(text)
                    }
                    _ => unreachable!(),
                }
            }
            _ => self.expected(what),
        }
    }

    // ---- attributes ---------------------------------------------------------------------

    pub(super) fn attributes(&mut self, skip_newlines: bool) -> PResult<Vec<Attribute>> {
        let mut attrs = Vec::new();
        while self.at(&Tok::At) {
            let start = self.bump().span;
            let name = self.lower_name("attribute name")?;
            let args = if self.at(&Tok::LParen) {
                self.attr_args()?
            } else {
                Vec::new()
            };
            attrs.push(Attribute {
                name,
                args,
                span: start.to(self.prev_span()),
            });
            if skip_newlines {
                self.skip_newlines();
            }
        }
        Ok(attrs)
    }

    fn attr_args(&mut self) -> PResult<Vec<AttrArg>> {
        self.nested(|p| p.attr_args_unguarded())
    }

    fn attr_args_unguarded(&mut self) -> PResult<Vec<AttrArg>> {
        self.delimited(&Tok::LParen, &Tok::RParen, |p| {
            let name = if p.at_word() && *p.peek2() == Tok::Colon {
                let name = p.member_name("an argument name")?;
                p.bump();
                Some(name)
            } else {
                None
            };
            let value = if p.at_word() && !matches!(p.peek(), Tok::True | Tok::False) {
                let mut path = vec![p.member_name("a name")?];
                while p.eat(&Tok::Dot) {
                    path.push(p.member_name("a name")?);
                }
                let args = if p.at(&Tok::LParen) { Some(p.attr_args()?) } else { None };
                AttrValue::Path { path, args }
            } else {
                match p.peek() {
                    Tok::Int(_)
                    | Tok::Float(_)
                    | Tok::Decimal(_)
                    | Tok::Duration(_)
                    | Tok::Char(_)
                    | Tok::Str(_)
                    | Tok::RawStr(_)
                    | Tok::True
                    | Tok::False => {
                        let literal = p.literal()?;
                        if let ExprKind::Str(parts) = &literal.kind
                            && parts.iter().any(|part| matches!(part, StrPart::Interp { .. }))
                        {
                            p.report(literal.span, "a string in an attribute cannot contain an interpolation");
                        }
                        AttrValue::Literal(literal)
                    }
                    _ => return p.expected("an attribute argument: a name or a literal"),
                }
            };
            Ok(AttrArg { name, value })
        })
    }

    // ---- declarations -------------------------------------------------------------------

    fn item(&mut self) -> PResult<Item> {
        let start = self.span();
        let docs = self.docs_before(start);
        let attrs = self.attributes(true)?;
        let public = self.eat_kw(Kw::Pub);
        let kind = match self.peek() {
            Tok::Kw(Kw::Type) => ItemKind::Type(self.type_decl(false)?),
            Tok::Kw(Kw::Resource) => {
                if *self.peek2() == Tok::Kw(Kw::Interface) {
                    self.bump();
                    ItemKind::Interface(self.interface_decl(true)?)
                } else {
                    ItemKind::Type(self.type_decl(true)?)
                }
            }
            Tok::Kw(Kw::Enum) => ItemKind::Enum(self.enum_decl()?),
            Tok::Kw(Kw::Error) => ItemKind::Error(self.error_decl()?),
            Tok::Kw(Kw::Interface) => ItemKind::Interface(self.interface_decl(false)?),
            Tok::Kw(Kw::Actor) => ItemKind::Actor(self.actor_decl()?),
            Tok::Kw(Kw::Fn) => ItemKind::Fn(self.fn_decl(FnCtx::Top)?),
            _ if self.fragment && attrs.is_empty() && !public => {
                let stmt = self.stmt()?;
                self.check_stmt(&stmt);
                ItemKind::Stmt(stmt)
            }
            Tok::Ident(_) if matches!(self.peek2(), Tok::ColonEq | Tok::Colon) => ItemKind::Const(self.const_decl()?),
            Tok::Kw(Kw::Var) => {
                let span = self.span();
                self.report_help(
                    span,
                    "a top-level binding cannot be `var`",
                    "top-level bindings are constants (Section 3.5); state that changes belongs in an actor or in `main`",
                );
                return Err(());
            }
            _ => return self.expected("a declaration"),
        };
        Ok(Item {
            docs,
            attrs,
            public,
            kind,
            span: start.to(self.prev_span()),
        })
    }

    fn const_decl(&mut self) -> PResult<ConstDecl> {
        let name = self.binding_name("constant name")?;
        if self.eat(&Tok::ColonEq) {
            let value = self.expr()?;
            return Ok(ConstDecl { name, ty: None, value });
        }
        self.expect(&Tok::Colon, "`:=` or `:`")?;
        let ty = self.ty()?;
        self.expect(&Tok::Eq, "`=`")?;
        let value = self.expr()?;
        Ok(ConstDecl {
            name,
            ty: Some(ty),
            value,
        })
    }

    pub(super) fn type_params(&mut self) -> PResult<Vec<TypeParam>> {
        if !self.at(&Tok::Lt) {
            return Ok(Vec::new());
        }
        self.bump();
        let mut params = Vec::new();
        while !self.at(&Tok::Gt) {
            let name = self.upper_name("type parameter")?;
            let constraint = if self.eat(&Tok::Colon) {
                if self.eat_kw(Kw::Resource) {
                    Constraint::Resource
                } else {
                    let mut interfaces = vec![self.named_type()?];
                    while self.eat(&Tok::Plus) {
                        interfaces.push(self.named_type()?);
                    }
                    Constraint::Interfaces(interfaces)
                }
            } else {
                Constraint::None
            };
            params.push(TypeParam { name, constraint });
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(&Tok::Gt, "`,` or `>`")?;
        if params.is_empty() {
            let span = self.prev_span();
            self.report(span, "an empty type parameter list");
        }
        Ok(params)
    }

    /// `: Interface + Interface`, the interfaces a declared type states it satisfies.
    fn satisfies(&mut self) -> PResult<Vec<Type>> {
        let mut interfaces = Vec::new();
        if self.eat(&Tok::Colon) {
            interfaces.push(self.named_type()?);
            while self.eat(&Tok::Plus) {
                interfaces.push(self.named_type()?);
            }
        }
        Ok(interfaces)
    }

    fn type_decl(&mut self, resource: bool) -> PResult<TypeDecl> {
        self.bump();
        let name = self.upper_name("type name")?;
        let type_params = self.type_params()?;
        let satisfies = self.satisfies()?;
        self.expect(&Tok::Eq, "`=`")?;
        let body = if self.at(&Tok::LBrace) {
            TypeBody::Struct(self.struct_fields()?)
        } else if resource {
            return self.expected("`{`: a resource is declared with a struct type");
        } else if self.eat_kw(Kw::Distinct) {
            TypeBody::Distinct(self.ty()?)
        } else {
            TypeBody::Alias(self.ty()?)
        };
        Ok(TypeDecl {
            resource,
            name,
            type_params,
            satisfies,
            body,
        })
    }

    /// `{ name: Type, ... }`: the body of a struct type, an error or an enum variant.
    pub(super) fn struct_fields(&mut self) -> PResult<Vec<Field>> {
        self.delimited(&Tok::LBrace, &Tok::RBrace, |p| {
            let docs = p.docs_before(p.span());
            let name = p.member_name("a field name")?;
            if name.is_upper() || name.name.bytes().any(|b| b.is_ascii_uppercase()) {
                p.report_help(
                    name.span,
                    format!("`{}` is not snake_case", name.name),
                    "a field is written in snake_case (Section 3.1)",
                );
            }
            p.expect(&Tok::Colon, "`:` and the field's type")?;
            let ty = p.field_type()?;
            let attrs = p.attributes(false)?;
            let default = if p.eat(&Tok::Eq) { Some(p.expr()?) } else { None };
            Ok(Field {
                docs,
                name,
                ty,
                attrs,
                default,
            })
        })
    }

    fn enum_decl(&mut self) -> PResult<EnumDecl> {
        self.bump();
        let name = self.upper_name("enum name")?;
        let type_params = self.type_params()?;
        let satisfies = self.satisfies()?;
        self.expect(&Tok::LBrace, "`{`")?;
        let saved = self.enter(true);
        let mut variants = Vec::new();
        loop {
            self.skip_newlines();
            if self.at(&Tok::RBrace) {
                break;
            }
            let docs = self.docs_before(self.span());
            let name = self.upper_name("variant name")?;
            let fields = if self.at(&Tok::LBrace) {
                Some(self.struct_fields()?)
            } else {
                None
            };
            variants.push(Variant { docs, name, fields });
            // In an enum body a comma may stand for the newline (Section 3.7).
            if !self.eat(&Tok::Comma) && !matches!(self.peek(), Tok::Newline | Tok::RBrace) {
                return self.expected("a new line, `,` or `}` after the variant");
            }
        }
        self.bump();
        self.leave(saved);
        Ok(EnumDecl {
            name,
            type_params,
            satisfies,
            variants,
        })
    }

    fn error_decl(&mut self) -> PResult<ErrorDecl> {
        self.bump();
        let name = self.upper_name("error name")?;
        let body = if self.eat(&Tok::Eq) {
            ErrorBody::Set(self.error_set()?)
        } else if self.at(&Tok::LBrace) {
            ErrorBody::Fields(self.struct_fields()?)
        } else {
            ErrorBody::Fields(Vec::new())
        };
        Ok(ErrorDecl { name, body })
    }

    fn interface_decl(&mut self, resource: bool) -> PResult<InterfaceDecl> {
        self.bump();
        let name = self.upper_name("interface name")?;
        let type_params = self.type_params()?;
        self.expect(&Tok::LBrace, "`{`")?;
        let saved = self.enter(true);
        let mut members = Vec::new();
        loop {
            self.skip_newlines();
            if self.at(&Tok::RBrace) {
                break;
            }
            let docs = self.docs_before(self.span());
            if !self.at_kw(Kw::Fn) {
                return self.expected("`fn` or `}`: an interface holds function signatures");
            }
            let decl = self.fn_decl(FnCtx::Interface)?;
            members.push(InterfaceMember { docs, sig: decl.sig });
            if !matches!(self.peek(), Tok::Newline | Tok::RBrace) {
                return self.expected("the end of the line");
            }
        }
        self.bump();
        self.leave(saved);
        Ok(InterfaceDecl {
            resource,
            name,
            type_params,
            members,
        })
    }

    fn actor_decl(&mut self) -> PResult<ActorDecl> {
        self.bump();
        let name = self.upper_name("actor name")?;
        let type_params = self.type_params()?;
        let satisfies = self.satisfies()?;
        self.expect(&Tok::LBrace, "`{`")?;
        let saved = self.enter(true);
        let mut fields = Vec::new();
        let mut methods = Vec::new();
        loop {
            self.skip_newlines();
            if self.at(&Tok::RBrace) {
                break;
            }
            let docs = self.docs_before(self.span());
            if self.at_kw(Kw::Fn) || self.at_kw(Kw::Pub) {
                let public = self.eat_kw(Kw::Pub);
                if !self.at_kw(Kw::Fn) {
                    return self.expected("`fn`: only an actor's methods may be `pub`");
                }
                let decl = self.fn_decl(FnCtx::Actor)?;
                methods.push(ActorMethod { docs, public, decl });
            } else {
                let mutable = self.eat_kw(Kw::Var);
                if !matches!(self.peek(), Tok::Ident(_) | Tok::UpperIdent(_)) {
                    return self.expected("a field, a method or `}`");
                }
                let name = self.binding_name("field name")?;
                self.expect(&Tok::Colon, "`:` and the field's type")?;
                let ty = self.ty()?;
                let default = if self.eat(&Tok::Eq) { Some(self.expr()?) } else { None };
                fields.push(ActorField {
                    docs,
                    mutable,
                    name,
                    ty,
                    default,
                });
            }
            if !matches!(self.peek(), Tok::Newline | Tok::RBrace) {
                return self.expected("the end of the line");
            }
        }
        self.bump();
        self.leave(saved);
        Ok(ActorDecl {
            name,
            type_params,
            satisfies,
            fields,
            methods,
        })
    }

    // ---- functions ----------------------------------------------------------------------

    fn fn_decl(&mut self, ctx: FnCtx) -> PResult<FnDecl> {
        let sig = self.fn_sig(ctx)?;
        let body = if self.at(&Tok::LBrace) {
            if ctx == FnCtx::Interface {
                let span = self.span();
                return self.fail(span, "a function in an interface has no body");
            }
            Some(self.block()?)
        } else if ctx == FnCtx::Interface || self.fragment {
            None
        } else {
            return self.expected("`{`: a function needs a body");
        };
        Ok(FnDecl { sig, body })
    }

    fn fn_sig(&mut self, ctx: FnCtx) -> PResult<FnSig> {
        let start = self.expect_kw(Kw::Fn)?.span;
        let target = if self.at(&Tok::LParen) {
            // A receiver: `(self: Order)`, `(self: var Order)`, and in an interface `(self: var)`.
            let open = self.bump().span;
            let saved = self.enter(false);
            if *self.peek() != Tok::Ident("self".to_string()) {
                return self.expected("`self`: a receiver is written `(self: Type)`");
            }
            self.bump();
            self.expect(&Tok::Colon, "`:`")?;
            let kind = self.param_kind();
            let ty = if self.at(&Tok::RParen) { None } else { Some(self.ty()?) };
            self.expect(&Tok::RParen, "`)`")?;
            self.leave(saved);
            match (ctx, &ty) {
                (FnCtx::Actor, _) => {
                    self.report_help(
                        open,
                        "a method of an actor has no receiver",
                        "inside an actor, fields are used by name and `self` is the actor's handle (Section 7.3)",
                    );
                }
                (FnCtx::Interface, Some(ty)) => {
                    self.report_help(
                        ty.span,
                        "a receiver in an interface has no type",
                        "write `(self: var)` or `(self: take)`, or leave the receiver out to borrow",
                    );
                }
                (FnCtx::Top, None) => {
                    self.report(open, "a receiver needs a type: `(self: Order)`");
                }
                _ => {}
            }
            FnTarget::Method { kind, ty }
        } else {
            let type_level = match self.peek() {
                Tok::UpperIdent(_) => true,
                Tok::Ident(name) => is_primitive(name) && *self.peek2() == Tok::Dot,
                _ => false,
            };
            if type_level {
                let ty = self.named_type()?;
                self.expect(
                    &Tok::Dot,
                    "`.` and the function's name: only types have capitalized names",
                )?;
                if ctx == FnCtx::Actor {
                    self.report(ty.span, "a type-level function cannot be declared inside an actor");
                }
                FnTarget::TypeLevel(ty)
            } else {
                FnTarget::Free
            }
        };
        let name = self.member_name("a function name")?;
        if name.is_upper() || name.name.bytes().any(|b| b.is_ascii_uppercase()) {
            self.report_help(
                name.span,
                format!("`{}` is not snake_case", name.name),
                "a function is written in snake_case (Section 3.1)",
            );
        }
        let type_params = self.type_params()?;
        let params = self.params(ctx)?;
        let ret = if self.eat(&Tok::Arrow) {
            Some(self.ty_no_result(false)?)
        } else {
            None
        };
        let errors = if self.eat(&Tok::Bang) {
            if matches!(self.peek(), Tok::UpperIdent(_) | Tok::Ident(_)) {
                ErrorSpec::Set(self.error_set()?)
            } else {
                ErrorSpec::Inferred
            }
        } else {
            ErrorSpec::None
        };
        Ok(FnSig {
            target,
            name,
            type_params,
            params,
            ret,
            errors,
            span: start.to(self.prev_span()),
        })
    }

    pub(super) fn param_kind(&mut self) -> ParamKind {
        if self.eat_kw(Kw::Var) {
            ParamKind::Var
        } else if self.eat_kw(Kw::Take) {
            ParamKind::Take
        } else {
            ParamKind::Borrow
        }
    }

    fn params(&mut self, ctx: FnCtx) -> PResult<Vec<Param>> {
        let params = self.delimited(&Tok::LParen, &Tok::RParen, |p| {
            let name = p.binding_name("parameter name")?;
            p.expect(&Tok::Colon, "`:` and the parameter's type")?;
            let variadic = p.eat(&Tok::Ellipsis);
            let kind_span = p.span();
            let kind = p.param_kind();
            if kind != ParamKind::Borrow && (variadic || ctx == FnCtx::Actor) {
                let why = if variadic {
                    "a variadic parameter is always borrowed"
                } else {
                    "every parameter of an actor method is `take` already (Section 7.3)"
                };
                p.report(kind_span, why);
            }
            let ty = p.ty()?;
            let default = if p.eat(&Tok::Eq) { Some(p.expr()?) } else { None };
            Ok(Param {
                name,
                kind,
                variadic,
                ty,
                default,
            })
        })?;
        if let Some(i) = params.iter().position(|p| p.variadic)
            && i + 1 != params.len()
        {
            self.report(params[i].name.span, "a variadic parameter must be the last one");
        }
        Ok(params)
    }
}
