//! Checking `match`: patterns, arms and exhaustiveness (Section 6.6).

use std::collections::HashMap;

use koo_syntax::Span;
use koo_syntax::ast;

use super::exprs::peel;
use super::{Checker, TypeName, diverges};
use crate::hir::*;

fn binds(pattern: &Pattern) -> bool {
    match pattern {
        Pattern::Bind(_) => true,
        Pattern::Some(inner) | Pattern::Ok(inner) => binds(inner),
        Pattern::Err { binding, fields, .. } => binding.is_some() || fields.iter().any(|(_, p)| binds(p)),
        Pattern::Variant { fields, .. } | Pattern::Struct { fields } => fields.iter().any(|(_, p)| binds(p)),
        Pattern::Tuple(parts) => parts.iter().any(binds),
        _ => false,
    }
}

/// Whether a pattern matches every value of the type it is written for.
fn irrefutable(pattern: &Pattern) -> bool {
    match pattern {
        Pattern::Wildcard | Pattern::Bind(_) => true,
        Pattern::Struct { fields } => fields.iter().all(|(_, p)| irrefutable(p)),
        Pattern::Tuple(parts) => parts.iter().all(irrefutable),
        _ => false,
    }
}

impl<'a> Checker<'a> {
    pub(super) fn match_expr(
        &mut self,
        scrutinee: &ast::Expr,
        arms: &[ast::MatchArm],
        span: Span,
        value: bool,
        expected: Option<&Ty>,
    ) -> Expr {
        let scrutinee = self.expr(scrutinee, None);
        let ty = scrutinee.ty.clone();
        let mut checked: Vec<Arm> = Vec::new();
        for arm in arms {
            self.scopes.push(HashMap::new());
            let mut patterns = Vec::new();
            for pattern in &arm.patterns {
                patterns.push(self.arm_pattern(pattern, &ty, arm.error));
            }
            if patterns.len() > 1 && patterns.iter().any(binds) {
                self.unsupported(arm.span, "alternatives (`|`) that bind names are");
            }
            let guard = arm.guard.as_ref().map(|guard| self.expr_as(guard, &Ty::Bool));
            let body = self.body(&arm.body, value, expected);
            if !value
                && let Some(discarded) = &body.value
                && matches!(discarded.ty, Ty::Result(..))
            {
                self.unhandled_result(discarded, discarded.span);
            }
            self.scopes.pop();
            checked.push(Arm { patterns, guard, body });
        }
        if !ty.is_unknown() {
            self.check_exhaustive(&ty, &checked, scrutinee.span);
        }
        let result = if value {
            let mut bodies: Vec<&mut Block> = checked.iter_mut().map(|arm| &mut arm.body).collect();
            self.unify(&mut bodies, expected, span)
        } else if !checked.is_empty() && checked.iter().all(|arm| diverges(&arm.body)) {
            Ty::Never
        } else {
            Ty::Unit
        };
        Expr {
            kind: ExprKind::Match {
                scrutinee: Box::new(scrutinee),
                arms: checked,
            },
            ty: result,
            span,
        }
    }

    /// A pattern at the top of an arm. On an optional the arms are `none` and a pattern for
    /// the value; on a result they are patterns for the success value and, after `!`,
    /// patterns for the errors.
    fn arm_pattern(&mut self, pattern: &ast::Pattern, ty: &Ty, error: bool) -> Pattern {
        match ty {
            Ty::Result(ok, set) => {
                if error {
                    self.error_pattern(pattern, set)
                } else {
                    Pattern::Ok(Box::new(self.pattern(pattern, ok)))
                }
            }
            _ if error => {
                if !ty.is_unknown() {
                    let found = self.ty_name(ty);
                    self.error_help(
                        pattern.span,
                        format!("an error arm in a `match` on `{found}`"),
                        "`!` begins an arm for an error, and only a result has errors (Section 6.4)",
                    );
                }
                // What the arm names is still a name, so that its uses are not reported.
                if let ast::PatternKind::Bind { name, .. } = &pattern.kind {
                    self.declare(name, Ty::Unknown, false, LocalKind::Owned);
                }
                Pattern::Wildcard
            }
            Ty::Optional(inner) => match &pattern.kind {
                ast::PatternKind::None => Pattern::None,
                ast::PatternKind::Wildcard => Pattern::Wildcard,
                _ => Pattern::Some(Box::new(self.pattern(pattern, inner))),
            },
            _ => self.pattern(pattern, ty),
        }
    }

    fn error_pattern(&mut self, pattern: &ast::Pattern, set: &ErrSet) -> Pattern {
        // `http.Error`, the error of a standard package, by the prelude's name for it.
        let in_package = |checker: &mut Checker<'_>, path: &[ast::Ident]| {
            checker.package_type(path).map(|name| ast::Ident {
                name,
                span: path[1].span,
            })
        };
        let qualified = match &pattern.kind {
            ast::PatternKind::Ctor { path, .. } if path.len() == 2 => in_package(self, path),
            ast::PatternKind::Bind { ty: Some(ty), .. } => match &ty.kind {
                ast::TypeKind::Named { path, args } if path.len() == 2 && args.is_empty() => in_package(self, path),
                _ => None,
            },
            _ => None,
        };
        let any_binding_ty = match set {
            ErrSet::Of(ids) if ids.len() == 1 => Ty::Error(ids[0]),
            _ => Ty::Failure,
        };
        let lookup = |checker: &mut Checker<'_>, name: &ast::Ident| -> Option<ErrorId> {
            match checker.type_names.get(&name.name) {
                Some(TypeName::Error(id)) => {
                    let id = *id;
                    if !set.contains(id) {
                        // A warning in the specification, so that a library may stop
                        // returning an error; reported as an error until warnings exist.
                        checker.error_help(
                            name.span,
                            format!("`{}` is not one of the errors of this result", name.name),
                            "remove the arm",
                        );
                    }
                    Some(id)
                }
                _ => {
                    checker.error(name.span, format!("`{}` is not an error type", name.name));
                    None
                }
            }
        };
        match &pattern.kind {
            ast::PatternKind::Wildcard => Pattern::Err {
                error: None,
                binding: None,
                fields: Vec::new(),
            },
            ast::PatternKind::Bind { name, ty: None } => {
                let binding = self.declare(name, any_binding_ty, false, LocalKind::Owned);
                Pattern::Err {
                    error: None,
                    binding: Some(binding),
                    fields: Vec::new(),
                }
            }
            ast::PatternKind::Bind { name, ty: Some(ty) } => {
                let error = match &ty.kind {
                    _ if qualified.is_some() => lookup(self, qualified.as_ref().unwrap()),
                    ast::TypeKind::Named { path, args } if path.len() == 1 && args.is_empty() => lookup(self, &path[0]),
                    _ => {
                        self.error(ty.span, "expected an error type");
                        None
                    }
                };
                let binding_ty = error.map(Ty::Error).unwrap_or(Ty::Unknown);
                let binding = self.declare(name, binding_ty, false, LocalKind::Owned);
                Pattern::Err {
                    error,
                    binding: Some(binding),
                    fields: Vec::new(),
                }
            }
            ast::PatternKind::Ctor { path, fields, rest } if path.len() == 1 || qualified.is_some() => {
                let name = qualified.as_ref().unwrap_or(&path[0]);
                let Some(id) = lookup(self, name) else {
                    return Pattern::Wildcard;
                };
                let defs = self.program.errors[id].fields.clone();
                let fields = match fields {
                    Some(fields) => self.field_patterns(fields, *rest, &defs, name),
                    None => Vec::new(),
                };
                Pattern::Err {
                    error: Some(id),
                    binding: None,
                    fields,
                }
            }
            _ => {
                self.error_help(
                    pattern.span,
                    "this is not a pattern for an error",
                    "after `!` write an error type, `name: ErrorType`, or a name for any error (Section 6.4)",
                );
                Pattern::Wildcard
            }
        }
    }

    fn field_patterns(
        &mut self,
        fields: &[ast::FieldPattern],
        rest: bool,
        defs: &[FieldDef],
        owner: &ast::Ident,
    ) -> Vec<(usize, Pattern)> {
        let mut out: Vec<(usize, Pattern)> = Vec::new();
        for field in fields {
            let Some(index) = defs.iter().position(|d| d.name == field.name.name) else {
                self.error(
                    field.name.span,
                    format!("`{}` has no field `{}`", owner.name, field.name.name),
                );
                continue;
            };
            if out.iter().any(|(i, _)| *i == index) {
                self.error(
                    field.name.span,
                    format!("the field `{}` appears twice", field.name.name),
                );
                continue;
            }
            let ty = defs[index].ty.clone();
            let pattern = match &field.pattern {
                Some(pattern) => self.pattern(pattern, &ty),
                None => Pattern::Bind(self.declare(&field.name, ty, false, LocalKind::Owned)),
            };
            out.push((index, pattern));
        }
        if !rest && out.len() < defs.len() {
            let missing: Vec<String> = defs
                .iter()
                .enumerate()
                .filter(|(i, _)| !out.iter().any(|(j, _)| j == i))
                .map(|(_, d)| format!("`{}`", d.name))
                .collect();
            self.error_help(
                owner.span,
                format!("this pattern does not name {}", missing.join(", ")),
                "name every field, or end the pattern with `..` (Section 6.6)",
            );
        }
        out
    }

    fn pattern(&mut self, pattern: &ast::Pattern, ty: &Ty) -> Pattern {
        let span = pattern.span;
        let mismatch = |checker: &mut Checker<'_>, what: &str| {
            if !ty.is_unknown() {
                let found = checker.ty_name(ty);
                checker.error(span, format!("{what} cannot match a value of type `{found}`"));
            }
            Pattern::Wildcard
        };
        match &pattern.kind {
            ast::PatternKind::Wildcard => Pattern::Wildcard,
            ast::PatternKind::None => match ty {
                Ty::Optional(_) => Pattern::None,
                _ => mismatch(self, "`none`"),
            },
            ast::PatternKind::Bind { name, ty: None } => {
                Pattern::Bind(self.declare(name, ty.clone(), false, LocalKind::Owned))
            }
            ast::PatternKind::Bind {
                ty: Some(annotation), ..
            } => {
                self.unsupported(annotation.span, "type tests in patterns are");
                Pattern::Wildcard
            }
            ast::PatternKind::Literal(literal) => {
                let value = self.expr(literal, Some(ty));
                match (&value.kind, &value.ty, peel(ty)) {
                    (_, Ty::Unknown, _) => Pattern::Wildcard,
                    (ExprKind::Int(n), _, Ty::Int | Ty::Byte) if value.ty == *ty => Pattern::Int(*n),
                    (ExprKind::Bool(b), _, Ty::Bool) if *ty == Ty::Bool => Pattern::Bool(*b),
                    (ExprKind::Str(s), _, Ty::Str) if *ty == Ty::Str => Pattern::Str(s.clone()),
                    (ExprKind::Float(_), _, _) => {
                        self.error_help(
                            span,
                            "a float cannot be matched exactly",
                            "compare it in a guard: `x if x < 0.5 => ...`",
                        );
                        Pattern::Wildcard
                    }
                    _ => {
                        let found = self.ty_name(&value.ty);
                        mismatch(self, &format!("a literal of type `{found}`"))
                    }
                }
            }
            ast::PatternKind::Range { lo, hi, inclusive } => {
                let lo = self.expr(lo, Some(ty));
                let hi = self.expr(hi, Some(ty));
                match (&lo.kind, &hi.kind, ty) {
                    (ExprKind::Int(lo), ExprKind::Int(hi), Ty::Int | Ty::Byte) => {
                        let hi = if *inclusive { *hi } else { hi.wrapping_sub(1) };
                        if *lo > hi {
                            self.error(span, "this range is empty");
                        }
                        Pattern::IntRange(*lo, hi)
                    }
                    _ => mismatch(self, "a range of integers"),
                }
            }
            ast::PatternKind::Ctor { path, fields, rest } => {
                let name = path.last().unwrap();
                match ty {
                    Ty::Enum(id) => {
                        if path.len() == 2 && path[0].name != self.program.enums[*id].name {
                            let expected = self.program.enums[*id].name.clone();
                            self.error(path[0].span, format!("expected a variant of `{expected}`"));
                            return Pattern::Wildcard;
                        }
                        if path.len() > 2 {
                            self.unsupported(span, "types from other packages are");
                            return Pattern::Wildcard;
                        }
                        let Some(variant) = self.program.enums[*id]
                            .variants
                            .iter()
                            .position(|v| v.name == name.name)
                        else {
                            let expected = self.program.enums[*id].name.clone();
                            let known = self.program.enums[*id]
                                .variants
                                .iter()
                                .map(|v| v.name.clone())
                                .collect();
                            let message = format!("`{expected}` has no variant `{}`", name.name);
                            self.error_near(name.span, message, &name.name, known, None);
                            return Pattern::Wildcard;
                        };
                        let defs = self.program.enums[*id].variants[variant].fields.clone();
                        // A bare `Name` matches the variant whatever its fields.
                        let fields = match fields {
                            Some(fields) => self.field_patterns(fields, *rest, &defs, name),
                            None => Vec::new(),
                        };
                        Pattern::Variant { variant, fields }
                    }
                    Ty::Struct(id) => {
                        if path.len() != 1 || name.name != self.program.structs[*id].name {
                            let expected = self.program.structs[*id].name.clone();
                            self.error(span, format!("expected a pattern for `{expected}`"));
                            return Pattern::Wildcard;
                        }
                        let defs = self.program.structs[*id].fields.clone();
                        let fields = match fields {
                            Some(fields) => self.field_patterns(fields, *rest, &defs, name),
                            None => Vec::new(),
                        };
                        Pattern::Struct { fields }
                    }
                    _ => mismatch(self, &format!("`{}`", name.name)),
                }
            }
            ast::PatternKind::Tuple(parts) => match ty {
                Ty::Tuple(types) if types.len() == parts.len() => {
                    let types = types.clone();
                    Pattern::Tuple(
                        parts
                            .iter()
                            .zip(&types)
                            .map(|(part, ty)| self.pattern(part, ty))
                            .collect(),
                    )
                }
                _ => mismatch(self, &format!("a tuple of {}", parts.len())),
            },
        }
    }

    fn check_exhaustive(&mut self, ty: &Ty, arms: &[Arm], span: Span) {
        // Only arms without a guard are certain to match.
        let patterns: Vec<&Pattern> = arms
            .iter()
            .filter(|arm| arm.guard.is_none())
            .flat_map(|arm| arm.patterns.iter())
            .collect();
        if patterns.iter().any(|p| irrefutable(p)) {
            return;
        }
        let mut missing: Vec<String> = Vec::new();
        match ty {
            Ty::Enum(id) => {
                for (index, variant) in self.program.enums[*id].variants.iter().enumerate() {
                    let covered = patterns.iter().any(|p| match p {
                        Pattern::Variant { variant, fields } => {
                            *variant == index && fields.iter().all(|(_, p)| irrefutable(p))
                        }
                        _ => false,
                    });
                    if !covered {
                        missing.push(format!("`{}`", variant.name));
                    }
                }
            }
            Ty::Optional(_) => {
                if !patterns.iter().any(|p| matches!(p, Pattern::None)) {
                    missing.push("`none`".to_string());
                }
                if !patterns
                    .iter()
                    .any(|p| matches!(p, Pattern::Some(inner) if irrefutable(inner)))
                {
                    missing.push("a value".to_string());
                }
            }
            Ty::Result(_, set) => {
                if !patterns
                    .iter()
                    .any(|p| matches!(p, Pattern::Ok(inner) if irrefutable(inner)))
                {
                    missing.push("the success value".to_string());
                }
                let any = patterns.iter().any(|p| matches!(p, Pattern::Err { error: None, .. }));
                if !any {
                    match set {
                        ErrSet::Any => missing.push("`! e` for the remaining errors".to_string()),
                        ErrSet::Of(ids) => {
                            for id in ids {
                                let covered = patterns.iter().any(|p| match p {
                                    Pattern::Err {
                                        error: Some(error),
                                        fields,
                                        ..
                                    } => error == id && fields.iter().all(|(_, p)| irrefutable(p)),
                                    _ => false,
                                });
                                if !covered {
                                    missing.push(format!("`! {}`", self.program.errors[*id].name));
                                }
                            }
                        }
                    }
                }
            }
            Ty::Bool => {
                for value in [true, false] {
                    if !patterns.iter().any(|p| matches!(p, Pattern::Bool(b) if *b == value)) {
                        missing.push(format!("`{value}`"));
                    }
                }
            }
            _ => missing.push("the remaining values: add a `_` arm".to_string()),
        }
        if !missing.is_empty() {
            let found = self.ty_name(ty);
            self.error(span, format!("this `match` does not cover {}", missing.join(", ")));
            self.label(format!("this is {}", super::a(&found)));
            self.note("a `match` covers every case (Section 6.6)");
            if !missing.iter().any(|case| case.contains("`_`")) {
                self.help("add an arm for each, or a last arm `_ => ...` for every case not named");
            }
        }
    }
}
