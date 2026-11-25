//! Expressions. The ten precedence levels are those of Section 3.4.

use super::*;
use crate::token::StrPiece;

impl Parser {
    /// Level 10: `x else y`, right-associative.
    pub(super) fn expr(&mut self) -> PResult<Expr> {
        self.nested(|p| p.expr_unguarded())
    }

    fn expr_unguarded(&mut self) -> PResult<Expr> {
        let expr = self.or_expr()?;
        if !self.at_kw(Kw::Else) {
            return Ok(expr);
        }
        self.bump();
        // `else e => alternative` binds the error (Section 6.4).
        let binding = if self.else_bind_ok && matches!(self.peek(), Tok::Ident(_)) && *self.peek2() == Tok::FatArrow {
            let name = self.binding_name("variable name")?;
            self.bump();
            Some(name)
        } else {
            None
        };
        let alt = self.body(false)?;
        let span = expr.span.to(self.prev_span());
        Ok(Expr {
            kind: ExprKind::Else {
                expr: Box::new(expr),
                binding,
                alt: Box::new(alt),
            },
            span,
        })
    }

    fn binary(lhs: Expr, op: BinaryOp, rhs: Expr) -> Expr {
        let span = lhs.span.to(rhs.span);
        Expr {
            kind: ExprKind::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            },
            span,
        }
    }

    /// Level 9.
    fn or_expr(&mut self) -> PResult<Expr> {
        self.chain(|p| p.or_links())
    }

    fn or_links(&mut self) -> PResult<Expr> {
        let mut lhs = self.and_expr()?;
        while self.eat_kw(Kw::Or) {
            self.link()?;
            let rhs = self.and_expr()?;
            lhs = Self::binary(lhs, BinaryOp::Or, rhs);
        }
        Ok(lhs)
    }

    /// Level 8.
    fn and_expr(&mut self) -> PResult<Expr> {
        self.chain(|p| p.and_links())
    }

    fn and_links(&mut self) -> PResult<Expr> {
        let mut lhs = self.not_expr()?;
        while self.eat_kw(Kw::And) {
            self.link()?;
            let rhs = self.not_expr()?;
            lhs = Self::binary(lhs, BinaryOp::And, rhs);
        }
        Ok(lhs)
    }

    /// Level 7.
    fn not_expr(&mut self) -> PResult<Expr> {
        if !self.at_kw(Kw::Not) {
            return self.comparison();
        }
        let start = self.bump().span;
        let operand = self.nested(|p| p.not_expr())?;
        let span = start.to(operand.span);
        Ok(Expr {
            kind: ExprKind::Unary {
                op: UnaryOp::Not,
                expr: Box::new(operand),
            },
            span,
        })
    }

    fn comparison_op(&self) -> Option<BinaryOp> {
        Some(match self.peek() {
            Tok::EqEq => BinaryOp::Eq,
            Tok::NotEq => BinaryOp::Ne,
            Tok::Lt => BinaryOp::Lt,
            Tok::Le => BinaryOp::Le,
            Tok::Gt => BinaryOp::Gt,
            Tok::Ge => BinaryOp::Ge,
            _ => return None,
        })
    }

    /// Level 6: comparisons and `is`, which do not associate.
    fn comparison(&mut self) -> PResult<Expr> {
        let lhs = self.range()?;
        let expr = if let Some(op) = self.comparison_op() {
            self.bump();
            let rhs = self.range()?;
            Self::binary(lhs, op, rhs)
        } else if self.eat_kw(Kw::Is) {
            let ty = self.named_type()?;
            let span = lhs.span.to(ty.span);
            Expr {
                kind: ExprKind::Is {
                    expr: Box::new(lhs),
                    ty,
                },
                span,
            }
        } else {
            return Ok(lhs);
        };
        if self.comparison_op().is_some() || self.at_kw(Kw::Is) {
            let span = self.span();
            self.report_help(
                span,
                "comparisons do not chain",
                "write `a < b and b < c`, or parenthesize one comparison",
            );
            return Err(());
        }
        Ok(expr)
    }

    /// Level 5: `a..b` and `a..=b`, which do not associate.
    fn range(&mut self) -> PResult<Expr> {
        let lhs = self.additive()?;
        let op = match self.peek() {
            Tok::DotDot => BinaryOp::Range,
            Tok::DotDotEq => BinaryOp::RangeInclusive,
            _ => return Ok(lhs),
        };
        self.bump();
        let rhs = self.additive()?;
        if matches!(self.peek(), Tok::DotDot | Tok::DotDotEq) {
            let span = self.span();
            return self.fail(span, "ranges do not chain");
        }
        Ok(Self::binary(lhs, op, rhs))
    }

    /// Level 4.
    fn additive(&mut self) -> PResult<Expr> {
        self.chain(|p| p.additive_links())
    }

    fn additive_links(&mut self) -> PResult<Expr> {
        let mut lhs = self.multiplicative()?;
        loop {
            let op = match self.peek() {
                Tok::Plus => BinaryOp::Add,
                Tok::Minus => BinaryOp::Sub,
                _ => return Ok(lhs),
            };
            self.bump();
            self.link()?;
            let rhs = self.multiplicative()?;
            lhs = Self::binary(lhs, op, rhs);
        }
    }

    /// Level 3.
    fn multiplicative(&mut self) -> PResult<Expr> {
        self.chain(|p| p.multiplicative_links())
    }

    fn multiplicative_links(&mut self) -> PResult<Expr> {
        let mut lhs = self.unary()?;
        loop {
            let op = match self.peek() {
                Tok::Star => BinaryOp::Mul,
                Tok::Slash => BinaryOp::Div,
                Tok::Percent => BinaryOp::Rem,
                _ => return Ok(lhs),
            };
            self.bump();
            self.link()?;
            let rhs = self.unary()?;
            lhs = Self::binary(lhs, op, rhs);
        }
    }

    /// Level 2.
    fn unary(&mut self) -> PResult<Expr> {
        if !self.at(&Tok::Minus) {
            return self.postfix();
        }
        let start = self.bump().span;
        let operand = self.nested(|p| p.unary())?;
        let span = start.to(operand.span);
        Ok(Expr {
            kind: ExprKind::Unary {
                op: UnaryOp::Neg,
                expr: Box::new(operand),
            },
            span,
        })
    }

    /// Whether `expr` is a path of names that ends in a capitalized one, so that `{` after
    /// it opens a struct literal and `<` a type-argument list. A name after a C import's
    /// prefix is never one (Section 11.1).
    fn is_type_path(&self, expr: &Expr) -> bool {
        fn pure(expr: &Expr) -> bool {
            match &expr.kind {
                ExprKind::Name(_) | ExprKind::TypeApp { .. } => true,
                ExprKind::Field { base, .. } => pure(base),
                _ => false,
            }
        }
        match &expr.kind {
            ExprKind::Name(name) => name.is_upper(),
            ExprKind::TypeApp { .. } => true,
            ExprKind::Field { base, name } => {
                let c_name = matches!(&base.kind, ExprKind::Name(prefix) if self.c_prefixes.contains(&prefix.name));
                name.is_upper() && pure(base) && !c_name
            }
            _ => false,
        }
    }

    /// After a capitalized name: a type-argument list, if one follows (Section 3.9).
    fn type_app(&mut self, base: Expr) -> PResult<Expr> {
        if !self.at(&Tok::Lt) || !self.is_type_path(&base) {
            return Ok(base);
        }
        const HELP: &str = "a capitalized name followed by `<` begins a type-argument list; to compare with `<`, parenthesize the left side (Section 3.9)";
        let reported = self.diags.len();
        let Ok(args) = self.type_args() else {
            if let Some(diagnostic) = self.diags.get_mut(reported) {
                diagnostic.help = vec![HELP.to_string()];
            }
            return Err(());
        };
        let span = base.span.to(self.prev_span());
        if !matches!(self.peek(), Tok::Dot | Tok::LBrace) {
            let here = self.here();
            self.report_help(
                here,
                format!(
                    "expected `.` or `{{` after the type arguments, found {}",
                    self.peek().describe()
                ),
                HELP,
            );
            return Err(());
        }
        Ok(Expr {
            kind: ExprKind::TypeApp {
                base: Box::new(base),
                args,
            },
            span,
        })
    }

    /// Level 1: field, call, index, propagate, and the struct literal.
    fn postfix(&mut self) -> PResult<Expr> {
        self.chain(|p| p.postfix_links())
    }

    fn postfix_links(&mut self) -> PResult<Expr> {
        let mut expr = self.primary()?;
        expr = self.type_app(expr)?;
        loop {
            if matches!(
                self.peek(),
                Tok::Dot | Tok::LParen | Tok::LBracket | Tok::Question | Tok::LBrace
            ) {
                self.link()?;
            }
            match self.peek() {
                Tok::Dot => {
                    self.bump();
                    let name = self.member_name("a member name")?;
                    let span = expr.span.to(name.span);
                    expr = Expr {
                        kind: ExprKind::Field {
                            base: Box::new(expr),
                            name,
                        },
                        span,
                    };
                    expr = self.type_app(expr)?;
                }
                Tok::LParen => {
                    let args = self.args()?;
                    let span = expr.span.to(self.prev_span());
                    expr = Expr {
                        kind: ExprKind::Call {
                            callee: Box::new(expr),
                            args,
                        },
                        span,
                    };
                }
                Tok::LBracket => {
                    self.bump();
                    let saved = self.enter(false);
                    let index = self.expr()?;
                    self.expect(&Tok::RBracket, "`]`")?;
                    self.leave(saved);
                    let span = expr.span.to(self.prev_span());
                    expr = Expr {
                        kind: ExprKind::Index {
                            base: Box::new(expr),
                            index: Box::new(index),
                        },
                        span,
                    };
                }
                Tok::Question => {
                    self.bump();
                    let span = expr.span.to(self.prev_span());
                    expr = Expr {
                        kind: ExprKind::Try(Box::new(expr)),
                        span,
                    };
                }
                Tok::LBrace if self.struct_lit_ok && self.is_type_path(&expr) => {
                    let fields = self.delimited(&Tok::LBrace, &Tok::RBrace, |p| {
                        let name = p.member_name("a field name")?;
                        let value = if p.eat(&Tok::Colon) {
                            Some(p.expr()?)
                        } else {
                            if !matches!(p.toks[p.pos - 1].kind, Tok::Ident(_)) {
                                p.report(
                                    name.span,
                                    format!("the field `{}` needs a value: `{}: ...`", name.name, name.name),
                                );
                            }
                            None
                        };
                        Ok(FieldInit { name, value })
                    })?;
                    let span = expr.span.to(self.prev_span());
                    expr = Expr {
                        kind: ExprKind::StructLit {
                            ty: Box::new(expr),
                            fields,
                        },
                        span,
                    };
                }
                _ => return Ok(expr),
            }
        }
    }

    /// `(a, name: b, var c)`.
    fn args(&mut self) -> PResult<Vec<Arg>> {
        let mut named_seen = false;
        self.delimited(&Tok::LParen, &Tok::RParen, |p| {
            let name = if matches!(p.peek(), Tok::Ident(_)) && *p.peek2() == Tok::Colon {
                let name = p.lower_name("parameter name")?;
                p.bump();
                named_seen = true;
                Some(name)
            } else {
                if named_seen {
                    let span = p.span();
                    p.report_help(
                        span,
                        "a positional argument follows a named one",
                        "after the first named argument, every argument is named (Section 6.1)",
                    );
                }
                None
            };
            let var_at = p.span().start;
            let var = p.eat_kw(Kw::Var);
            let value = p.expr()?;
            Ok(Arg {
                name,
                var,
                var_at,
                value,
            })
        })
    }

    /// A literal token as an expression.
    pub(super) fn literal(&mut self) -> PResult<Expr> {
        let token = self.bump();
        let kind = match token.kind {
            Tok::Int(s) => ExprKind::Int(s),
            Tok::Float(s) => ExprKind::Float(s),
            Tok::Decimal(s) => ExprKind::Decimal(s),
            Tok::Duration(s) => ExprKind::Duration(s),
            Tok::Char(c) => ExprKind::Char(c),
            Tok::RawStr(s) => ExprKind::RawStr(s),
            Tok::True => ExprKind::Bool(true),
            Tok::False => ExprKind::Bool(false),
            Tok::NoneLit => ExprKind::None,
            Tok::Str(pieces) => {
                let mut parts = Vec::new();
                for piece in pieces {
                    match piece {
                        StrPiece::Lit(text) => parts.push(StrPart::Lit(text)),
                        StrPiece::Interp { tokens, format, span } => {
                            if let Some(expr) = self.interpolated(tokens, span) {
                                parts.push(StrPart::Interp { expr, format });
                            }
                        }
                    }
                }
                ExprKind::Str(parts)
            }
            other => unreachable!("not a literal: {other:?}"),
        };
        Ok(Expr { kind, span: token.span })
    }

    /// Parses the expression inside `{...}` in a string. Its errors are reported, and the
    /// rest of the string is still parsed.
    fn interpolated(&mut self, mut tokens: Vec<Token>, span: Span) -> Option<Expr> {
        if tokens.is_empty() {
            return None;
        }
        tokens.push(Token {
            kind: Tok::Eof,
            span: Span {
                start: span.end,
                end: span.end,
            },
            nl_before: false,
        });
        let mut inner = Parser::new(tokens, false);
        inner.nl_sig.push(false);
        inner.nesting = self.nesting + 1;
        inner.c_prefixes = self.c_prefixes.clone();
        let result = inner.expr();
        if result.is_ok() && !inner.at(&Tok::Eof) {
            let _ = inner.expected::<()>("`}` or `:` and a format");
        }
        self.diags.append(&mut inner.diags);
        result.ok()
    }

    fn primary(&mut self) -> PResult<Expr> {
        let start = self.span();
        let kind = match self.peek() {
            Tok::Int(_)
            | Tok::Float(_)
            | Tok::Decimal(_)
            | Tok::Duration(_)
            | Tok::Char(_)
            | Tok::Str(_)
            | Tok::RawStr(_)
            | Tok::True
            | Tok::False
            | Tok::NoneLit => return self.literal(),
            Tok::Ident(_) | Tok::Underscore if *self.peek2() == Tok::Arrow => {
                // `x -> body`.
                let param = self.lambda_param()?;
                self.bump();
                let body = self.body(true)?;
                ExprKind::Lambda {
                    params: vec![param],
                    body: Box::new(body),
                }
            }
            Tok::Ident(word)
                if matches!(word.as_str(), "f" | "F")
                    && matches!(self.raw(1), Tok::Str(_))
                    && self.toks[self.pos + 1].span.start == self.span().end =>
            {
                // `f"total: {n}"`: the string of Python. It is read as the string it is.
                let span = self.span();
                self.diag(
                    Diagnostic::new(span, "a string needs no `f` to hold values")
                        .with_note("every string takes values in braces: `\"total: {n}\"`")
                        .with_fix("remove it", span, ""),
                );
                self.bump();
                return self.primary();
            }
            Tok::Ident(_) | Tok::UpperIdent(_) => ExprKind::Name(Self::ident_from(self.bump())),
            Tok::LParen if self.paren_then(&Tok::Arrow) => {
                // `(a, b) -> body`, `(e: NotFound) -> body`, `() -> body`.
                let params = self.delimited(&Tok::LParen, &Tok::RParen, |p| p.lambda_param())?;
                self.bump();
                let body = self.body(true)?;
                ExprKind::Lambda {
                    params,
                    body: Box::new(body),
                }
            }
            Tok::LParen => {
                self.bump();
                let saved = self.enter(false);
                let mut items = Vec::new();
                let mut comma = false;
                while !self.at(&Tok::RParen) {
                    items.push(self.expr()?);
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                    comma = true;
                }
                self.expect(&Tok::RParen, "`,` or `)`")?;
                self.leave(saved);
                if items.len() == 1 && !comma {
                    ExprKind::Paren(Box::new(items.pop().unwrap()))
                } else {
                    if items.len() == 1 {
                        let span = start.to(self.prev_span());
                        self.report_help(
                            span,
                            "a tuple has two or more elements",
                            "remove the trailing comma; `()` is the empty tuple (Section 4.5)",
                        );
                    }
                    ExprKind::Tuple(items)
                }
            }
            Tok::LBracket => self.list_or_map()?,
            Tok::Kw(Kw::If) => {
                let if_expr = self.if_expr()?;
                self.check_if(&if_expr, true);
                ExprKind::If(Box::new(if_expr))
            }
            Tok::Kw(Kw::Match) => return self.match_expr(),
            Tok::Underscore => {
                let span = self.span();
                self.report_help(
                    span,
                    "`_` is not a value",
                    "`_` stands for something ignored, in a pattern or a binding",
                );
                return Err(());
            }
            Tok::Bang => {
                let span = self.span();
                let glued = self.toks[(self.pos + 1).min(self.toks.len() - 1)].span.start == span.end;
                self.diag(
                    Diagnostic::new(span, "expected an expression, found `!`")
                        .with_note("`!` introduces the errors of a function: `fn f() -> T ! E`")
                        .with_fix(
                            "logical negation is written `not`",
                            span,
                            if glued { "not " } else { "not" },
                        ),
                );
                return Err(());
            }
            _ => return self.expected("an expression"),
        };
        Ok(Expr {
            kind,
            span: start.to(self.prev_span()),
        })
    }

    fn lambda_param(&mut self) -> PResult<LambdaParam> {
        let start = self.span();
        let name = if self.eat(&Tok::Underscore) {
            None
        } else {
            Some(self.binding_name("parameter name")?)
        };
        let (kind, ty) = if self.eat(&Tok::Colon) {
            let kind = self.param_kind();
            (kind, Some(self.ty()?))
        } else {
            (ParamKind::Borrow, None)
        };
        Ok(LambdaParam {
            name,
            kind,
            ty,
            span: start.to(self.prev_span()),
        })
    }

    /// `[a, b]`, `[]`, `[k: v]` and `[:]`.
    fn list_or_map(&mut self) -> PResult<ExprKind> {
        let opened = self.pos;
        self.bump();
        let saved = self.enter(false);
        let kind = if self.eat(&Tok::Colon) {
            ExprKind::Map(Vec::new())
        } else if self.at(&Tok::RBracket) {
            ExprKind::List(Vec::new())
        } else {
            let first = self.expr()?;
            if self.eat(&Tok::Colon) {
                let mut entries = vec![(first, self.expr()?)];
                while self.eat(&Tok::Comma) && !self.at(&Tok::RBracket) {
                    let key = self.expr()?;
                    self.expect(&Tok::Colon, "`:` and the entry's value")?;
                    entries.push((key, self.expr()?));
                }
                ExprKind::Map(entries)
            } else {
                let mut items = vec![first];
                while self.eat(&Tok::Comma) && !self.at(&Tok::RBracket) {
                    items.push(self.expr()?);
                }
                ExprKind::List(items)
            }
        };
        if !self.at(&Tok::RBracket) {
            let reported = self.diags.len();
            self.expected_separator(&Tok::RBracket);
            if self.closer(opened).is_none()
                && let Some(diagnostic) = self.diags.get_mut(reported)
            {
                diagnostic
                    .related
                    .push((self.toks[opened].span, "this `[` is never closed".to_string()));
            }
            return Err(());
        }
        self.bump();
        self.leave(saved);
        Ok(kind)
    }

    /// `if cond { ... } else if ... { ... } else { ... }`, at the `if`. The blocks are not
    /// checked here: what they may hold depends on whether the `if` is used for its value,
    /// which the caller knows (`check_if`).
    pub(super) fn if_expr(&mut self) -> PResult<IfExpr> {
        self.expect_kw(Kw::If)?;
        let cond = self.cond()?;
        let then = self.block_unchecked()?;
        let otherwise = if self.eat_kw(Kw::Else) {
            if self.at_kw(Kw::If) {
                Some(ElseBranch::If(Box::new(self.nested(|p| p.if_expr())?)))
            } else if self.at(&Tok::LBrace) {
                Some(ElseBranch::Block(self.block_unchecked()?))
            } else {
                return self.expected("`{` or `if` after `else`");
            }
        } else {
            None
        };
        Ok(IfExpr { cond, then, otherwise })
    }
}
