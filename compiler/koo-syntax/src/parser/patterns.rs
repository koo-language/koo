//! `match` and its patterns (Section 6.6).

use super::*;

impl Parser {
    /// `match scrutinee { arms }`, at the `match`.
    pub(super) fn match_expr(&mut self) -> PResult<Expr> {
        let start = self.expect_kw(Kw::Match)?.span;
        let scrutinee = self.expr_before_block()?;
        self.expect(&Tok::LBrace, "`{`")?;
        let saved = self.enter(true);
        let mut arms = Vec::new();
        loop {
            self.skip_newlines();
            if self.at(&Tok::RBrace) {
                break;
            }
            arms.push(self.match_arm()?);
            if !matches!(self.peek(), Tok::Newline | Tok::RBrace) {
                return self.expected("the end of the line: `match` arms are written one per line");
            }
        }
        self.bump();
        self.leave(saved);
        Ok(Expr {
            kind: ExprKind::Match {
                scrutinee: Box::new(scrutinee),
                arms,
            },
            span: start.to(self.prev_span()),
        })
    }

    fn match_arm(&mut self) -> PResult<MatchArm> {
        let start = self.span();
        let error = self.eat(&Tok::Bang);
        let mut patterns = vec![self.pattern()?];
        while self.eat(&Tok::Pipe) {
            patterns.push(self.pattern()?);
        }
        let guard = if self.eat_kw(Kw::If) {
            Some(self.expr_before_arrow()?)
        } else {
            None
        };
        if self.at(&Tok::Arrow) || self.at(&Tok::Colon) {
            // `1 -> ...`, `1: ...`: the arm of another language.
            let span = self.span();
            let found = self.peek().describe();
            self.diag(
                Diagnostic::new(span, format!("expected `=>`, found {found}"))
                    .with_note("an arm of a `match` is `pattern => result`; `->` writes a lambda and a return type")
                    .with_fix("write `=>`", span, "=>"),
            );
            self.bump();
        } else {
            self.expect(&Tok::FatArrow, "`=>`")?;
        }
        let body = self.body(true)?;
        Ok(MatchArm {
            error,
            patterns,
            guard,
            body,
            span: start.to(self.prev_span()),
        })
    }

    pub(super) fn pattern(&mut self) -> PResult<Pattern> {
        self.nested(|p| p.pattern_unguarded())
    }

    fn pattern_unguarded(&mut self) -> PResult<Pattern> {
        let start = self.span();
        let kind = match self.peek() {
            Tok::Underscore => {
                self.bump();
                PatternKind::Wildcard
            }
            Tok::NoneLit => {
                self.bump();
                PatternKind::None
            }
            Tok::Ident(_) => match self.peek2() {
                // `sql.Error`: a type or variant of another package.
                Tok::Dot => {
                    let package = self.lower_name("package name")?;
                    self.bump();
                    let mut path = vec![package];
                    self.ctor_pattern(&mut path)?
                }
                Tok::Colon => {
                    let name = self.binding_name("variable name")?;
                    self.bump();
                    PatternKind::Bind {
                        name,
                        ty: Some(self.ty_no_result(false)?),
                    }
                }
                _ => PatternKind::Bind {
                    name: self.binding_name("variable name")?,
                    ty: None,
                },
            },
            Tok::UpperIdent(_) => {
                let mut path = Vec::new();
                self.ctor_pattern(&mut path)?
            }
            Tok::LParen => {
                let mut parts = self.delimited(&Tok::LParen, &Tok::RParen, |p| p.pattern())?;
                if parts.len() == 1 {
                    parts.pop().unwrap().kind
                } else {
                    PatternKind::Tuple(parts)
                }
            }
            Tok::Minus
            | Tok::Int(_)
            | Tok::Float(_)
            | Tok::Decimal(_)
            | Tok::Duration(_)
            | Tok::Char(_)
            | Tok::Str(_)
            | Tok::RawStr(_)
            | Tok::True
            | Tok::False => {
                let lo = self.literal_pattern()?;
                if matches!(self.peek(), Tok::DotDot | Tok::DotDotEq) {
                    let inclusive = self.bump().kind == Tok::DotDotEq;
                    let hi = self.literal_pattern()?;
                    PatternKind::Range { lo, hi, inclusive }
                } else {
                    PatternKind::Literal(lo)
                }
            }
            _ => return self.expected("a pattern"),
        };
        Ok(Pattern {
            kind,
            span: start.to(self.prev_span()),
        })
    }

    /// `Name`, `Enum.Variant` and `Name { field, field: pattern, .. }`. `path` holds any
    /// package prefix already read.
    fn ctor_pattern(&mut self, path: &mut Vec<Ident>) -> PResult<PatternKind> {
        path.push(self.upper_name("type or variant name")?);
        while self.at(&Tok::Dot) && matches!(self.peek2(), Tok::UpperIdent(_)) {
            self.bump();
            path.push(Self::ident_from(self.bump()));
        }
        let path = std::mem::take(path);
        if !self.at(&Tok::LBrace) {
            return Ok(PatternKind::Ctor {
                path,
                fields: None,
                rest: false,
            });
        }
        self.bump();
        let saved = self.enter(false);
        let mut fields = Vec::new();
        let mut rest = false;
        while !self.at(&Tok::RBrace) {
            if self.eat(&Tok::DotDot) {
                rest = true;
                if !self.at(&Tok::RBrace) {
                    return self.expected("`}`: `..` comes last in a pattern");
                }
                break;
            }
            let shorthand_ok = matches!(self.peek(), Tok::Ident(_));
            let name = self.member_name("a field name")?;
            let pattern = if self.eat(&Tok::Colon) {
                Some(self.pattern()?)
            } else {
                if !shorthand_ok {
                    self.report(
                        name.span,
                        format!("the field `{}` needs a pattern: `{}: ...`", name.name, name.name),
                    );
                }
                None
            };
            fields.push(FieldPattern { name, pattern });
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(&Tok::RBrace, "`,` or `}`")?;
        self.leave(saved);
        Ok(PatternKind::Ctor {
            path,
            fields: Some(fields),
            rest,
        })
    }

    /// A literal in a pattern: a number (possibly negative), a character, a string without
    /// interpolation, `true` or `false`.
    fn literal_pattern(&mut self) -> PResult<Expr> {
        let start = self.span();
        if self.eat(&Tok::Minus) {
            if !matches!(
                self.peek(),
                Tok::Int(_) | Tok::Float(_) | Tok::Decimal(_) | Tok::Duration(_)
            ) {
                return self.expected("a number");
            }
            let number = self.literal()?;
            return Ok(Expr {
                kind: ExprKind::Unary {
                    op: UnaryOp::Neg,
                    expr: Box::new(number),
                },
                span: start.to(self.prev_span()),
            });
        }
        if !matches!(
            self.peek(),
            Tok::Int(_)
                | Tok::Float(_)
                | Tok::Decimal(_)
                | Tok::Duration(_)
                | Tok::Char(_)
                | Tok::Str(_)
                | Tok::RawStr(_)
                | Tok::True
                | Tok::False
        ) {
            return self.expected("a literal: the bounds of a range pattern are literals");
        }
        let literal = self.literal()?;
        if let ExprKind::Str(parts) = &literal.kind
            && parts.iter().any(|p| matches!(p, StrPart::Interp { .. }))
        {
            self.report(literal.span, "a string in a pattern cannot contain an interpolation");
        }
        Ok(literal)
    }
}
