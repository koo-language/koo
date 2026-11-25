//! Types.

use super::*;

impl Parser {
    /// A type, which may be a result type `T ! E1 | E2`.
    pub(super) fn ty(&mut self) -> PResult<Type> {
        self.ty_with(false)
    }

    /// A field's type, which may also contain an inline struct type (Section 4.6).
    pub(super) fn field_type(&mut self) -> PResult<Type> {
        self.ty_with(true)
    }

    fn ty_with(&mut self, allow_struct: bool) -> PResult<Type> {
        let ok = self.ty_no_result(allow_struct)?;
        if !self.at(&Tok::Bang) {
            return Ok(ok);
        }
        self.bump();
        let errors = self.error_set()?;
        let span = ok.span.to(self.prev_span());
        Ok(Type {
            kind: TypeKind::Result {
                ok: Box::new(ok),
                errors,
            },
            span,
        })
    }

    /// `E1 | E2`: the error types after `!`.
    pub(super) fn error_set(&mut self) -> PResult<Vec<Type>> {
        let mut errors = vec![self.named_type()?];
        while self.eat(&Tok::Pipe) {
            errors.push(self.named_type()?);
        }
        Ok(errors)
    }

    /// A type that is not a result type at its top level.
    pub(super) fn ty_no_result(&mut self, allow_struct: bool) -> PResult<Type> {
        self.nested(|p| p.ty_no_result_unguarded(allow_struct))
    }

    fn ty_no_result_unguarded(&mut self, allow_struct: bool) -> PResult<Type> {
        let start = self.span();
        let mut ty = match self.peek() {
            Tok::UpperIdent(_) | Tok::Ident(_) => self.named_type()?,
            Tok::LBracket => {
                self.bump();
                let saved = self.enter(false);
                let element = self.ty_with(allow_struct)?;
                let kind = if self.eat(&Tok::Colon) {
                    let value = self.ty_with(allow_struct)?;
                    TypeKind::Map(Box::new(element), Box::new(value))
                } else if self.eat(&Tok::Semi) {
                    let size = self.expr()?;
                    TypeKind::Array(Box::new(element), Box::new(size))
                } else {
                    TypeKind::List(Box::new(element))
                };
                self.expect(&Tok::RBracket, "`]`")?;
                self.leave(saved);
                Type {
                    kind,
                    span: start.to(self.prev_span()),
                }
            }
            Tok::LParen => {
                let mut types = self.delimited(&Tok::LParen, &Tok::RParen, |p| p.ty_with(allow_struct))?;
                let span = start.to(self.prev_span());
                if types.len() == 1 {
                    // Parentheses around one type only group: `(Order ! NotFound)?`.
                    let inner = types.pop().unwrap();
                    Type { kind: inner.kind, span }
                } else {
                    Type {
                        kind: TypeKind::Tuple(types),
                        span,
                    }
                }
            }
            Tok::Kw(Kw::Fn) => self.fn_type(false)?,
            Tok::Kw(Kw::Resource) if *self.peek2() == Tok::Kw(Kw::Fn) => {
                self.bump();
                self.fn_type(true)?
            }
            Tok::LBrace if allow_struct => {
                let fields = self.struct_fields()?;
                Type {
                    kind: TypeKind::Struct(fields),
                    span: start.to(self.prev_span()),
                }
            }
            Tok::LBrace => {
                let span = self.span();
                self.report_help(
                    span,
                    "a struct type cannot be written here",
                    "declare it with `type Name = { ... }`; only a field's type may be an inline struct type (Section 4.6)",
                );
                return Err(());
            }
            Tok::Kw(Kw::Enum) => {
                let span = self.span();
                self.diag(
                    Diagnostic::new(span, "expected a type, found `enum`")
                        .with_note("an enum is a declaration of its own, not a type written after `type Name =`")
                        .with_help("write `enum Color { Red, Green, Blue }`; a variant may have fields: `Circle { radius: float }` (Section 4.7)"),
                );
                return Err(());
            }
            _ => return self.expected("a type"),
        };
        while self.at(&Tok::Question) {
            self.bump();
            self.link()?;
            let span = start.to(self.prev_span());
            ty = Type {
                kind: TypeKind::Optional(Box::new(ty)),
                span,
            };
        }
        Ok(ty)
    }

    /// `fn(A, var B) -> R ! E`, at the `fn`. In a function type, `!` after the return type
    /// belongs to the function type.
    fn fn_type(&mut self, resource: bool) -> PResult<Type> {
        let start = if resource { self.prev_span() } else { self.span() };
        self.bump();
        let params = self.delimited(&Tok::LParen, &Tok::RParen, |p| {
            let kind = p.param_kind();
            Ok((kind, p.ty()?))
        })?;
        let ret = if self.eat(&Tok::Arrow) {
            Some(Box::new(self.ty_no_result(false)?))
        } else {
            None
        };
        let errors = if self.eat(&Tok::Bang) {
            self.error_set()?
        } else {
            Vec::new()
        };
        Ok(Type {
            kind: TypeKind::Fn {
                resource,
                params,
                ret,
                errors,
            },
            span: start.to(self.prev_span()),
        })
    }

    /// A type written by name: `int`, `Order`, `sql.Pool`, `Cache<string, Order>`,
    /// `Reply.User`, `c.order`.
    pub(super) fn named_type(&mut self) -> PResult<Type> {
        let start = self.span();
        let mut path = Vec::new();
        match self.peek() {
            Tok::UpperIdent(_) => path.push(Self::ident_from(self.bump())),
            Tok::Ident(name) => {
                let primitive = is_primitive(name);
                let first = Self::ident_from(self.bump());
                if primitive {
                    return Ok(Type {
                        kind: TypeKind::Named {
                            path: vec![first],
                            args: Vec::new(),
                        },
                        span: start,
                    });
                }
                if !self.at(&Tok::Dot) {
                    let mut diagnostic =
                        Diagnostic::new(first.span, format!("expected a type, found `{}`", first.name));
                    diagnostic = match (first.name.as_str(), super::habits::koo_type(&first.name)) {
                        (_, Some(koo)) => diagnostic
                            .with_note("the number types are `int` (64 bits), `float` (64 bits) and `byte`; text is `string`")
                            .with_fix(format!("in Koo that is `{koo}`"), first.span, koo),
                        ("void" | "unit" | "nil" | "none" | "null", _) => diagnostic
                            .with_help("a function that returns nothing declares no return type: leave out the `->` and the type"),
                        ("list" | "array" | "vec" | "slice", _) => diagnostic.with_help("a list of `T` is written `[T]`"),
                        ("map" | "dict" | "hash" | "hashmap", _) => {
                            diagnostic.with_help("a map from `K` to `V` is written `[K: V]`")
                        }
                        ("struct" | "class" | "record", _) => diagnostic
                            .with_help("a type with fields is written with braces alone: `type Point = { x: int, y: int }`"),
                        ("any" | "object" | "dynamic" | "auto" | "var", _) => diagnostic
                            .with_help("Koo has no type that holds anything: name the type, or use a type parameter, `fn first<T>(items: [T]) -> T?`"),
                        ("char" | "rune", _) => diagnostic
                            .with_help("a string is made of bytes and read as characters: `s.chars()` gives each as a `string`"),
                        _ => diagnostic.with_help(
                            "a type name is capitalized, or is a primitive type such as `int` or `string` (Section 3.1)",
                        ),
                    };
                    self.diag(diagnostic);
                    return Err(());
                }
                // A package prefix. After it the name is usually capitalized, but a name
                // imported from C keeps its C spelling (Section 11.1).
                self.bump();
                let c_prefix = self.c_prefixes.contains(&first.name);
                path.push(first);
                match self.peek() {
                    Tok::UpperIdent(_) => path.push(Self::ident_from(self.bump())),
                    Tok::Ident(_) => {
                        let name = Self::ident_from(self.bump());
                        if !c_prefix {
                            self.report_help(
                                name.span,
                                format!("expected a type name, found `{}`", name.name),
                                "a type name is capitalized; only a name imported from C keeps another spelling (Section 11.1)",
                            );
                        }
                        path.push(name);
                    }
                    _ => return self.expected("a type name"),
                }
            }
            _ => return self.expected("a type name"),
        }
        let args = if self.at(&Tok::Lt) {
            self.type_args()?
        } else {
            Vec::new()
        };
        while self.at(&Tok::Dot) && matches!(self.peek2(), Tok::UpperIdent(_)) {
            self.bump();
            path.push(Self::ident_from(self.bump()));
        }
        Ok(Type {
            kind: TypeKind::Named { path, args },
            span: start.to(self.prev_span()),
        })
    }

    /// `<T, U>` at the `<`.
    pub(super) fn type_args(&mut self) -> PResult<Vec<Type>> {
        self.expect(&Tok::Lt, "`<`")?;
        let mut args = Vec::new();
        loop {
            args.push(self.ty()?);
            if !self.eat(&Tok::Comma) || matches!(self.peek(), Tok::Gt | Tok::Ge) {
                break;
            }
        }
        if self.at(&Tok::Ge) {
            // `List<int>= ...`: the lexer joined `>` and `=`. Split them.
            let token = &mut self.toks[self.pos];
            token.kind = Tok::Eq;
            token.span.start += 1;
            token.nl_before = false;
            return Ok(args);
        }
        self.expect(&Tok::Gt, "`,` or `>`")?;
        Ok(args)
    }
}
