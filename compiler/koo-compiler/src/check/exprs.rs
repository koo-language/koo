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
fn parse_format(text: &str, ty: &Ty) -> Result<Format, &'static str> {
    let mut format = Format::default();
    let mut rest = text;
    for (symbol, align) in [('<', Align::Left), ('>', Align::Right), ('^', Align::Center)] {
        if let Some(after) = rest.strip_prefix(symbol) {
            format.align = align;
            rest = after;
            break;
        }
    }
    let number = matches!(ty, Ty::Int | Ty::Float | Ty::Byte);
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
        if *ty != Ty::Float {
            return Err("a precision is for a `float`");
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
            ast::ExprKind::Decimal(_) => {
                self.unsupported(span, "`Decimal` is");
                self.unknown(span)
            }
            ast::ExprKind::Duration(_) => {
                self.unsupported(span, "`Duration` is");
                self.unknown(span)
            }
            ast::ExprKind::Char(_) => {
                self.unsupported(span, "`char` is");
                self.unknown(span)
            }
            ast::ExprKind::RawStr(text) => self.mk(ExprKind::Str(text.clone()), Ty::Str, span),
            ast::ExprKind::Str(parts) => self.string(parts, span),
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
            ast::ExprKind::Try(inner) => self.try_expr(inner, span),
            ast::ExprKind::Unary { op, expr: inner } => match op {
                ast::UnaryOp::Neg => {
                    if let ast::ExprKind::Int(text) = &inner.kind {
                        return self.int_literal(text, true, expected, span);
                    }
                    let operand = self.expr(inner, expected);
                    match operand.ty {
                        Ty::Int | Ty::Float => {
                            let ty = operand.ty.clone();
                            self.mk(ExprKind::Unary(UnOp::Neg, Box::new(operand)), ty, span)
                        }
                        Ty::Unknown => self.unknown(span),
                        _ => {
                            let found = self.ty_name(&operand.ty);
                            self.error(span, format!("`-` is not defined for `{found}`"));
                            self.unknown(span)
                        }
                    }
                }
                ast::UnaryOp::Not => {
                    let operand = self.expr(inner, Some(&Ty::Bool));
                    let operand = self.coerce(operand, &Ty::Bool);
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

    fn string(&mut self, parts: &[ast::StrPart], span: Span) -> Expr {
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
                    let value = self.expr(expr, None);
                    self.require_printable(&value);
                    let format = match format.as_deref() {
                        None => Format::default(),
                        Some(text) => match parse_format(text, &value.ty) {
                            Ok(format) => format,
                            Err(why) => {
                                if !value.ty.is_unknown() {
                                    self.error_help(
                                        expr.span,
                                        format!("the format `{text}` cannot be used here: {why}"),
                                        "a format is an alignment (`<`, `>` or `^`), a width, and for a float a precision: `{name:>10}`, `{id:08}`, `{ratio:.2}`, `{price:8.2}`",
                                    );
                                }
                                Format::default()
                            }
                        },
                    };
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
            if let Some(constant) = self.consts.get(&name.name) {
                let mut constant = constant.clone();
                constant.span = span;
                return constant;
            }
            if let Some(&id) = self
                .free_fns
                .get(&name.name)
                .filter(|_| !name.name.contains("__") || self.body_in_prelude)
            {
                // A named function is a value of its function type (Section 4.11).
                let sig = &self.sigs[id];
                let ty = Ty::Fn(Box::new(FnTy {
                    params: sig.params.iter().map(|p| (p.kind, p.ty.clone())).collect(),
                    ret: sig.ret.clone(),
                }));
                return self.mk(ExprKind::FnRef(id), ty, span);
            }
            if self.generic_fns.contains_key(&name.name) || matches!(name.name.as_str(), "print" | "panic") {
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
                .position(|v| v.name == name.name);
            if let Some(variant) = variant {
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
                let owners: Vec<String> = self
                    .program
                    .enums
                    .iter()
                    .filter(|e| e.variants.iter().any(|v| v.name == name.name))
                    .map(|e| e.name.clone())
                    .collect();
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
                            format!("`{}` has no variant `{}`", type_name.name, name.name),
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
                Some(TypeName::Enum(id)) => {
                    let variant = self.program.enums[id].variants.iter().position(|v| v.name == name.name);
                    match variant {
                        Some(variant) => self.variant_value(id, variant, span),
                        None => {
                            let known = self.program.enums[id].variants.iter().map(|v| v.name.clone()).collect();
                            self.error_near(
                                name.span,
                                format!("`{}` has no variant `{}`", type_name.name, name.name),
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
        if private && name.name.starts_with('_') {
            let found = self.ty_name(&base.ty);
            self.error(name.span, format!("`{found}` has no field `{}`", name.name));
            return self.unknown(span);
        }
        let fields: Option<&[FieldDef]> = match &base.ty {
            Ty::Struct(id) => Some(&self.program.structs[*id].fields),
            Ty::Error(id) => Some(&self.program.errors[*id].fields),
            _ => None,
        };
        if let Some(fields) = fields {
            return match fields.iter().position(|f| f.name == field_name) {
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
                self.error_help(
                    span,
                    format!("the value may be absent, so `.{}` cannot be used on it", name.name),
                    "unwrap it first with `?`, `else` or `match` (Section 4.8)",
                );
                self.unknown(span)
            }
            (Ty::Result(..), _) => {
                self.error_help(
                    span,
                    format!("the value may be an error, so `.{}` cannot be used on it", name.name),
                    "handle the result first with `?`, `else` or `match` (Section 6.3)",
                );
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

    fn try_expr(&mut self, inner: &ast::Expr, span: Span) -> Expr {
        let value = self.expr(inner, None);
        match value.ty.clone() {
            Ty::Result(ok, set) => {
                match &self.ret {
                    Ty::Result(_, declared) => {
                        if !set.within(declared) {
                            let missing: Vec<String> = match (&set, declared) {
                                (ErrSet::Of(ids), ErrSet::Of(declared)) => ids
                                    .iter()
                                    .filter(|id| !declared.contains(id))
                                    .map(|&id| self.program.errors[id].name.clone())
                                    .collect(),
                                _ => vec!["Failure".to_string()],
                            };
                            let missing = missing.join(" | ");
                            self.error_help(
                                span,
                                format!("`?` propagates `{missing}`, which is not in this function's error set"),
                                format!(
                                    "add `{missing}` after the `!` in the function's signature, or handle it here with `else` or `match`"
                                ),
                            );
                        }
                    }
                    Ty::Unknown => self.note_errors(&set),
                    _ => {
                        let set = self.ty_name(&Ty::Result(Box::new(Ty::Unit), set));
                        let set = set.trim_start_matches("() ");
                        self.error_help(
                            span,
                            "`?` in a function that cannot fail",
                            format!("add `{set}` to the function's signature, or handle the error here with `else` or `match`"),
                        );
                    }
                }
                self.mk(ExprKind::Try(Box::new(value)), *ok, span)
            }
            Ty::Optional(inner_ty) => {
                if !matches!(self.ret, Ty::Optional(_) | Ty::Unknown) {
                    self.error_help(
                        span,
                        "`?` on an optional in a function that does not return an optional",
                        "supply an alternative with `else` (Section 6.4)",
                    );
                }
                self.mk(ExprKind::Try(Box::new(value)), *inner_ty, span)
            }
            Ty::Unknown => self.unknown(span),
            Ty::Never => value,
            other => {
                let found = self.ty_name(&other);
                self.error_help(
                    span,
                    format!("`?` needs a result or an optional, found `{found}`"),
                    "there is nothing to propagate: remove the `?`",
                );
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
        if a.ty != b.ty {
            let (left, right) = (self.ty_name(&a.ty), self.ty_name(&b.ty));
            let help = match (&a.ty, &b.ty) {
                (Ty::Int, Ty::Float) | (Ty::Float, Ty::Int) => {
                    "numeric types do not mix: convert one side with `float.from(x)` (Section 4.2)"
                }
                _ => "both sides of an operator have the same type",
            };
            self.error_help(
                span,
                format!("`{}` cannot be applied to `{left}` and `{right}`", op.as_str()),
                help,
            );
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
                BinOp::Eq | BinOp::Ne if self.is_prelude_type(&a.ty) => {
                    "maps and sets cannot be compared with `==` in this compiler yet"
                }
                BinOp::Eq | BinOp::Ne => "a function value has no `==` (Section 3.4)",
                BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                    "give the type a method `fn (self: T) compare(other: T) -> int` to order it (Section 4.9)"
                }
                _ => "arithmetic is defined on the numeric types, and `+` also joins strings (Section 3.4)",
            };
            self.error_help(span, format!("`{}` is not defined for `{found}`", op.as_str()), help);
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
                Some(ty) => checked.push(self.coerce(value, ty)),
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
    pub(super) fn literal_type(&mut self, ty: &ast::Expr, expected: Option<&Ty>) -> Option<Ty> {
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
                    let params = self.generic_types[&name.name].params.join(", ");
                    self.error_help(
                        ty.span,
                        format!("the type arguments of `{}` are not known here", name.name),
                        format!("write them: `{}<{params}>`", name.name),
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
        let target = match &ty.kind {
            ast::ExprKind::Name(name) => {
                let variant = match expected.map(peel) {
                    Some(Ty::Enum(id)) => self.program.enums[*id]
                        .variants
                        .iter()
                        .position(|v| v.name == name.name)
                        .map(|v| Target::Variant(*id, v)),
                    _ => None,
                };
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
        let mut given: HashMap<usize, Expr> = HashMap::new();
        for field in fields {
            let Some(index) = defs.iter().position(|d| d.name == field.name.name) else {
                let known = defs.iter().map(|d| d.name.clone()).collect();
                let message = format!("`{type_name}` has no field `{}`", field.name.name);
                self.error_near(field.name.span, message, &field.name.name, known, None);
                if let Some(value) = &field.value {
                    self.expr(value, None);
                }
                continue;
            };
            let value = match &field.value {
                Some(value) => self.expr_as(value, &defs[index].ty),
                None => {
                    // The shorthand `Order { id }` names a variable of the field's name.
                    let name = self.name(&field.name, Some(&defs[index].ty));
                    self.coerce(name, &defs[index].ty)
                }
            };
            if given.insert(index, value).is_some() {
                self.error(
                    field.name.span,
                    format!("the field `{}` is given twice", field.name.name),
                );
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
            self.error_help(
                span,
                format!("`{type_name}` is missing {}", list_fields(&missing)),
                "every field without a default must be given (Section 4.6)",
            );
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
        // for the whole expression.
        let _ = expected;
        let value = self.expr(inner, None);
        let (ok, set) = match value.ty.clone() {
            Ty::Optional(inner) => (*inner, None),
            Ty::Result(ok, set) => (*ok, Some(set)),
            Ty::Unknown | Ty::Never => (Ty::Unknown, None),
            other => {
                let found = self.ty_name(&other);
                self.error_help(
                    inner.span,
                    format!("`else` needs a result or an optional before it, found `{found}`"),
                    "the value is always present, so the alternative would never be used",
                );
                (Ty::Unknown, None)
            }
        };
        if value.ty == Ty::Never {
            // The operand never arrives, so neither does the alternative. It is still
            // checked, in a scope of its own.
            self.scopes.push(HashMap::new());
            if let Some(name) = binding {
                self.declare(name, Ty::Unknown, false, LocalKind::Owned);
            }
            self.body(alt, true, None);
            self.scopes.pop();
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
                Some(self.declare(name, ty, false, LocalKind::Owned))
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
        if !diverges(&alt)
            && let Some(alt_value) = alt.value.take()
        {
            alt.value = Some(Box::new(self.coerce(*alt_value, &ok)));
        }
        self.mk(
            ExprKind::Else {
                expr: Box::new(value),
                binding,
                alt: Box::new(alt),
            },
            ok,
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
            self.error_help(
                span,
                format!("the branches have different types: `{a}` and `{b}`"),
                "every branch of an `if` or `match` used for its value has the same type (Section 6.6)",
            );
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
