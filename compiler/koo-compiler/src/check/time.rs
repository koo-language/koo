//! `Duration` and `Time` (Section 10.6).
//!
//! The two types are structs of the prelude, each a count of nanoseconds. What the
//! compiler adds is the literal, `250ms`, and the operators: `+`, `-`, `*` and `/` on
//! them are calls of the methods the prelude gives them for it.

use koo_syntax::Span;

use super::{Checker, TypeName};
use crate::hir::*;

/// The units of a duration literal, and the nanoseconds in each.
const UNITS: [(&str, u128); 6] = [
    ("h", 3_600_000_000_000),
    ("min", 60_000_000_000),
    ("s", 1_000_000_000),
    ("ms", 1_000_000),
    ("us", 1_000),
    ("ns", 1),
];

impl Checker<'_> {
    fn prelude_struct(&self, name: &str) -> Option<Ty> {
        match self.type_names.get(name) {
            Some(TypeName::Struct(id)) if *id < self.prelude_types.0 => Some(Ty::Struct(*id)),
            _ => None,
        }
    }

    pub(super) fn duration_ty(&self) -> Option<Ty> {
        self.prelude_struct("Duration")
    }

    pub(super) fn time_ty(&self) -> Option<Ty> {
        self.prelude_struct("Time")
    }

    /// Whether a type is one of the two.
    pub(super) fn is_time_type(&self, ty: &Ty) -> bool {
        matches!(ty, Ty::Struct(_)) && (self.duration_ty().as_ref() == Some(ty) || self.time_ty().as_ref() == Some(ty))
    }

    /// Whether a type is a struct of the prelude whose fields are its own: a duration, a
    /// time, a query, a row, a pool. A program neither reads those fields nor builds
    /// the struct from them.
    pub(super) fn is_opaque(&self, ty: &Ty) -> bool {
        match ty {
            Ty::Struct(id) if *id < self.prelude_types.0 => self.program.structs[*id]
                .fields
                .iter()
                .any(|field| field.name.starts_with('_')),
            _ => false,
        }
    }

    /// A string literal where a query is expected (Section 3.3): the text between the
    /// values, and the values, each as a parameter. Nothing of a value becomes text of
    /// the statement.
    pub(super) fn sql_template(&mut self, parts: &[koo_syntax::ast::StrPart], span: Span) -> Option<Expr> {
        use koo_syntax::ast;
        let Some(Ty::Struct(query)) = self.prelude_struct("SqlQuery") else {
            return None;
        };
        let mut texts = vec![String::new()];
        let mut params = Vec::new();
        for part in parts {
            match part {
                ast::StrPart::Lit(text) => texts.last_mut().unwrap().push_str(text),
                ast::StrPart::Interp { expr, format } => {
                    if format.is_some() {
                        self.error_help(
                            expr.span,
                            "a value in a query cannot have a format",
                            "the value is sent to the database as it is, apart from the statement, and not as text (Section 3.3)",
                        );
                    }
                    let value = self.expr(expr, None);
                    if value.ty.is_unknown() {
                        continue;
                    }
                    let inner = match &value.ty {
                        Ty::Optional(inner) => inner.as_ref(),
                        other => other,
                    };
                    let known = matches!(inner, Ty::Int | Ty::Float | Ty::Bool | Ty::Str)
                        || self.time_ty().as_ref() == Some(inner);
                    if !known {
                        let found = self.ty_name(&value.ty);
                        self.error_help(
                            expr.span,
                            format!("a `{found}` cannot be a parameter of a query"),
                            "a parameter is an `int`, a `float`, a `bool`, a `string` or a `Time`, or an optional of one, which is NULL when it is `none` (Section 10.3)",
                        );
                        continue;
                    }
                    let name = ast::Ident {
                        name: "_sql".to_string(),
                        span: expr.span,
                    };
                    params.push(self.method_call(value, &name, &[], expr.span, None));
                    texts.push(String::new());
                }
            }
        }
        let parts: Vec<Expr> = texts
            .into_iter()
            .map(|text| self.mk(ExprKind::Str(text), Ty::Str, span))
            .collect();
        let parts = self.mk(ExprKind::ListLit(parts), Ty::list(Ty::Str), span);
        let params = self.mk(ExprKind::ListLit(params), Ty::list(Ty::optional(Ty::Str)), span);
        Some(self.mk(ExprKind::StructLit(query, vec![parts, params]), Ty::Struct(query), span))
    }

    /// `1h30min`: a `Duration` of so many nanoseconds. The lexer has seen to it that the
    /// text is digits and units, the units from largest to smallest.
    pub(super) fn duration_literal(&mut self, text: &str, span: Span) -> Expr {
        let Some(Ty::Struct(id)) = self.duration_ty() else {
            self.unsupported(span, "`Duration` is");
            return self.unknown(span);
        };
        let mut nanos: u128 = 0;
        let mut rest = text;
        while !rest.is_empty() {
            let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit() || c == '_').len();
            let letters = rest[digits..].len()
                - rest[digits..]
                    .trim_start_matches(|c: char| c.is_ascii_alphabetic())
                    .len();
            let count = rest[..digits].replace('_', "").parse::<u128>().unwrap_or(u128::MAX);
            let size = UNITS
                .iter()
                .find(|(unit, _)| *unit == &rest[digits..digits + letters])
                .map_or(0, |(_, size)| *size);
            nanos = nanos.saturating_add(count.saturating_mul(size));
            rest = &rest[digits + letters..];
            if digits + letters == 0 {
                break;
            }
        }
        if nanos > i64::MAX as u128 {
            self.error_help(
                span,
                "this duration is longer than a `Duration` can be",
                "a `Duration` is a count of nanoseconds in 64 bits, which is a little over 292 years (Section 10.6)",
            );
            return self.unknown(span);
        }
        let value = self.mk(ExprKind::Int(nanos as i64), Ty::Int, span);
        self.mk(ExprKind::StructLit(id, vec![value]), Ty::Struct(id), span)
    }

    /// Calls a method of the prelude's on a duration or a time.
    fn time_method(&mut self, name: &str, receiver: Expr, argument: Option<Expr>, span: Span) -> Option<Expr> {
        let id = self.method_of(&receiver.ty, name)?;
        let ret = self.sigs[id].ret.clone();
        let mut args = vec![Arg {
            kind: ParamKind::Borrow,
            expr: receiver,
        }];
        args.extend(argument.map(|expr| Arg {
            kind: ParamKind::Borrow,
            expr,
        }));
        Some(Expr {
            kind: ExprKind::Call { func: id, args },
            ty: ret,
            span,
        })
    }

    /// `a op b` where a side is a duration or a time. Durations add and subtract, and
    /// are multiplied and divided by an `int`; a time and a duration add to a time; two
    /// times subtract to a duration. There is nothing for any other pair.
    pub(super) fn time_arithmetic(&mut self, op: BinOp, a: &Expr, b: &Expr, span: Span) -> Option<Expr> {
        let (duration, time) = (self.duration_ty()?, self.time_ty()?);
        let is = |expr: &Expr, ty: &Ty| expr.ty == *ty;
        let (method, swapped) = match op {
            BinOp::Add if is(a, &duration) && is(b, &duration) => ("_add", false),
            BinOp::Sub if is(a, &duration) && is(b, &duration) => ("_sub", false),
            BinOp::Mul if is(a, &duration) && b.ty == Ty::Int => ("_mul", false),
            BinOp::Mul if a.ty == Ty::Int && is(b, &duration) => ("_mul", true),
            BinOp::Div if is(a, &duration) && b.ty == Ty::Int => ("_div", false),
            BinOp::Add if is(a, &time) && is(b, &duration) => ("_add", false),
            BinOp::Add if is(a, &duration) && is(b, &time) => ("_add", true),
            BinOp::Sub if is(a, &time) && is(b, &duration) => ("_sub", false),
            BinOp::Sub if is(a, &time) && is(b, &time) => ("_since", false),
            _ => return None,
        };
        let (receiver, argument) = if swapped { (b, a) } else { (a, b) };
        self.time_method(method, receiver.clone(), Some(argument.clone()), span)
    }

    /// `-d` for a duration.
    pub(super) fn negated_duration(&mut self, operand: &Expr, span: Span) -> Option<Expr> {
        if self.duration_ty().as_ref() != Some(&operand.ty) {
            return None;
        }
        self.time_method("_neg", operand.clone(), None, span)
    }
}
