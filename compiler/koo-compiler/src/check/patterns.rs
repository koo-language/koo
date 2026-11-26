//! Checking `match`: patterns, arms and exhaustiveness (Section 6.6).

use std::collections::{HashMap, HashSet};

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

/// What a pattern says of a value, for finding out whether the arms of a `match` cover
/// every case: anything at all; a value made one of the ways its type has, with the
/// shapes of its parts; or one value among too many to list, as `7` is of the integers.
#[derive(Clone)]
enum Shape {
    Any,
    Made(usize, Vec<Shape>),
    One,
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
            if arm.error && !matches!(ty, Ty::Result(..) | Ty::Unknown) && self.without_errors() {
                // An arm for the errors of a function that, this time, has none.
                let reported = self.diags.len();
                self.scopes.push(HashMap::new());
                for pattern in &arm.patterns {
                    self.arm_pattern(pattern, &Ty::Unknown, true);
                }
                if let Some(guard) = &arm.guard {
                    self.expr(guard, None);
                }
                self.body(&arm.body, value, expected);
                self.scopes.pop();
                self.diags.truncate(reported);
                continue;
            }
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
                // By its own name, or by another name for it.
                Some(TypeName::Error(id) | TypeName::Alias(Ty::Error(id))) => {
                    let id = *id;
                    if !set.contains(id) {
                        // A warning in the specification, so that a library may stop
                        // returning an error; reported as an error until warnings exist.
                        let shown = checker.ty_name(&Ty::Error(id));
                        checker.error_help(
                            name.span,
                            format!("`{shown}` is not one of the errors of this result"),
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
                let several = any_binding_ty == Ty::Failure;
                let binding = self.declare(name, any_binding_ty, false, LocalKind::Owned);
                if several {
                    self.caught.insert(binding, set.clone());
                }
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
                    Some(fields) => self.field_patterns(fields, *rest, &defs, name, &[]),
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
        hidden: &[bool],
    ) -> Vec<(usize, Pattern)> {
        let hidden = |index: usize| hidden.get(index).copied().unwrap_or(false);
        let mut out: Vec<(usize, Pattern)> = Vec::new();
        for field in fields {
            let found = defs
                .iter()
                .position(|d| d.name == field.name.name)
                .filter(|index| !hidden(*index));
            let Some(index) = found else {
                self.error(
                    field.name.span,
                    format!("`{}` has no field `{}`", owner.name, field.name.name),
                );
                // What the pattern would have bound is still a name, of no known type,
                // so that the one mistake is reported once.
                match &field.pattern {
                    Some(pattern) => {
                        self.pattern(pattern, &Ty::Unknown);
                    }
                    None => {
                        self.declare(&field.name, Ty::Unknown, false, LocalKind::Owned);
                    }
                }
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
        let seen = (0..defs.len()).filter(|index| !hidden(*index)).count();
        if !rest && out.len() < seen {
            let missing: Vec<String> = defs
                .iter()
                .enumerate()
                .filter(|(i, _)| !hidden(*i) && !out.iter().any(|(j, _)| j == i))
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
        // Inside another pattern, a pattern that says what a value is, a literal or a
        // variant, matches an optional that has such a value. A name takes the optional
        // as it is, with or without a value, and `none` and `_` mean what they say.
        if let Ty::Optional(inner) = ty
            && !matches!(
                pattern.kind,
                ast::PatternKind::None | ast::PatternKind::Wildcard | ast::PatternKind::Bind { .. }
            )
        {
            return Pattern::Some(Box::new(self.pattern(pattern, inner)));
        }
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
                        // `sql.ErrorKind.Constraint`: an enum of a standard package.
                        let enum_name = match path.as_slice() {
                            [package, of, _] => match self.package_type(&[package.clone(), of.clone()]) {
                                Some(full) => Some((full, package.span)),
                                None => {
                                    self.unsupported(span, "types from other packages are");
                                    return Pattern::Wildcard;
                                }
                            },
                            [of, _] => Some((of.name.clone(), of.span)),
                            _ => None,
                        };
                        if let Some((written, at)) = enum_name
                            && written != self.program.enums[*id].name
                            && !matches!(self.type_names.get(&written), Some(TypeName::Alias(Ty::Enum(named))) if named == id)
                        {
                            let expected = self.ty_name(ty);
                            self.error(at, format!("expected a variant of `{expected}`"));
                            return Pattern::Wildcard;
                        }
                        if path.len() > 3 {
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
                            Some(fields) => self.field_patterns(fields, *rest, &defs, name, &[]),
                            None => Vec::new(),
                        };
                        Pattern::Variant { variant, fields }
                    }
                    Ty::Struct(id) => {
                        let another = matches!(
                            self.type_names.get(&name.name),
                            Some(TypeName::Alias(Ty::Struct(named))) if named == id
                        );
                        // A structure of a standard package is written with the package.
                        let written = match path.as_slice() {
                            [only] => Some(only.name.clone()),
                            [package, of] => self.package_type(&[package.clone(), of.clone()]),
                            _ => None,
                        };
                        if written.as_deref() != Some(self.program.structs[*id].name.as_str()) && !another {
                            let expected = self.ty_name(ty);
                            self.error(span, format!("expected a pattern for `{expected}`"));
                            return Pattern::Wildcard;
                        }
                        let defs = self.program.structs[*id].fields.clone();
                        // What a type of the library keeps hidden, and what a handle of
                        // an actor is made of, a pattern reaches no more than `x.field`.
                        let handle = self.handles.contains(id);
                        let library = *id < self.prelude_types.0;
                        let hidden: Vec<bool> = defs
                            .iter()
                            .map(|def| !self.body_in_prelude && (handle || (library && def.name.starts_with('_'))))
                            .collect();
                        let fields = match fields {
                            Some(fields) => self.field_patterns(fields, *rest, &defs, name, &hidden),
                            None => Vec::new(),
                        };
                        Pattern::Struct { fields }
                    }
                    // An error, where the value of a result is matched: the arm for an
                    // error begins with `!`.
                    _ if matches!(self.type_names.get(&name.name), Some(TypeName::Error(_))) => {
                        self.error_help(
                            span,
                            format!("`{}` is an error, and this arm is for a value", name.name),
                            format!("an arm for an error begins with `!`: `! {} ... => ...`", name.name),
                        );
                        Pattern::Wildcard
                    }
                    _ => mismatch(self, &format!("`{}`", name.name)),
                }
            }
            ast::PatternKind::Tuple(parts) => match ty {
                // `()` is the one value of a function that returns nothing.
                Ty::Unit if parts.is_empty() => Pattern::Wildcard,
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
        let rows: Vec<Vec<Shape>> = arms
            .iter()
            .filter(|arm| arm.guard.is_none())
            .flat_map(|arm| arm.patterns.iter())
            .flat_map(|pattern| self.shapes(pattern, ty))
            .map(|shape| vec![shape])
            .collect();
        let mut steps = 0;
        let found = self.uncovered(std::slice::from_ref(ty), &rows, 8, &mut steps);
        let missing: Vec<String> = found
            .into_iter()
            .map(|case| match case[0].as_str() {
                // Nothing to name but "anything else", whatever its parts.
                vague if vague.chars().all(|c| "_(), ".contains(c)) => {
                    "the remaining values: add a `_` arm".to_string()
                }
                plain @ ("a value" | "the success value" | "`! e` for the remaining errors") => plain.to_string(),
                written => format!("`{written}`"),
            })
            .collect();
        // Said once, though it is missing in more ways than one.
        let mut said = HashSet::new();
        let missing: Vec<String> = missing.into_iter().filter(|case| said.insert(case.clone())).collect();
        if !missing.is_empty() {
            let found = self.ty_name(ty);
            self.error(span, format!("this `match` does not cover {}", missing.join(", ")));
            self.label(format!("this is {}", super::a(&found)));
            self.note("a `match` covers every case (Section 6.6)");
            if missing.iter().any(|case| case.starts_with("`! e` for")) {
                // Any error at all may come, so only an arm for any error covers them.
                self.help("add a last arm `! e => ...`, which takes every error that the arms above it do not");
            } else if missing.iter().all(|case| case.starts_with("`! ")) {
                // Only errors are missing, and an error is matched by an arm that
                // begins with `!`.
                let first = missing[0].trim_matches('`').trim_end_matches(" { .. }");
                let first = first.strip_prefix("! ").unwrap_or(first);
                self.help(format!(
                    "add an arm for each, as `! e: {first} => ...`, or a last arm `! e => ...` for every error not named"
                ));
            } else if !missing.iter().any(|case| case.contains("`_`")) {
                self.help("add an arm for each, or a last arm `_ => ...` for every case not named");
            }
        }
    }

    /// The ways a value of a type is made, each with a name and what it is made of; or
    /// nothing for a type with too many values to list, an `int` or a `string`.
    fn makers(&self, ty: &Ty) -> Option<Vec<(String, Vec<Ty>)>> {
        let of = |fields: &[FieldDef]| fields.iter().map(|field| field.ty.clone()).collect::<Vec<Ty>>();
        Some(match ty {
            Ty::Bool => vec![("true".to_string(), Vec::new()), ("false".to_string(), Vec::new())],
            Ty::Unit => vec![("()".to_string(), Vec::new())],
            Ty::Enum(id) => self.program.enums[*id]
                .variants
                .iter()
                .map(|variant| (variant.name.clone(), of(&variant.fields)))
                .collect(),
            Ty::Optional(inner) => vec![
                ("none".to_string(), Vec::new()),
                ("a value".to_string(), vec![(**inner).clone()]),
            ],
            Ty::Result(ok, set) => {
                let mut makers = vec![("the success value".to_string(), vec![(**ok).clone()])];
                match set {
                    // Every error there is: one more case, which only `! e` covers.
                    ErrSet::Any => makers.push(("`! e` for the remaining errors".to_string(), Vec::new())),
                    ErrSet::Of(ids) => {
                        for id in ids {
                            let error = &self.program.errors[*id];
                            let name = self.ty_name(&Ty::Error(*id));
                            makers.push((format!("! {name}"), of(&error.fields)));
                        }
                    }
                }
                makers
            }
            Ty::Tuple(types) => vec![(String::new(), types.clone())],
            Ty::Struct(id) => vec![(
                self.program.structs[*id].name.clone(),
                of(&self.program.structs[*id].fields),
            )],
            _ => return None,
        })
    }

    /// A pattern as the ways of making a value that it matches. There is more than one
    /// only for `! e` on a result with a list of errors, which is each of them.
    fn shapes(&self, pattern: &Pattern, ty: &Ty) -> Vec<Shape> {
        // The patterns of the parts of a value, by their places, with anything where
        // the pattern names none.
        let parts = |checker: &Self, fields: &[(usize, Pattern)], types: &[Ty]| -> Vec<Vec<Shape>> {
            let mut made: Vec<Vec<Shape>> = vec![Vec::new()];
            for (index, ty) in types.iter().enumerate() {
                let choices = match fields.iter().find(|(at, _)| *at == index) {
                    Some((_, pattern)) => checker.shapes(pattern, ty),
                    None => vec![Shape::Any],
                };
                made = made
                    .into_iter()
                    .flat_map(|sofar| {
                        choices.iter().map(move |choice| {
                            let mut next = sofar.clone();
                            next.push(choice.clone());
                            next
                        })
                    })
                    .collect();
            }
            made
        };
        let numbered =
            |patterns: &[Pattern]| -> Vec<(usize, Pattern)> { patterns.iter().cloned().enumerate().collect() };
        let makers = self.makers(ty);
        let fields_of = |maker: usize| {
            makers
                .as_ref()
                .and_then(|makers| makers.get(maker))
                .map(|(_, types)| types.clone())
        };
        let made = |checker: &Self, maker: usize, fields: &[(usize, Pattern)]| -> Vec<Shape> {
            match fields_of(maker) {
                Some(types) => parts(checker, fields, &types)
                    .into_iter()
                    .map(|parts| Shape::Made(maker, parts))
                    .collect(),
                None => vec![Shape::One],
            }
        };
        match (pattern, ty) {
            (Pattern::Wildcard | Pattern::Bind(_), _) => vec![Shape::Any],
            (Pattern::Bool(value), _) => vec![Shape::Made(usize::from(!*value), Vec::new())],
            (Pattern::None, _) => vec![Shape::Made(0, Vec::new())],
            (Pattern::Some(inner), _) => made(self, 1, &[(0, (**inner).clone())]),
            (Pattern::Ok(inner), _) => made(self, 0, &[(0, (**inner).clone())]),
            (Pattern::Err { error, fields, .. }, Ty::Result(_, set)) => match (error, set) {
                (None, ErrSet::Any) => vec![Shape::Made(1, Vec::new())],
                (None, ErrSet::Of(ids)) => (0..ids.len()).flat_map(|at| made(self, at + 1, &[])).collect(),
                (Some(_), ErrSet::Any) => vec![Shape::One],
                (Some(error), ErrSet::Of(ids)) => match ids.iter().position(|id| id == error) {
                    Some(at) => made(self, at + 1, fields),
                    None => vec![Shape::One],
                },
            },
            (Pattern::Variant { variant, fields }, _) => made(self, *variant, fields),
            (Pattern::Struct { fields }, _) => made(self, 0, fields),
            (Pattern::Tuple(patterns), _) => made(self, 0, &numbered(patterns)),
            _ => vec![Shape::One],
        }
    }

    /// Values that no row matches: up to `limit` of them, each as the text of its
    /// parts, one for each of `tys`. A row is the shapes of an arm, a column for each
    /// type. The search gives up, and asks for a `_`, when the cases are too many.
    fn uncovered(&self, tys: &[Ty], rows: &[Vec<Shape>], limit: usize, steps: &mut usize) -> Vec<Vec<String>> {
        *steps += 1;
        if limit == 0 {
            return Vec::new();
        }
        if *steps > 20_000 {
            return vec![vec!["_".to_string(); tys.len()]];
        }
        let Some((first, rest)) = tys.split_first() else {
            return if rows.is_empty() { vec![Vec::new()] } else { Vec::new() };
        };
        let after = |row: &Vec<Shape>| row[1..].to_vec();
        let Some(makers) = self.makers(first) else {
            // Too many values to name: only an arm that takes anything covers them.
            let rows: Vec<Vec<Shape>> = rows
                .iter()
                .filter(|row| matches!(row[0], Shape::Any))
                .map(after)
                .collect();
            return self
                .uncovered(rest, &rows, limit, steps)
                .into_iter()
                .map(|mut case| {
                    case.insert(0, "_".to_string());
                    case
                })
                .collect();
        };
        let mut found = Vec::new();
        for (maker, (name, parts)) in makers.iter().enumerate() {
            let rows: Vec<Vec<Shape>> = rows
                .iter()
                .filter_map(|row| match &row[0] {
                    Shape::Any => Some([vec![Shape::Any; parts.len()], after(row)].concat()),
                    Shape::Made(by, inner) if *by == maker => Some([inner.clone(), after(row)].concat()),
                    _ => None,
                })
                .collect();
            // A way of making the value that no arm names at all is missing whole: it
            // is the case, and not each of the values its parts may have.
            let named = matches!(first, Ty::Bool | Ty::Enum(_) | Ty::Optional(_) | Ty::Result(..));
            if named && rows.is_empty() {
                found.push([vec![name.clone()], vec!["_".to_string(); rest.len()]].concat());
                if found.len() == limit {
                    return found;
                }
                continue;
            }
            let tys: Vec<Ty> = parts.iter().chain(rest).cloned().collect();
            for case in self.uncovered(&tys, &rows, limit - found.len(), steps) {
                let (inner, tail) = case.split_at(parts.len());
                let plain = inner.iter().all(|part| part == "_");
                let shown = match first {
                    Ty::Tuple(_) => format!("({})", inner.join(", ")),
                    // An error some of whose values are matched, and not the rest.
                    Ty::Result(..) if !plain && maker > 0 => format!("{name} {{ .. }}"),
                    Ty::Optional(_) | Ty::Result(..) if !plain => inner[0].clone(),
                    Ty::Struct(_) if plain => "_".to_string(),
                    Ty::Enum(_) | Ty::Struct(_) if !plain => format!("{name} {{ .. }}"),
                    _ => name.clone(),
                };
                found.push([vec![shown], tail.to_vec()].concat());
                if found.len() == limit {
                    return found;
                }
            }
        }
        found
    }
}
