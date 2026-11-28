//! Checking expressions.

use std::collections::HashMap;

use koo_syntax::Span;
use koo_syntax::ast;

use super::{Checker, TypeName, diverges};
use crate::hir::*;

/// The type inside any optional and result layers: what a literal written for `T?` or
/// `T ! E` is a literal of.
pub(super) fn peel(ty: &Ty) -> &Ty {
    match ty {
        Ty::Optional(inner) | Ty::Result(inner, _) => peel(inner),
        other => other,
    }
}

pub(super) fn block_ty(block: &Block) -> Ty {
    if diverges(block) {
        Ty::Never
    } else {
        block.value.as_ref().map(|v| v.ty.clone()).unwrap_or(Ty::Unit)
    }
}

/// Reads the format of an interpolation: `[<>^][0][width][.precision]`.
fn parse_format(text: &str, ty: &Ty, decimal: bool) -> Result<Format, &'static str> {
    let mut format = Format::default();
    let mut rest = text;
    for (symbol, align) in [('<', Align::Left), ('>', Align::Right), ('^', Align::Center)] {
        if let Some(after) = rest.strip_prefix(symbol) {
            format.align = align;
            rest = after;
            break;
        }
    }
    let number = matches!(ty, Ty::Int | Ty::Float | Ty::Byte) || decimal;
    format.number = decimal;
    if rest.len() > 1 && rest.starts_with('0') && rest.as_bytes()[1].is_ascii_digit() {
        if !number {
            return Err("padding with zeros is for numbers");
        }
        format.zero = true;
        rest = &rest[1..];
    }
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 {
        format.width = match rest[..digits].parse::<u32>() {
            Ok(width) if width <= 1000 => width,
            _ => return Err("the width is at most 1000"),
        };
        rest = &rest[digits..];
    }
    if let Some(precision) = rest.strip_prefix('.') {
        if *ty != Ty::Float && !decimal {
            return Err("a precision is for a `float` or a `Decimal`");
        }
        if precision.is_empty() || !precision.bytes().all(|b| b.is_ascii_digit()) {
            return Err("the precision is a number of decimals");
        }
        format.precision = match precision.parse::<u32>() {
            Ok(precision) if precision <= 100 => Some(precision),
            _ => return Err("the precision is at most 100"),
        };
        rest = "";
    }
    if !rest.is_empty() || text.is_empty() {
        return Err("it is not a format");
    }
    Ok(format)
}

fn parse_int(text: &str) -> Option<u64> {
    let digits: String = text.chars().filter(|c| *c != '_').collect();
    if let Some(hex) = digits.strip_prefix("0x") {
        u64::from_str_radix(hex, 16).ok()
    } else if let Some(bin) = digits.strip_prefix("0b") {
        u64::from_str_radix(bin, 2).ok()
    } else {
        digits.parse().ok()
    }
}

impl<'a> Checker<'a> {
    pub(super) fn mk(&self, kind: ExprKind, ty: Ty, span: Span) -> Expr {
        Expr { kind, ty, span }
    }

    /// Checks an expression. `expected` is a hint that lets literals take the type the
    /// context wants; the caller still calls `coerce`.
    pub(crate) fn expr(&mut self, expr: &ast::Expr, expected: Option<&Ty>) -> Expr {
        let span = expr.span;
        match &expr.kind {
            ast::ExprKind::Int(text) => self.int_literal(text, false, expected, span),
            ast::ExprKind::Float(text) => {
                let digits: String = text.chars().filter(|c| *c != '_').collect();
                match digits.parse::<f64>() {
                    Ok(value) => self.mk(ExprKind::Float(value), Ty::Float, span),
                    Err(_) => {
                        self.error(span, "this float literal cannot be read");
                        self.unknown(span)
                    }
                }
            }
            ast::ExprKind::Decimal(text) => self.decimal_literal(text, false, span),
            ast::ExprKind::Duration(text) => self.duration_literal(text, span),
            ast::ExprKind::Char(_) => {
                self.unsupported(span, "`char` is");
                self.unknown(span)
            }
            ast::ExprKind::RawStr(text) => self.mk(ExprKind::Str(text.clone()), Ty::Str, span),
            ast::ExprKind::Str(parts) => {
                // Where a query or a line of the log is expected, the literal is one
                // (`check/time.rs`).
                let template = match expected {
                    Some(ty @ Ty::Struct(id)) if self.is_opaque(ty) => match self.program.structs[*id].name.as_str() {
                        "Sql__Query" => self.sql_template(parts, span),
                        "Log__Message" => self.log_template(parts, span),
                        _ => None,
                    },
                    _ => None,
                };
                match template {
                    Some(made) => made,
                    None => self.string(parts, span),
                }
            }
            ast::ExprKind::Bool(value) => self.mk(ExprKind::Bool(*value), Ty::Bool, span),
            ast::ExprKind::None => {
                // `none` is the outermost absence of the expected type (Section 4.8).
                let mut hint = expected;
                while let Some(Ty::Result(ok, _)) = hint {
                    hint = Some(ok);
                }
                match hint {
                    Some(ty @ Ty::Optional(_)) => self.mk(ExprKind::NoneLit, ty.clone(), span),
                    Some(Ty::Unknown) => self.unknown(span),
                    Some(other) => {
                        let want = self.ty_name(other);
                        self.error_help(
                            span,
                            format!("expected `{want}`, found `none`"),
                            format!("only an optional can be absent: `{want}?` (Section 4.8)"),
                        );
                        self.unknown(span)
                    }
                    None => {
                        self.error_help(
                            span,
                            "the type of `none` is not known here",
                            "give the binding a type: `x: int? = none`",
                        );
                        self.unknown(span)
                    }
                }
            }
            ast::ExprKind::Name(name) => self.name(name, expected),
            ast::ExprKind::TypeApp { .. } => {
                self.error_help(
                    span,
                    "a type is not a value",
                    "call one of its functions: `List<int>.new()`",
                );
                self.unknown(span)
            }
            ast::ExprKind::Field { base, name } => self.field(base, name, span, expected),
            ast::ExprKind::Call { callee, args } => self.call(callee, args, span, expected),
            ast::ExprKind::Index { base, index } => {
                let base = self.expr(base, None);
                if let Some((key_ty, _)) = self.map_types(&base.ty) {
                    // `m[key]` is the value for the key, if there is one (Section 4.5).
                    let key = self.expr_as(index, &key_ty);
                    return self.prelude_call("get", vec![base, key], span);
                }
                if matches!(
                    index.kind,
                    ast::ExprKind::Binary {
                        op: ast::BinaryOp::Range | ast::BinaryOp::RangeInclusive,
                        ..
                    }
                ) {
                    self.unsupported(index.span, "slices are");
                    return self.unknown(span);
                }
                let index = self.expr(index, Some(&Ty::Int));
                let index = self.coerce(index, &Ty::Int);
                match base.ty.clone() {
                    Ty::List(element) => self.mk(ExprKind::Index(Box::new(base), Box::new(index)), *element, span),
                    Ty::Str => {
                        self.error_help(
                            span,
                            "a string cannot be indexed",
                            "use `s.chars()`, `s.bytes()` or `s.slice(from, to)` (Section 4.4)",
                        );
                        self.unknown(span)
                    }
                    Ty::Unknown => self.unknown(span),
                    other => {
                        let found = self.ty_name(&other);
                        self.error(span, format!("`{found}` cannot be indexed"));
                        self.unknown(span)
                    }
                }
            }
            ast::ExprKind::Try(inner) => self.try_expr(inner, span, expected),
            ast::ExprKind::Unary { op, expr: inner } => match op {
                ast::UnaryOp::Neg => {
                    if let ast::ExprKind::Int(text) = &inner.kind {
                        return self.int_literal(text, true, expected, span);
                    }
                    // (A constant, like the literal it negates.)
                    if let ast::ExprKind::Decimal(text) = &inner.kind {
                        return self.decimal_literal(text, true, span);
                    }
                    let operand = self.expr(inner, expected);
                    match operand.ty {
                        Ty::Int | Ty::Float => {
                            let ty = operand.ty.clone();
                            self.mk(ExprKind::Unary(UnOp::Neg, Box::new(operand)), ty, span)
                        }
                        Ty::Unknown => self.unknown(span),
                        _ => match self.negated_duration(&operand, span) {
                            Some(negated) => negated,
                            None => {
                                let found = self.ty_name(&operand.ty);
                                self.error(span, format!("`-` is not defined for `{found}`"));
                                self.unknown(span)
                            }
                        },
                    }
                }
                ast::UnaryOp::Not => {
                    let operand = self.expr(inner, Some(&Ty::Bool));
                    let reported = self.diags.len();
                    let operand = self.coerce(operand, &Ty::Bool);
                    if self.diags.len() > reported {
                        self.help("`not` takes a `bool`: compare the value first, as in `n == 0` or `name == \"\"`");
                    }
                    self.mk(ExprKind::Unary(UnOp::Not, Box::new(operand)), Ty::Bool, span)
                }
            },
            ast::ExprKind::Binary { op, lhs, rhs } => self.binary(*op, lhs, rhs, span, expected),
            ast::ExprKind::Is { .. } => {
                self.unsupported(span, "`is` tests are");
                self.unknown(span)
            }
            ast::ExprKind::Else {
                expr: inner,
                binding,
                alt,
            } => self.else_expr(inner, binding.as_ref(), alt, span, expected),
            ast::ExprKind::List(items) => self.list(items, span, expected),
            ast::ExprKind::Map(entries) => self.map_literal(entries, span, expected),
            ast::ExprKind::Tuple(items) if items.is_empty() => self.mk(ExprKind::Unit, Ty::Unit, span),
            ast::ExprKind::Tuple(items) => {
                let hints: Vec<Option<Ty>> = match expected.map(peel) {
                    Some(Ty::Tuple(types)) if types.len() == items.len() => types.iter().cloned().map(Some).collect(),
                    _ => vec![None; items.len()],
                };
                let mut values = Vec::new();
                for (item, hint) in items.iter().zip(&hints) {
                    let value = self.expr(item, hint.as_ref());
                    values.push(match hint {
                        Some(hint) if self.can_coerce(&value.ty, hint) => self.coerce(value, hint),
                        _ => value,
                    });
                }
                if values.iter().any(|v| v.ty.is_unknown()) {
                    return self.unknown(span);
                }
                if let Some(never) = values.iter().position(|v| v.ty == Ty::Never) {
                    // No tuple is made, but the elements before the one that never
                    // returns are evaluated all the same, in order (Section 5.4).
                    values.truncate(never + 1);
                    let always = self.mk(ExprKind::Bool(true), Ty::Bool, span);
                    return self.mk(
                        ExprKind::If {
                            cond: Box::new(Cond::Expr(always)),
                            then: Block {
                                stmts: values.into_iter().map(Stmt::Expr).collect(),
                                value: None,
                            },
                            otherwise: None,
                        },
                        Ty::Never,
                        span,
                    );
                }
                if values.iter().any(|v| v.ty == Ty::Unit) {
                    self.error(span, "a tuple cannot hold `()`");
                    return self.unknown(span);
                }
                let ty = Ty::Tuple(values.iter().map(|v| v.ty.clone()).collect());
                self.mk(ExprKind::TupleLit(values), ty, span)
            }
            ast::ExprKind::Paren(inner) => self.expr(inner, expected),
            ast::ExprKind::StructLit { ty, fields } => self.struct_literal(ty, fields, span, expected),
            ast::ExprKind::Lambda { params, body } => self.lambda(params, body, expected, span),
            ast::ExprKind::If(if_expr) => self.if_expr(if_expr, span, true, expected),
            ast::ExprKind::Match { scrutinee, arms } => self.match_expr(scrutinee, arms, span, true, expected),
        }
    }

    /// Checks an expression and converts it to the type the context requires.
    pub(crate) fn expr_as(&mut self, expr: &ast::Expr, expected: &Ty) -> Expr {
        let checked = self.expr(expr, Some(expected));
        self.coerce(checked, expected)
    }

    fn int_literal(&mut self, text: &str, negative: bool, expected: Option<&Ty>, span: Span) -> Expr {
        let Some(magnitude) = parse_int(text) else {
            self.error(span, "this integer literal does not fit in 64 bits");
            return self.unknown(span);
        };
        let value = if negative {
            if magnitude > 1 << 63 {
                None
            } else {
                Some((magnitude as i64).wrapping_neg())
            }
        } else {
            i64::try_from(magnitude).ok()
        };
        let Some(value) = value else {
            self.error_help(
                span,
                "this integer literal does not fit in `int`",
                "`int` is 64 bits, from -9223372036854775808 to 9223372036854775807 (Section 4.2)",
            );
            return self.unknown(span);
        };
        if expected.map(peel) == Some(&Ty::Byte) {
            if !(0..=255).contains(&value) {
                self.error(span, "a `byte` is from 0 to 255");
            }
            return self.mk(ExprKind::Int(value), Ty::Byte, span);
        }
        self.mk(ExprKind::Int(value), Ty::Int, span)
    }

    pub(super) fn string(&mut self, parts: &[ast::StrPart], span: Span) -> Expr {
        if parts.iter().all(|p| matches!(p, ast::StrPart::Lit(_))) {
            let text: String = parts
                .iter()
                .map(|p| match p {
                    ast::StrPart::Lit(text) => text.as_str(),
                    ast::StrPart::Interp { .. } => "",
                })
                .collect();
            return self.mk(ExprKind::Str(text), Ty::Str, span);
        }
        let mut out = Vec::new();
        for part in parts {
            match part {
                ast::StrPart::Lit(text) => out.push(InterpPart::Lit(text.clone())),
                ast::StrPart::Interp { expr, format } => {
                    let mut value = self.expr(expr, None);
                    self.require_printable(&value);
                    let decimal = self.decimal_ty().as_ref() == Some(&value.ty);
                    let mut format = match format.as_deref() {
                        None => Format {
                            number: decimal,
                            ..Format::default()
                        },
                        Some(text) => match parse_format(text, &value.ty, decimal) {
                            Ok(format) => format,
                            Err(why) => {
                                if !value.ty.is_unknown() {
                                    self.error_help(
                                        expr.span,
                                        format!("the format `{text}` cannot be used here: {why}"),
                                        "a format is an alignment (`<`, `>` or `^`), a width, and for a float or a `Decimal` a precision: `{name:>10}`, `{id:08}`, `{ratio:.2}`, `{price:8.2}`",
                                    );
                                }
                                Format::default()
                            }
                        },
                    };
                    // A decimal with a precision is the text of the decimal with so many
                    // digits after its point, and what is left of the format is the text's.
                    if decimal
                        && let Some(places) = format.precision.take()
                        && let Some(fixed) = self.decimal_fixed(value.clone(), places, expr.span)
                    {
                        value = fixed;
                    }
                    out.push(InterpPart::Value(value, format));
                }
            }
        }
        self.mk(ExprKind::Interp(out), Ty::Str, span)
    }

    /// Reports a value that has no text form: `()`, or something that never arrives.
    pub(super) fn require_printable(&mut self, value: &Expr) {
        if matches!(value.ty, Ty::Unit | Ty::Never) {
            self.error(value.span, "this expression has no value to print");
            self.label("this gives nothing back");
        }
        if matches!(value.ty, Ty::Result(..)) {
            self.unhandled_result(value, value.span);
        }
    }

    fn name(&mut self, name: &ast::Ident, expected: Option<&Ty>) -> Expr {
        let span = name.span;
        if !name.is_upper() {
            if let Some(local) = self.resolve_local(&name.name) {
                let ty = self.locals[local].ty.clone();
                self.used[local] = true;
                if self.lent_in_closure.contains(&local) {
                    self.lent_fn_ok = false;
                    self.error_help(
                        span,
                        format!("this closure uses `{}`, a function that was only lent", name.name),
                        format!(
                            "the closure may be kept after the call that lent it has ended; to keep the function too, declare the parameter as `{}: take fn(...)` (Section 5.3)",
                            name.name
                        ),
                    );
                } else if self.borrowed_fns.contains(&local) && !std::mem::take(&mut self.lent_fn_ok) {
                    self.error_help(
                        span,
                        format!("`{}` is a function that was only lent to this one", name.name),
                        format!(
                            "it may be called, or passed on to another function parameter; to keep it, declare the parameter as `{}: take fn(...)` (Section 5.3)",
                            name.name
                        ),
                    );
                }
                return self.mk(ExprKind::Local(local), ty, span);
            }
            let plain = self.plain(&name.name);
            if let Some(constant) = self.consts.get(&plain) {
                let mut constant = constant.clone();
                constant.span = span;
                return constant;
            }
            if let Some(&id) = self
                .free_fns
                .get(&plain)
                .filter(|_| !name.name.contains("__") || self.body_in_prelude)
            {
                // A named function is a value of its function type (Section 4.11).
                let sig = &self.sigs[id];
                // Where a function that may fail is wanted, one that cannot will do, and
                // so will one that fails with fewer errors: `home` stands for
                // `(request) -> home(request)`, whose result is widened.
                if let Some(Ty::Fn(want)) = expected.map(peel)
                    && want.ret != sig.ret
                    && want.params.len() == sig.params.len()
                    && want
                        .params
                        .iter()
                        .zip(&sig.params)
                        .all(|((kind, ty), param)| *kind == param.kind && *ty == param.ty)
                    && sig.receiver.is_none()
                    && self.can_coerce(&sig.ret, &want.ret)
                {
                    let ident = |text: String| ast::Ident { name: text, span };
                    let kinds: Vec<ast::ParamKind> = want
                        .params
                        .iter()
                        .map(|(kind, _)| match kind {
                            ParamKind::Borrow => ast::ParamKind::Borrow,
                            ParamKind::Var => ast::ParamKind::Var,
                            ParamKind::Take => ast::ParamKind::Take,
                        })
                        .collect();
                    let params: Vec<ast::LambdaParam> = kinds
                        .iter()
                        .enumerate()
                        .map(|(i, kind)| ast::LambdaParam {
                            name: Some(ident(format!("arg__{i}"))),
                            kind: *kind,
                            ty: None,
                            span,
                        })
                        .collect();
                    let args: Vec<ast::Arg> = kinds
                        .iter()
                        .enumerate()
                        .map(|(i, kind)| ast::Arg {
                            name: None,
                            var: *kind == ast::ParamKind::Var,
                            var_at: span.start,
                            value: ast::Expr {
                                kind: ast::ExprKind::Name(ident(format!("arg__{i}"))),
                                span,
                            },
                        })
                        .collect();
                    let call = ast::Expr {
                        kind: ast::ExprKind::Call {
                            callee: Box::new(ast::Expr {
                                kind: ast::ExprKind::Name(name.clone()),
                                span,
                            }),
                            args,
                        },
                        span,
                    };
                    let body = ast::Body::Stmt(Box::new(ast::Stmt {
                        kind: ast::StmtKind::Expr(call),
                        span,
                    }));
                    return self.lambda(&params, &body, expected, span);
                }
                let ty = Ty::Fn(Box::new(FnTy {
                    params: sig.params.iter().map(|p| (p.kind, p.ty.clone())).collect(),
                    ret: sig.ret.clone(),
                }));
                return self.mk(ExprKind::FnRef(id), ty, span);
            }
            if self.generic_fns.contains_key(&plain) || matches!(name.name.as_str(), "print" | "panic") {
                self.unsupported(span, "generic and built-in functions used as values are");
            } else {
                let (known, habit) = (self.names_in_scope(), super::suggest::name_habit(&name.name));
                self.error_near(span, format!("unknown name `{}`", name.name), &name.name, known, habit);
            }
            return self.unknown(span);
        }
        // A bare capitalized name is looked up among the variants of the expected enum
        // first, then among types (Section 3.9).
        if let Some(Ty::Enum(id)) = expected.map(peel) {
            let variant = self.program.enums[*id]
                .variants
                .iter()
                .position(|v| v.name == super::bare(&name.name));
            if let Some(variant) = variant {
                self.kept_variant(*id, span);
                return self.variant_value(*id, variant, span);
            }
        }
        match self.type_names.get(&name.name).cloned() {
            Some(TypeName::Error(id)) if self.program.errors[id].fields.is_empty() => {
                self.mk(ExprKind::ErrorLit(id, Vec::new()), Ty::Error(id), span)
            }
            Some(TypeName::Struct(id)) if self.program.structs[id].fields.is_empty() => {
                self.mk(ExprKind::StructLit(id, Vec::new()), Ty::Struct(id), span)
            }
            Some(TypeName::Error(_) | TypeName::Struct(_)) => {
                self.error_help(
                    span,
                    format!("`{}` has fields", name.name),
                    format!("write `{} {{ ... }}` to make a value", name.name),
                );
                self.unknown(span)
            }
            Some(_) => {
                self.error(span, format!("`{}` is a type, not a value", name.name));
                self.unknown(span)
            }
            None => {
                // Perhaps a variant of some enum, written where its type is not expected.
                // Perhaps a variant of some enum, written where its type is not expected:
                // of an enum of this file's own package first, then of one it may name,
                // and never of one a package keeps to itself.
                let here = self.package_here().to_string();
                let mut owners: Vec<(u8, String)> = (0..self.program.enums.len())
                    .filter(|&id| {
                        self.program.enums[id]
                            .variants
                            .iter()
                            .any(|v| v.name == super::bare(&name.name))
                    })
                    .filter_map(|id| {
                        let full = &self.program.enums[id].name;
                        let base = full.split('<').next().unwrap_or(full);
                        let package = self.package_of_type(&Ty::Enum(id));
                        if package
                            .as_ref()
                            .is_some_and(|package| self.kept_to_its_package(base, package))
                        {
                            return None;
                        }
                        let library = id < self.prelude_enums;
                        let own = match &package {
                            Some(package) => *package == here,
                            None => here.is_empty() && !library,
                        };
                        let rank = if own {
                            0
                        } else if library {
                            2
                        } else {
                            1
                        };
                        Some((rank, self.ty_name(&Ty::Enum(id))))
                    })
                    .collect();
                owners.sort_by_key(|(rank, _)| *rank);
                let owners: Vec<String> = owners.into_iter().map(|(_, owner)| owner).collect();
                match owners.first() {
                    Some(owner) => self.error_help(
                        span,
                        format!("`{}` is not known here", name.name),
                        format!(
                            "a variant is written bare only where its enum is expected; write `{owner}.{}` (Section 4.7)",
                            name.name
                        ),
                    ),
                    None => {
                        let (known, habit) = (self.names_in_scope(), super::suggest::name_habit(&name.name));
                        self.error_near(span, format!("unknown name `{}`", name.name), &name.name, known, habit);
                    }
                }
                self.unknown(span)
            }
        }
    }

    fn variant_value(&mut self, id: EnumId, variant: usize, span: Span) -> Expr {
        let def = &self.program.enums[id].variants[variant];
        if !def.fields.is_empty() {
            let name = def.name.clone();
            self.error_help(
                span,
                format!("the variant `{name}` has fields"),
                format!("write `{name} {{ ... }}`"),
            );
            return self.unknown(span);
        }
        self.mk(ExprKind::EnumLit(id, variant, Vec::new()), Ty::Enum(id), span)
    }

    fn field(&mut self, base: &ast::Expr, name: &ast::Ident, span: Span, expected: Option<&Ty>) -> Expr {
        // `orders.limit`: a constant of a package the file imports, or a function of
        // it as a value.
        if let ast::ExprKind::Name(module) = &base.kind
            && !self.body_in_prelude
            && !name.is_upper()
            && self.resolve_local(&module.name).is_none()
            && let Some(import) = self.modules.get(&(self.file, module.name.clone()))
        {
            let own = import.2.clone();
            let full = format!("{own}__{}", name.name);
            let known = self.consts.contains_key(&full) || self.free_fns.contains_key(&full);
            if known {
                self.modules.get_mut(&(self.file, module.name.clone())).unwrap().1 = true;
                if self.kept_to_its_package(&full, &own) {
                    self.error_help(
                        name.span,
                        format!("`{}.{}` is not offered by its package", module.name, name.name),
                        "what a package offers to those who import it is declared `pub` (Section 4.6)",
                    );
                    return self.unknown(span);
                }
                let written = ast::Expr {
                    kind: ast::ExprKind::Name(ast::Ident {
                        name: full,
                        span: name.span,
                    }),
                    span,
                };
                // The name is the compiler's own here, and is looked for as it is.
                let prelude = std::mem::replace(&mut self.body_in_prelude, true);
                let value = self.expr(&written, expected);
                self.body_in_prelude = prelude;
                return value;
            }
        }
        // `sql.Isolation.Serializable`: a variant of an enum of a standard package.
        if let ast::ExprKind::Field {
            base: package,
            name: type_name,
        } = &base.kind
            && let ast::ExprKind::Name(package) = &package.kind
            && type_name.is_upper()
            && self.resolve_local(&package.name).is_none()
            && let Some(full) = self.package_type(&[package.clone(), type_name.clone()])
        {
            let of = ast::Expr {
                kind: ast::ExprKind::Name(ast::Ident {
                    name: full,
                    span: type_name.span,
                }),
                span: base.span,
            };
            return self.field(&of, name, span, expected);
        }
        // `Tree<int>.Leaf`: a variant of a generic enum, written with the type in full.
        if let ast::ExprKind::TypeApp { base: generic, .. } = &base.kind
            && let ast::ExprKind::Name(type_name) = &generic.kind
            && self.generic_types.contains_key(&type_name.name)
            && let Some(Ty::Enum(id)) = self.literal_type(base, expected)
        {
            return match self.program.enums[id].variants.iter().position(|v| v.name == name.name) {
                Some(variant) => self.variant_value(id, variant, span),
                None => {
                    let known = self.program.enums[id].variants.iter().map(|v| v.name.clone()).collect();
                    self.error_near(
                        name.span,
                        format!("`{}` has no variant `{}`", self.ty_name(&Ty::Enum(id)), name.name),
                        &name.name,
                        known,
                        None,
                    );
                    self.unknown(span)
                }
            };
        }
        // `Payment.Cash`: a variant written with its enum.
        if let ast::ExprKind::Name(type_name) = &base.kind
            && type_name.is_upper()
            && self.generic_types.contains_key(&type_name.name)
        {
            return match self.literal_type(base, expected) {
                Some(Ty::Enum(id)) => match self.program.enums[id].variants.iter().position(|v| v.name == name.name) {
                    Some(variant) => self.variant_value(id, variant, span),
                    None => {
                        let known = self.program.enums[id].variants.iter().map(|v| v.name.clone()).collect();
                        self.error_near(
                            name.span,
                            format!("`{}` has no variant `{}`", self.ty_name(&Ty::Enum(id)), name.name),
                            &name.name,
                            known,
                            None,
                        );
                        self.unknown(span)
                    }
                },
                _ => self.unknown(span),
            };
        }
        if let ast::ExprKind::Name(type_name) = &base.kind
            && type_name.is_upper()
        {
            return match self.type_names.get(&type_name.name).cloned() {
                // By its own name, or by another name for it.
                Some(TypeName::Enum(id) | TypeName::Alias(Ty::Enum(id))) => {
                    let variant = self.program.enums[id].variants.iter().position(|v| v.name == name.name);
                    match variant {
                        Some(variant) => self.variant_value(id, variant, span),
                        None => {
                            let known = self.program.enums[id].variants.iter().map(|v| v.name.clone()).collect();
                            self.error_near(
                                name.span,
                                format!("`{}` has no variant `{}`", self.ty_name(&Ty::Enum(id)), name.name),
                                &name.name,
                                known,
                                None,
                            );
                            self.unknown(span)
                        }
                    }
                }
                Some(_) => {
                    self.error_help(
                        span,
                        format!("`{}.{}` is not a value", type_name.name, name.name),
                        "a type-level function is called: add `(...)`",
                    );
                    self.unknown(span)
                }
                None => {
                    self.error(type_name.span, format!("unknown type `{}`", type_name.name));
                    self.unknown(span)
                }
            };
        }
        let _ = expected;
        let base = self.expr(base, None);
        // The fields of a prelude type are private to it; its `len` is a property.
        let private = self.is_prelude_type(&base.ty) && !self.body_in_prelude;
        let field_name = if private && name.name == "len" {
            "_len"
        } else {
            name.name.as_str()
        };
        let hidden = private || (self.is_opaque(&base.ty) && !self.body_in_prelude) || self.kept_field(&base.ty);
        if hidden && name.name.starts_with('_') {
            let found = self.ty_name(&base.ty);
            self.error(name.span, format!("`{found}` has no field `{}`", name.name));
            return self.unknown(span);
        }
        let fields: Option<&[FieldDef]> = match &base.ty {
            Ty::Struct(id) => Some(&self.program.structs[*id].fields),
            Ty::Error(id) => Some(&self.program.errors[*id].fields),
            _ => None,
        };
        // Which actor a handle is of, and where the actor lives, is what the handle is
        // made of, and not for a program to read.
        let inside = !self.body_in_prelude
            && matches!(&base.ty, Ty::Struct(id) if self.handles.contains(id))
            && matches!(field_name, "id" | "home");
        if let Some(fields) = fields {
            return match fields.iter().position(|f| f.name == field_name).filter(|_| !inside) {
                Some(index) => {
                    let ty = fields[index].ty.clone();
                    self.mk(ExprKind::Field(Box::new(base), index), ty, span)
                }
                None => {
                    let found = self.ty_name(&base.ty);
                    let (known, habit) = (self.field_names(&base.ty), self.field_habit(&base.ty, &name.name));
                    let message = format!("`{found}` has no field `{}`", name.name);
                    self.error_near(name.span, message, &name.name, known, habit);
                    self.unknown(span)
                }
            };
        }
        match (&base.ty, name.name.as_str()) {
            (Ty::List(_) | Ty::Str, "len") => self.mk(ExprKind::Builtin(Builtin::Len, vec![base]), Ty::Int, span),
            (Ty::Unknown, _) => self.unknown(span),
            (Ty::Optional(_), _) => {
                let found = self.ty_name(&base.ty);
                self.error(
                    base.span,
                    format!("the value may be absent, so `.{}` cannot be used on it", name.name),
                );
                self.label(format!("this is {}: it may be `none`", super::a(&found)));
                self.help(format!(
                    "give a value to use when there is none, `(x else fallback).{0}`, or use it only when it is there, `if value := x {{ ... value.{0} ... }}` (Section 4.8)",
                    name.name
                ));
                self.unknown(span)
            }
            (Ty::Result(..), _) => {
                let found = self.ty_name(&base.ty);
                self.error(
                    base.span,
                    format!("the value may be an error, so `.{}` cannot be used on it", name.name),
                );
                self.label(format!("this is {}", super::a(&found)));
                self.help(format!(
                    "handle the error first: `x?.{0}` passes it on to the caller, and `(x else fallback).{0}` gives a value in its place (Section 6.4)",
                    name.name
                ));
                self.unknown(span)
            }
            (other, _) => {
                let found = self.ty_name(other);
                let (known, habit) = (self.field_names(other), self.field_habit(other, &name.name));
                let message = format!("`{found}` has no field `{}`", name.name);
                self.error_near(name.span, message, &name.name, known, habit);
                self.unknown(span)
            }
        }
    }

    fn try_expr(&mut self, inner: &ast::Expr, span: Span, expected: Option<&Ty>) -> Expr {
        // What is wanted of `f()?` is what `f` succeeds with, which settles the type
        // parameters of a generic `f` that its arguments leave open: `x: Order = decode(text)?`.
        let hint = expected.filter(|_| matches!(inner.kind, ast::ExprKind::Call { .. }));
        let value = self.expr(inner, hint);
        match value.ty.clone() {
            Ty::Result(ok, set) => {
                match &self.ret {
                    Ty::Result(_, declared) => {
                        if !set.within(declared) {
                            let missing: Vec<String> = match (&set, declared) {
                                (ErrSet::Of(ids), ErrSet::Of(declared)) => ids
                                    .iter()
                                    .filter(|id| !declared.contains(id))
                                    .map(|&id| self.ty_name(&Ty::Error(id)))
                                    .collect(),
                                _ => vec!["Failure".to_string()],
                            };
                            let missing = missing.join(" | ");
                            let declared = self.error_names(declared);
                            self.error(
                                span,
                                format!("`?` propagates `{missing}`, which is not in this function's error set"),
                            );
                            self.label(format!("the function may fail only with `{declared}`"));
                            if self.here.is_none() {
                                self.help(format!("add `{missing}` after the `!` in the function's signature"));
                            }
                            self.help(
                                "to handle it here, give a value in its place with `else`, or inspect it with `match`",
                            );
                            self.fix_error_set(&missing);
                        }
                    }
                    Ty::Unknown => self.note_errors(&set),
                    _ => {
                        let errors = self.error_names(&set);
                        self.error(span, "`?` in a function that cannot fail");
                        self.label(format!("this passes `{errors}` on to the caller"));
                        self.related(self.here.map(|here| here.name), "no error is declared after a `!`");
                        if self.here.is_none() {
                            self.help(format!("add `! {errors}` to the function's signature"));
                        }
                        self.help("to handle the error here, give a value in its place with `else`, or inspect it with `match`");
                        self.fix_error_set(&errors);
                    }
                }
                self.mk(ExprKind::Try(Box::new(value)), *ok, span)
            }
            Ty::Optional(inner_ty) => {
                if !matches!(self.ret, Ty::Optional(_) | Ty::Unknown) {
                    self.error(
                        span,
                        "`?` on an optional in a function that does not return an optional",
                    );
                    self.label("if this is `none`, `?` returns `none` from the function");
                    let returns = match &self.ret {
                        Ty::Unit => "the function returns nothing".to_string(),
                        other => format!("the function returns `{}`", self.ty_name(other)),
                    };
                    self.related(self.here.and_then(|here| here.ret.or(Some(here.name))), returns);
                    self.help("give a value to use when there is none: `x else fallback` (Section 6.4)");
                }
                self.mk(ExprKind::Try(Box::new(value)), *inner_ty, span)
            }
            Ty::Unknown => self.unknown(span),
            Ty::Never => value,
            // The result of a function that, this time, has no errors to pass on.
            _ if self.without_errors() => value,
            other => {
                let found = self.ty_name(&other);
                self.error(span, format!("`?` needs a result or an optional, found `{found}`"));
                self.label(format!("this is {}: it is always there", super::a(&found)));
                let mark = Span {
                    start: span.end.saturating_sub(1),
                    end: span.end,
                };
                self.fix("there is nothing to pass on: remove the `?`", mark, "");
                self.unknown(span)
            }
        }
    }

    pub(super) fn binary(
        &mut self,
        op: ast::BinaryOp,
        lhs: &ast::Expr,
        rhs: &ast::Expr,
        span: Span,
        expected: Option<&Ty>,
    ) -> Expr {
        use ast::BinaryOp as B;
        match op {
            B::And | B::Or => {
                let a = self.expr_as(lhs, &Ty::Bool);
                let b = self.expr_as(rhs, &Ty::Bool);
                let kind = if op == B::And {
                    ExprKind::And(Box::new(a), Box::new(b))
                } else {
                    ExprKind::Or(Box::new(a), Box::new(b))
                };
                return self.mk(kind, Ty::Bool, span);
            }
            B::Range | B::RangeInclusive => {
                self.error_help(
                    span,
                    "a range is not supported as a value by this compiler yet",
                    "it supports ranges as the subject of a `for` loop: `for i in 0..n { ... }`",
                );
                return self.unknown(span);
            }
            _ => {}
        }
        let arithmetic = matches!(op, B::Add | B::Sub | B::Mul | B::Div | B::Rem);
        let a = self.expr(lhs, if arithmetic { expected } else { None });
        let b = self.expr(rhs, Some(&a.ty));
        if a.ty.is_unknown() || b.ty.is_unknown() {
            return self.unknown(span);
        }
        // A date has no arithmetic: it moves by days or by months, which are not the same.
        if arithmetic && self.date_ty().is_some_and(|date| a.ty == date || b.ty == date) {
            let (left, right) = (self.ty_name(&a.ty), self.ty_name(&b.ty));
            self.error_help(
                span,
                format!("`{}` is not defined for `{left}` and `{right}`", op.as_str()),
                "a date moves with `d.add_days(n)` and `d.add_months(n)`, and `a.days_until(b)` is the days from one date to another (Section 10.6)",
            );
            return self.unknown(span);
        }
        // Arithmetic on durations and times is the prelude's (`check/time.rs`).
        if arithmetic && (self.is_time_type(&a.ty) || self.is_time_type(&b.ty)) {
            let time_op = match op {
                B::Add => Some(BinOp::Add),
                B::Sub => Some(BinOp::Sub),
                B::Mul => Some(BinOp::Mul),
                B::Div => Some(BinOp::Div),
                _ => None,
            };
            match time_op.and_then(|time_op| self.time_arithmetic(time_op, &a, &b, span)) {
                Some(result) => return result,
                None if self
                    .decimal_ty()
                    .is_some_and(|decimal| a.ty == decimal || b.ty == decimal) =>
                {
                    let (left, right) = (self.ty_name(&a.ty), self.ty_name(&b.ty));
                    if a.ty == b.ty && matches!(op, B::Div | B::Rem) {
                        self.error_help(
                            span,
                            format!("`{}` is not defined for `Decimal`", op.as_str()),
                            "a quotient is asked for with the digits it is to have after the point, and is rounded half to even unless told otherwise: `a.divide(b, scale: 2)` (Section 4.3)",
                        );
                    } else {
                        self.error_help(
                            span,
                            format!("`{}` is not defined for `{left}` and `{right}`", op.as_str()),
                            "a `Decimal` mixes with no other number: write the other as one, `1.1d` for `1.1`, or `Decimal.from(n)` for an `int` (Section 4.2)",
                        );
                    }
                    return self.unknown(span);
                }
                None => {
                    let (left, right) = (self.ty_name(&a.ty), self.ty_name(&b.ty));
                    self.error_help(
                        span,
                        format!("`{}` is not defined for `{left}` and `{right}`", op.as_str()),
                        "durations add and subtract, and are multiplied and divided by an `int`; a `Time` and a `Duration` add to a `Time`, and two times subtract to a `Duration` (Section 10.6)",
                    );
                    return self.unknown(span);
                }
            }
        }
        let hir_op = match op {
            B::Add => BinOp::Add,
            B::Sub => BinOp::Sub,
            B::Mul => BinOp::Mul,
            B::Div => BinOp::Div,
            B::Rem => BinOp::Rem,
            B::Eq => BinOp::Eq,
            B::Ne => BinOp::Ne,
            B::Lt => BinOp::Lt,
            B::Le => BinOp::Le,
            B::Gt => BinOp::Gt,
            B::Ge => BinOp::Ge,
            B::Range | B::RangeInclusive | B::And | B::Or => unreachable!(),
        };
        // An operand that never arrives (`x + panic("...")`) is still evaluated, in order;
        // the operation itself never happens.
        if a.ty == Ty::Never || b.ty == Ty::Never {
            return self.mk(ExprKind::Binary(hir_op, Box::new(a), Box::new(b)), Ty::Never, span);
        }
        // An optional is compared with a value of what it may hold: `req.query("kind")
        // == "coffee"` is false when there is no `kind`, as it is when it is another.
        let (mut a, mut b) = (a, b);
        if matches!(hir_op, BinOp::Eq | BinOp::Ne) {
            if a.ty == Ty::optional(b.ty.clone()) {
                let ty = a.ty.clone();
                b = self.coerce(b, &ty);
            } else if b.ty == Ty::optional(a.ty.clone()) {
                let ty = b.ty.clone();
                a = self.coerce(a, &ty);
            }
        }
        if a.ty != b.ty {
            let (left, right) = (self.ty_name(&a.ty), self.ty_name(&b.ty));
            self.error(
                rhs.span,
                format!("`{}` cannot be applied to `{left}` and `{right}`", op.as_str()),
            );
            self.label(format!("this is {}", super::a(&right)));
            self.related(Some(lhs.span), format!("this is {}", super::a(&left)));
            // What the other side is called in advice: its name, if it is a variable.
            let named = |expr: &ast::Expr| match &expr.kind {
                ast::ExprKind::Name(name) => name.name.clone(),
                _ => "x".to_string(),
            };
            match (&a.ty, &b.ty) {
                (Ty::Int, Ty::Float) | (Ty::Float, Ty::Int) => {
                    self.note("an `int` never becomes a `float` by itself (Section 4.2)");
                    let int = if a.ty == Ty::Int { lhs } else { rhs };
                    match &int.kind {
                        ast::ExprKind::Int(_) => {
                            let end = Span {
                                start: int.span.end,
                                end: int.span.end,
                            };
                            self.fix("write the number as a `float`", end, ".0");
                        }
                        // Parentheses that are there already serve as the call's.
                        ast::ExprKind::Paren(_) => {
                            let start = int.span.start;
                            self.fix(
                                "convert the `int` to a `float`",
                                Span { start, end: start },
                                "float.from",
                            );
                        }
                        _ => {
                            let (start, end) = (int.span.start, int.span.end);
                            self.fix_edits(
                                "convert the `int` to a `float`",
                                vec![
                                    (Span { start, end: start }, "float.from(".to_string()),
                                    (Span { start: end, end }, ")".to_string()),
                                ],
                            );
                        }
                    }
                }
                (Ty::Str, _) | (_, Ty::Str) if matches!(op, ast::BinaryOp::Add) => {
                    let other = named(if a.ty == Ty::Str { rhs } else { lhs });
                    self.help(format!(
                        "put the value inside the string, as in `\"total: {{{other}}}\"`, or make it text with `{other}.to_string()`"
                    ));
                }
                (Ty::Str, Ty::Int | Ty::Float) | (Ty::Int | Ty::Float, Ty::Str) => {
                    let (text, number) = if a.ty == Ty::Str { (lhs, rhs) } else { (rhs, lhs) };
                    let kind = if matches!((&a.ty, &b.ty), (Ty::Float, _) | (_, Ty::Float)) {
                        "float"
                    } else {
                        "int"
                    };
                    // `n == "3"`: the number was meant.
                    let digits = match &text.kind {
                        ast::ExprKind::Str(parts) => match parts.as_slice() {
                            [ast::StrPart::Lit(digits)] if digits.parse::<f64>().is_ok() => Some(digits.clone()),
                            _ => None,
                        },
                        _ => None,
                    };
                    match digits {
                        Some(digits) if kind == "float" || digits.parse::<i64>().is_ok() => {
                            self.fix("compare with the number", text.span, digits);
                        }
                        _ => self.help(format!(
                            "to compare numbers, turn the text into one with `{kind}.parse({})`; to compare text, write `{}.to_string()`",
                            named(text),
                            named(number)
                        )),
                    }
                }
                (Ty::Optional(inner), other) | (other, Ty::Optional(inner)) if **inner == *other => {
                    self.help("one side may be absent: give it a value with `else`, as in `(x else 0)`, or unwrap it with `if value := x { ... }`");
                }
                (Ty::Result(inner, _), other) | (other, Ty::Result(inner, _)) if **inner == *other => {
                    self.help("one side may be an error: `x?` passes the error on to the caller, and `(x else 0)` gives a value in its place (Section 6.4)");
                }
                _ => self.note("both sides of an operator have the same type"),
            }
            return self.unknown(span);
        }
        let defined = match hir_op {
            BinOp::Add => matches!(a.ty, Ty::Int | Ty::Float | Ty::Str),
            BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => matches!(a.ty, Ty::Int | Ty::Float),
            // `==` is structural, and every type has it but function values (Section 3.4).
            BinOp::Eq | BinOp::Ne => self.has_equality(&a.ty),
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                // A type with a `compare` method has the four ordering operators, and
                // they call it (Section 4.9).
                if let Some(compare) = self.compare_method(&a.ty) {
                    let ordering = Expr {
                        kind: ExprKind::Call {
                            func: compare,
                            args: vec![
                                Arg {
                                    kind: self.sigs[compare]
                                        .receiver
                                        .as_ref()
                                        .map(|r| r.kind)
                                        .unwrap_or(ParamKind::Borrow),
                                    expr: a,
                                },
                                Arg {
                                    kind: self.sigs[compare].params[0].kind,
                                    expr: b,
                                },
                            ],
                        },
                        ty: Ty::Int,
                        span,
                    };
                    let zero = self.mk(ExprKind::Int(0), Ty::Int, span);
                    return self.mk(
                        ExprKind::Binary(hir_op, Box::new(ordering), Box::new(zero)),
                        Ty::Bool,
                        span,
                    );
                }
                matches!(a.ty, Ty::Int | Ty::Float | Ty::Str | Ty::Byte) || self.is_plain_enum(&a.ty)
            }
        };
        if !defined {
            let found = self.ty_name(&a.ty);
            let help = match hir_op {
                BinOp::Eq | BinOp::Ne if matches!(a.ty, Ty::Fn(_)) => "a function value has no `==` (Section 3.4)",
                BinOp::Eq | BinOp::Ne => "it holds a function, and a function value has no `==` (Section 3.4)",
                BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                    "give the type a method `fn (self: T) compare(other: T) -> int` to order it (Section 4.9)"
                }
                _ => "arithmetic is defined on the numeric types, and `+` also joins strings (Section 3.4)",
            };
            self.error_help(span, format!("`{}` is not defined for `{found}`", op.as_str()), help);
            self.label(format!("both sides are {}", super::a(&found)));
            return self.unknown(span);
        }
        let ty = if arithmetic { a.ty.clone() } else { Ty::Bool };
        self.mk(ExprKind::Binary(hir_op, Box::new(a), Box::new(b)), ty, span)
    }

    /// An enum whose variants all have no fields: it has `==` and an order (Section 4.7).
    fn is_plain_enum(&self, ty: &Ty) -> bool {
        match ty {
            Ty::Enum(id) => self.program.enums[*id].variants.iter().all(|v| v.fields.is_empty()),
            _ => false,
        }
    }

    fn list(&mut self, items: &[ast::Expr], span: Span, expected: Option<&Ty>) -> Expr {
        let hint = match expected.map(peel) {
            Some(Ty::List(element)) => Some((**element).clone()),
            _ => None,
        };
        let mut checked: Vec<Expr> = Vec::new();
        let mut element = hint;
        for item in items {
            let value = self.expr(item, element.as_ref());
            match &element {
                Some(ty) => {
                    // The first element sets the type, unless the context already did.
                    let first = checked
                        .first()
                        .map(|first: &Expr| first.span)
                        .filter(|_| expected.is_none());
                    let why = format!(
                        "the first element is {}, and a list has one type of element",
                        super::a(&self.ty_name(ty))
                    );
                    checked.push(self.coerce_for(value, ty, first, &why));
                }
                None => {
                    if !value.ty.is_unknown() && value.ty != Ty::Never {
                        element = Some(value.ty.clone());
                    }
                    checked.push(value);
                }
            }
        }
        let Some(element) = element else {
            if items.is_empty() && !expected.is_some_and(|ty| ty.is_unknown()) {
                self.error_help(
                    span,
                    "the element type of `[]` is not known here",
                    "give the binding a type: `xs: [int] = []`",
                );
            }
            // `[panic("...")]`: the list is never built, but its first element is evaluated.
            return match checked.into_iter().find(|item| item.ty == Ty::Never) {
                Some(never) => never,
                None => self.unknown(span),
            };
        };
        if matches!(element, Ty::Unit) {
            self.error(span, "a list of `()` is not supported");
            return self.unknown(span);
        }
        self.mk(ExprKind::ListLit(checked), Ty::list(element), span)
    }

    /// The type that a type name in an expression names: `Order`, `Stack<int>`, or a
    /// generic type whose arguments the expected type supplies.
    /// An expression that names a type of an imported package, `orders.Order`, with
    /// the name the type has in the program in its place; and one that begins with such
    /// a name (`orders.State.Paid`, `orders.Box<int>`) likewise. Nothing for any other.
    pub(super) fn unqualified(&mut self, written: &ast::Expr) -> Option<ast::Expr> {
        let kind = match &written.kind {
            ast::ExprKind::Field { base, name } => {
                if let ast::ExprKind::Name(package) = &base.kind
                    && name.is_upper()
                    && self.resolve_local(&package.name).is_none()
                    && let Some(full) = self.package_type(&[package.clone(), name.clone()])
                {
                    ast::ExprKind::Name(ast::Ident {
                        name: full,
                        span: written.span,
                    })
                } else {
                    ast::ExprKind::Field {
                        base: Box::new(self.unqualified(base)?),
                        name: name.clone(),
                    }
                }
            }
            ast::ExprKind::TypeApp { base, args } => ast::ExprKind::TypeApp {
                base: Box::new(self.unqualified(base)?),
                args: args.clone(),
            },
            _ => return None,
        };
        Some(ast::Expr {
            kind,
            span: written.span,
        })
    }

    pub(super) fn literal_type(&mut self, ty: &ast::Expr, expected: Option<&Ty>) -> Option<Ty> {
        if let Some(of) = self.unqualified(ty) {
            return self.literal_type(&of, expected);
        }
        match &ty.kind {
            ast::ExprKind::Name(name) => {
                if let Some(bound) = self.type_env.get(&name.name) {
                    return Some(bound.clone());
                }
                if self.generic_types.contains_key(&name.name) {
                    // The arguments come from the type the context expects.
                    if let Some(expected) = expected.map(peel)
                        && self
                            .instance_of
                            .get(expected)
                            .is_some_and(|(generic, _)| *generic == name.name)
                    {
                        return Some(expected.clone());
                    }
                    // The expected type was wrong, and that was reported.
                    if expected.is_some_and(|ty| ty.is_unknown()) {
                        return None;
                    }
                    let params = self.generic_types[&name.name].params.join(", ");
                    self.error_help(
                        ty.span,
                        format!("the type arguments of `{}` are not known here", name.name),
                        format!(
                            "say what `{params}` is: `{0}<int>`, or whatever it holds, in place of `{0}`",
                            name.name
                        ),
                    );
                    return None;
                }
                match self.type_names.get(&name.name) {
                    Some(TypeName::Struct(id)) => Some(Ty::Struct(*id)),
                    Some(TypeName::Enum(id)) => Some(Ty::Enum(*id)),
                    Some(TypeName::Error(id)) => Some(Ty::Error(*id)),
                    Some(TypeName::Alias(aliased)) => Some(aliased.clone()),
                    Some(TypeName::ErrorSet(_)) => {
                        self.error(ty.span, format!("`{}` is an error set, not a type", name.name));
                        None
                    }
                    None => {
                        self.error(ty.span, format!("unknown type `{}`", name.name));
                        None
                    }
                }
            }
            ast::ExprKind::TypeApp { base, args } => {
                let ast::ExprKind::Name(name) = &base.kind else {
                    self.unsupported(ty.span, "types from other packages are");
                    return None;
                };
                let args: Vec<Ty> = args.iter().map(|arg| self.resolve_type(arg)).collect();
                if name.name == "List" && args.len() == 1 {
                    return Some(Ty::list(args[0].clone()));
                }
                if !self.generic_types.contains_key(&name.name) {
                    self.error(ty.span, format!("`{}` is not a generic type", name.name));
                    return None;
                }
                match self.instantiate_type(&name.name, args, ty.span) {
                    Ty::Unknown => None,
                    made => Some(made),
                }
            }
            _ => {
                self.unsupported(ty.span, "types from other packages are");
                None
            }
        }
    }

    fn struct_literal(&mut self, ty: &ast::Expr, fields: &[ast::FieldInit], span: Span, expected: Option<&Ty>) -> Expr {
        #[derive(Clone, Copy)]
        enum Target {
            Struct(StructId),
            Error(ErrorId),
            Variant(EnumId, usize),
        }
        // `http.Response { ... }`, `orders.State.Paid { ... }`, `orders.Box<int> { ... }`:
        // a type of a package the file imports.
        if let Some(of) = self.unqualified(ty) {
            return self.struct_literal(&of, fields, span, expected);
        }
        let target = match &ty.kind {
            ast::ExprKind::Name(name) => {
                let variant = match expected.map(peel) {
                    Some(Ty::Enum(id)) => self.program.enums[*id]
                        .variants
                        .iter()
                        .position(|v| v.name == super::bare(&name.name))
                        .map(|v| Target::Variant(*id, v)),
                    _ => None,
                };
                if let Some(Target::Variant(id, _)) = variant {
                    self.kept_variant(id, ty.span);
                }
                match variant {
                    Some(target) => Some(target),
                    None => match self.literal_type(ty, expected) {
                        Some(Ty::Struct(id)) => Some(Target::Struct(id)),
                        Some(Ty::Error(id)) => Some(Target::Error(id)),
                        Some(_) => {
                            self.error(ty.span, format!("`{}` is not a struct", name.name));
                            None
                        }
                        None => None,
                    },
                }
            }
            ast::ExprKind::TypeApp { .. } => match self.literal_type(ty, expected) {
                Some(Ty::Struct(id)) => Some(Target::Struct(id)),
                Some(_) => {
                    self.error(ty.span, "this is not a struct");
                    None
                }
                None => None,
            },
            ast::ExprKind::Field { base, name } => match self.literal_type(base, expected) {
                Some(Ty::Enum(id)) => match self.program.enums[id].variants.iter().position(|v| v.name == name.name) {
                    Some(variant) => Some(Target::Variant(id, variant)),
                    None => {
                        let enum_name = self.program.enums[id].name.clone();
                        let known = self.program.enums[id].variants.iter().map(|v| v.name.clone()).collect();
                        let message = format!("`{enum_name}` has no variant `{}`", name.name);
                        self.error_near(name.span, message, &name.name, known, None);
                        None
                    }
                },
                Some(_) => {
                    self.unsupported(ty.span, "types from other packages are");
                    None
                }
                None => None,
            },
            _ => {
                self.unsupported(ty.span, "types from other packages are");
                None
            }
        };
        let Some(target) = target else {
            for field in fields {
                if let Some(value) = &field.value {
                    self.expr(value, None);
                }
            }
            return self.unknown(span);
        };
        // A type of the library with something hidden in it is made by the library,
        // unless what is hidden has a default and the rest is the program's to give.
        let sealed = |checker: &Self, id: StructId| {
            checker.is_opaque(&Ty::Struct(id))
                && checker.program.structs[id]
                    .fields
                    .iter()
                    .zip(&checker.defaults[id])
                    .any(|(field, default)| field.name.starts_with('_') && default.is_none())
        };
        if let Target::Struct(id) = target
            && !self.body_in_prelude
            && (sealed(self, id) || self.handles.contains(&id))
        {
            let name = self.ty_name(&Ty::Struct(id));
            let help = if self.decimal_ty() == Some(Ty::Struct(id)) {
                "its fields are its own: write a decimal as `19.99d`, or get one from `Decimal.parse(text)` or `Decimal.from(n)`"
            } else if self.program.structs[id].name == "Zone" && id < self.prelude_types.0 {
                "its field is its own: a zone is had by its name, `Zone.named(\"Europe/Paris\")`, which fails for a name that the database of zones does not have"
            } else if self.date_ty() == Some(Ty::Struct(id)) {
                "its fields are its own: get a date from `Date.of(year, month, day)`, `Date.parse(text)`, `time.today()` or `t.date()`"
            } else if self.is_time_type(&Ty::Struct(id)) {
                "its fields are its own: write a duration as `250ms`, and get a time from `time.now()`, `Time.from_unix(seconds)` or `Time.parse(text)`"
            } else if self.handles.contains(&id) {
                "a handle is what `start` returns: it is made by starting an actor, and by nothing else"
            } else if self.program.structs[id].name == "Http__Response" {
                "a response is made by `http.text`, `http.json`, `http.status`, `http.send`, `http.bytes`, `http.file` or `http.redirect`, and changed by `.header(...)`, `.cookie(...)` and `.with_status(...)`"
            } else if self.program.structs[id].name == "Http__Request" {
                "a request is what a client sent, and a handler is given it: to try a handler's work without a server, put the work in a function that takes what it needs of the request"
            } else {
                "its fields are its own: a query is written as a string literal where one is expected, and a pool and its rows come from the database"
            };
            self.error_help(span, format!("`{name}` cannot be built from its fields"), help);
            return self.unknown(span);
        }
        if let Target::Struct(id) = target
            && !self.body_in_prelude
            && self.is_prelude_type(&Ty::Struct(id))
        {
            let name = self.ty_name(&Ty::Struct(id));
            self.error_help(
                span,
                format!("`{name}` cannot be built from its fields"),
                "its fields are its own: write a map as `[key: value]` or `[:]`, and make a set with `Set<T>.new()` or `Set<T>.from_list(items)`",
            );
            return self.unknown(span);
        }
        let (defs, defaults, type_name): (Vec<FieldDef>, Vec<Option<Expr>>, String) = match target {
            Target::Struct(id) => (
                self.program.structs[id].fields.clone(),
                self.defaults[id].clone(),
                self.program.structs[id].name.clone(),
            ),
            Target::Error(id) => (
                self.program.errors[id].fields.clone(),
                Vec::new(),
                self.program.errors[id].name.clone(),
            ),
            Target::Variant(id, variant) => {
                let def = &self.program.enums[id].variants[variant];
                (def.fields.clone(), Vec::new(), def.name.clone())
            }
        };
        // What a type of the library keeps hidden is not a program's to give either.
        let hidden = |checker: &Self, name: &str| {
            name.starts_with('_')
                && !checker.body_in_prelude
                && (matches!(target, Target::Struct(id) if id < checker.prelude_types.0)
                    || match target {
                        Target::Struct(id) => checker.kept_field(&Ty::Struct(id)),
                        Target::Error(id) => checker.kept_field(&Ty::Error(id)),
                        Target::Variant(id, _) => checker.kept_field(&Ty::Enum(id)),
                    })
        };
        let mut given: HashMap<usize, Expr> = HashMap::new();
        for field in fields {
            let found = defs
                .iter()
                .position(|d| d.name == field.name.name)
                .filter(|_| !hidden(self, &field.name.name));
            let Some(index) = found else {
                let known = defs
                    .iter()
                    .map(|d| d.name.clone())
                    .filter(|name| !hidden(self, name))
                    .collect();
                let shown = match target {
                    Target::Struct(id) => self.ty_name(&Ty::Struct(id)),
                    _ => type_name.clone(),
                };
                let message = format!("`{shown}` has no field `{}`", field.name.name);
                self.error_near(field.name.span, message, &field.name.name, known, None);
                if let Some(value) = &field.value {
                    self.expr(value, None);
                }
                continue;
            };
            let base = type_name.split('<').next().unwrap_or("").to_string();
            let declared = self.field_at.get(&(base, field.name.name.clone())).copied();
            let why = format!("the field `{}` is declared here", field.name.name);
            let reported = self.diags.len();
            let value = match &field.value {
                Some(value) => {
                    let checked = self.expr_as(value, &defs[index].ty);
                    self.explain_mismatch(reported, value.span, declared, why);
                    checked
                }
                None => {
                    // The shorthand `Order { id }` names a variable of the field's name.
                    let name = self.name(&field.name, Some(&defs[index].ty));
                    self.coerce_for(name, &defs[index].ty, declared, &why)
                }
            };
            if given.insert(index, value).is_some() {
                self.error(
                    field.name.span,
                    format!("the field `{}` is given twice", field.name.name),
                );
                self.label("given again here");
                let first = fields.iter().find(|f| f.name.name == field.name.name);
                self.related(first.map(|f| f.name.span), "first given here");
            }
        }
        let mut values = Vec::new();
        let mut missing = Vec::new();
        for (index, def) in defs.iter().enumerate() {
            match given.remove(&index) {
                Some(value) => values.push(value),
                None => match defaults.get(index).cloned().flatten() {
                    Some(default) => values.push(default),
                    None => {
                        missing.push(def.name.clone());
                        values.push(self.unknown(span));
                    }
                },
            }
        }
        if !missing.is_empty() {
            let shown = match target {
                Target::Struct(id) => self.ty_name(&Ty::Struct(id)),
                Target::Error(id) => self.ty_name(&Ty::Error(id)),
                _ => type_name.clone(),
            };
            self.error(span, format!("`{shown}` is missing {}", list_fields(&missing)));
            let quoted: Vec<String> = missing.iter().map(|name| format!("`{name}: ...`")).collect();
            self.label(format!("add {}", quoted.join(", ")));
            let base = type_name.split('<').next().unwrap_or("").to_string();
            for name in missing.iter().take(3) {
                let at = self.field_at.get(&(base.clone(), name.clone())).copied();
                self.related(at, format!("`{name}` has no default"));
            }
            self.note("every field without a default must be given (Section 4.6)");
            return self.unknown(span);
        }
        match target {
            Target::Struct(id) => self.mk(ExprKind::StructLit(id, values), Ty::Struct(id), span),
            Target::Error(id) => self.mk(ExprKind::ErrorLit(id, values), Ty::Error(id), span),
            Target::Variant(id, variant) => self.mk(ExprKind::EnumLit(id, variant, values), Ty::Enum(id), span),
        }
    }

    fn else_expr(
        &mut self,
        inner: &ast::Expr,
        binding: Option<&ast::Ident>,
        alt: &ast::Body,
        span: Span,
        expected: Option<&Ty>,
    ) -> Expr {
        // The operand of `else` is a `T?` or a `T ! E`, so the hint for it is not the hint
        // for the whole expression, except to a generic call, which takes from it what
        // it succeeds with.
        let hint = expected.filter(|_| matches!(inner.kind, ast::ExprKind::Call { .. }));
        let value = self.expr(inner, hint);
        // The result of a function that, this time, has no errors.
        let absent =
            !matches!(value.ty, Ty::Optional(_) | Ty::Result(..) | Ty::Unknown | Ty::Never) && self.without_errors();
        let (ok, set) = match value.ty.clone() {
            Ty::Optional(inner) => (*inner, None),
            Ty::Result(ok, set) => (*ok, Some(set)),
            Ty::Unknown | Ty::Never => (Ty::Unknown, None),
            _ if absent => (Ty::Unknown, None),
            other => {
                let found = self.ty_name(&other);
                self.error(
                    inner.span,
                    format!("`else` needs a result or an optional before it, found `{found}`"),
                );
                self.label(format!("this is {}: it is always there", super::a(&found)));
                let rest = Span {
                    start: inner.span.end,
                    end: span.end,
                };
                self.fix("the alternative would never be used: remove it", rest, "");
                (Ty::Unknown, None)
            }
        };
        if value.ty == Ty::Never || absent {
            // The operand never arrives, or always does, so the alternative is never
            // used. It is still checked, in a scope of its own.
            let reported = self.diags.len();
            self.scopes.push(HashMap::new());
            if let Some(name) = binding {
                self.declare(name, Ty::Unknown, false, LocalKind::Owned);
            }
            self.body(alt, true, None);
            self.scopes.pop();
            if absent {
                self.diags.truncate(reported);
            }
            return value;
        }
        self.scopes.push(HashMap::new());
        let binding = match (binding, &set) {
            (Some(name), Some(set)) => {
                // The error's type is the error type when the set has one member, and
                // `Failure` otherwise (Section 6.4).
                let ty = match set {
                    ErrSet::Of(ids) if ids.len() == 1 => Ty::Error(ids[0]),
                    _ => Ty::Failure,
                };
                let several = ty == Ty::Failure;
                let local = self.declare(name, ty, false, LocalKind::Owned);
                if several {
                    self.caught.insert(local, set.clone());
                }
                Some(local)
            }
            (Some(name), None) => {
                if !value.ty.is_unknown() {
                    self.error_help(
                        name.span,
                        "an optional has no error to bind",
                        "write `else` followed by the alternative",
                    );
                }
                Some(self.declare(name, Ty::Unknown, false, LocalKind::Owned))
            }
            (None, _) => None,
        };
        let mut alt = self.body(alt, true, Some(&ok));
        self.scopes.pop();
        // The alternative may itself be an optional or a result of what the operand
        // holds, and then so is the whole (Section 5.7).
        let mut whole = ok.clone();
        if !diverges(&alt)
            && let Some(alt_value) = alt.value.take()
        {
            let again = match &alt_value.ty {
                Ty::Optional(held) | Ty::Result(held, _) => **held == ok && !ok.is_unknown(),
                _ => false,
            };
            if again {
                whole = alt_value.ty.clone();
                alt.value = Some(alt_value);
            } else {
                let why = format!(
                    "this is {}, so the alternative is {}",
                    super::a(&self.ty_name(&value.ty)),
                    super::a(&self.ty_name(&ok))
                );
                alt.value = Some(Box::new(self.coerce_for(*alt_value, &ok, Some(inner.span), &why)));
            }
        }
        self.mk(
            ExprKind::Else {
                expr: Box::new(value),
                binding,
                alt: Box::new(alt),
            },
            whole,
            span,
        )
    }

    /// The body of a `match` arm, or the alternative of an `else`: a block or one simple
    /// statement. `value` says whether the body is used for its value.
    pub(super) fn body(&mut self, body: &ast::Body, value: bool, expected: Option<&Ty>) -> Block {
        match body {
            ast::Body::Block(block) if value => self.value_block(block, expected),
            ast::Body::Block(block) => self.block(block),
            ast::Body::Stmt(stmt) => match &stmt.kind {
                ast::StmtKind::Expr(expr) if value => {
                    let checked = self.expr(expr, expected);
                    Block {
                        stmts: Vec::new(),
                        value: Some(Box::new(checked)),
                    }
                }
                ast::StmtKind::Return(_)
                | ast::StmtKind::Fail(_)
                | ast::StmtKind::Break(_)
                | ast::StmtKind::Continue(_) => {
                    let jump = self.jump(stmt);
                    Block {
                        stmts: vec![Stmt::Expr(jump)],
                        value: None,
                    }
                }
                _ => {
                    self.scopes.push(HashMap::new());
                    let mut checked = Vec::new();
                    self.stmt(stmt, &mut checked);
                    self.scopes.pop();
                    if value && !matches!(stmt.kind, ast::StmtKind::Expr(_)) {
                        self.error(stmt.span, "an assignment has no value");
                    }
                    Block {
                        stmts: checked,
                        value: None,
                    }
                }
            },
        }
    }

    /// Gives the branches of an `if` or `match` used for its value one type (Section 6.6).
    pub(super) fn unify(&mut self, branches: &mut [&mut Block], expected: Option<&Ty>, span: Span) -> Ty {
        let types: Vec<Ty> = branches.iter().map(|b| block_ty(b)).collect();
        let Some(first) = types.iter().find(|ty| **ty != Ty::Never && !ty.is_unknown()) else {
            return if types.iter().all(|ty| *ty == Ty::Never) && !types.is_empty() {
                Ty::Never
            } else {
                Ty::Unknown
            };
        };
        if types
            .iter()
            .all(|ty| ty == first || *ty == Ty::Never || ty.is_unknown())
        {
            return first.clone();
        }
        // The branches differ. They may still all widen to the expected type, or to the
        // optional of the one type the others have.
        let target = match expected {
            Some(expected) if types.iter().all(|ty| self.can_coerce(ty, expected)) => Some(expected.clone()),
            _ => types
                .iter()
                .find(|candidate| types.iter().all(|ty| self.can_coerce(ty, candidate)))
                .cloned(),
        };
        let Some(target) = target else {
            let other = types
                .iter()
                .find(|ty| *ty != first && **ty != Ty::Never && !ty.is_unknown())
                .unwrap();
            let (a, b) = (self.ty_name(first), self.ty_name(other));
            // The mistake is shown at the branch that departs from the first.
            let value_at = |wanted: &Ty| {
                branches
                    .iter()
                    .find(|branch| block_ty(branch) == *wanted)
                    .and_then(|branch| branch.value.as_ref().map(|value| value.span))
            };
            let (first_at, other_at) = (value_at(first), value_at(other));
            self.error(
                other_at.unwrap_or(span),
                format!("the branches have different types: `{a}` and `{b}`"),
            );
            if other_at.is_some() {
                self.label(format!("this branch is {}", super::a(&b)));
                self.related(first_at, format!("this branch is {}", super::a(&a)));
            }
            self.note("every branch of an `if` or `match` used for its value has the same type (Section 6.6)");
            return Ty::Unknown;
        };
        for branch in branches.iter_mut() {
            if let Some(value) = branch.value.take() {
                branch.value = Some(Box::new(self.coerce(*value, &target)));
            }
        }
        target
    }

    pub(super) fn if_expr(&mut self, if_expr: &ast::IfExpr, span: Span, value: bool, expected: Option<&Ty>) -> Expr {
        self.scopes.push(HashMap::new());
        let cond = self.cond(&if_expr.cond);
        let parts = std::mem::take(&mut self.cond_stmts);
        let mut then = if value {
            self.value_block(&if_expr.then, expected)
        } else {
            self.block(&if_expr.then)
        };
        then.stmts.splice(0..0, parts);
        self.scopes.pop();
        let mut otherwise = match &if_expr.otherwise {
            Some(ast::ElseBranch::Block(block)) => Some(if value {
                self.value_block(block, expected)
            } else {
                self.block(block)
            }),
            Some(ast::ElseBranch::If(next)) => {
                let nested = self.if_expr(next, span, value, expected);
                Some(if value {
                    Block {
                        stmts: Vec::new(),
                        value: Some(Box::new(nested)),
                    }
                } else {
                    Block {
                        stmts: vec![Stmt::Expr(nested)],
                        value: None,
                    }
                })
            }
            None => None,
        };
        let ty = if value {
            match &mut otherwise {
                Some(otherwise) => self.unify(&mut [&mut then, otherwise], expected, span),
                None => {
                    self.error_help(
                        span,
                        "an `if` used for its value needs an `else`",
                        "add `else { ... }` with a value of the same type (Section 6.6)",
                    );
                    Ty::Unknown
                }
            }
        } else if diverges(&then) && otherwise.as_ref().is_some_and(diverges) {
            Ty::Never
        } else {
            Ty::Unit
        };
        self.mk(
            ExprKind::If {
                cond: Box::new(cond),
                then,
                otherwise,
            },
            ty,
            span,
        )
    }
}

fn list_fields(names: &[String]) -> String {
    let quoted: Vec<String> = names.iter().map(|n| format!("`{n}`")).collect();
    match quoted.as_slice() {
        [one] => format!("the field {one}"),
        _ => format!("the fields {}", quoted.join(", ")),
    }
}
