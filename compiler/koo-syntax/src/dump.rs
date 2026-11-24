//! A compact, stable text form of the syntax tree, for tests and for `koo parse --tree`.
//!
//! Expressions and statements are printed as S-expressions, so that grouping is explicit:
//! `a + b * c` prints as `(+ a (* b c))`. Types and patterns are printed in Koo syntax.

use crate::ast::*;

pub fn file(file: &File) -> String {
    let mut out = String::new();
    for import in &file.imports {
        match &import.kind {
            ImportKind::Package { path, major } => {
                out.push_str(&format!("(import {path}"));
                if let Some(major) = major {
                    out.push_str(&format!(" v{major}"));
                }
            }
            ImportKind::C { header, link } => {
                out.push_str(&format!("(import-c {header:?}"));
                if let Some(link) = link {
                    out.push_str(&format!(" link {link:?}"));
                }
            }
        }
        out.push_str(&format!(" as {})\n", import.binding));
    }
    for item in &file.items {
        out.push_str(&self::item(item));
        out.push('\n');
    }
    out
}

fn join<T>(items: &[T], sep: &str, f: impl Fn(&T) -> String) -> String {
    items.iter().map(f).collect::<Vec<_>>().join(sep)
}

pub fn item(item: &Item) -> String {
    let mut out = String::new();
    for attr in &item.attrs {
        out.push_str(&format!("@{} ", attr.name.name));
    }
    if item.public {
        out.push_str("pub ");
    }
    out.push_str(&match &item.kind {
        ItemKind::Type(d) => {
            let word = if d.resource { "resource" } else { "type" };
            let body = match &d.body {
                TypeBody::Struct(fs) => fields(fs),
                TypeBody::Distinct(t) => format!("distinct {}", ty(t)),
                TypeBody::Alias(t) => ty(t),
            };
            format!(
                "({word} {}{}{} {body})",
                d.name.name,
                type_params(&d.type_params),
                satisfies(&d.satisfies)
            )
        }
        ItemKind::Enum(d) => {
            let variants = join(&d.variants, " ", |v| match &v.fields {
                Some(fs) => format!("{} {}", v.name.name, fields(fs)),
                None => v.name.name.clone(),
            });
            format!(
                "(enum {}{}{} [{variants}])",
                d.name.name,
                type_params(&d.type_params),
                satisfies(&d.satisfies)
            )
        }
        ItemKind::Error(d) => match &d.body {
            ErrorBody::Fields(fs) => format!("(error {} {})", d.name.name, fields(fs)),
            ErrorBody::Set(types) => {
                format!("(error {} = {})", d.name.name, join(types, " | ", ty))
            }
        },
        ItemKind::Interface(d) => {
            let word = if d.resource { "resource-interface" } else { "interface" };
            let members = join(&d.members, " ", |m| format!("({})", sig(&m.sig)));
            format!("({word} {}{} [{members}])", d.name.name, type_params(&d.type_params))
        }
        ItemKind::Actor(d) => {
            let fs = join(&d.fields, ", ", |f| {
                let var = if f.mutable { "var " } else { "" };
                let default = f
                    .default
                    .as_ref()
                    .map(|e| format!(" = {}", expr(e)))
                    .unwrap_or_default();
                format!("{var}{}: {}{default}", f.name.name, ty(&f.ty))
            });
            let methods = join(&d.methods, " ", |m| {
                let public = if m.public { "pub " } else { "" };
                format!("{public}{}", fn_decl(&m.decl))
            });
            format!(
                "(actor {}{}{} {{{fs}}} [{methods}])",
                d.name.name,
                type_params(&d.type_params),
                satisfies(&d.satisfies)
            )
        }
        ItemKind::Fn(d) => fn_decl(d),
        ItemKind::Const(d) => match &d.ty {
            Some(t) => format!("(const {}: {} {})", d.name.name, ty(t), expr(&d.value)),
            None => format!("(const {} {})", d.name.name, expr(&d.value)),
        },
        ItemKind::Stmt(s) => stmt(s),
    });
    out
}

fn satisfies(types: &[Type]) -> String {
    if types.is_empty() {
        String::new()
    } else {
        format!(": {}", join(types, " + ", ty))
    }
}

fn type_params(params: &[TypeParam]) -> String {
    if params.is_empty() {
        return String::new();
    }
    let inner = join(params, ", ", |p| match &p.constraint {
        Constraint::None => p.name.name.clone(),
        Constraint::Resource => format!("{}: resource", p.name.name),
        Constraint::Interfaces(types) => format!("{}: {}", p.name.name, join(types, " + ", ty)),
    });
    format!("<{inner}>")
}

fn fields(fields: &[Field]) -> String {
    let inner = join(fields, ", ", |f| {
        let attrs = join(&f.attrs, "", |a| format!(" @{}", a.name.name));
        let default = f
            .default
            .as_ref()
            .map(|e| format!(" = {}", expr(e)))
            .unwrap_or_default();
        format!("{}: {}{attrs}{default}", f.name.name, ty(&f.ty))
    });
    format!("{{{inner}}}")
}

fn kind(kind: ParamKind) -> &'static str {
    match kind {
        ParamKind::Borrow => "",
        ParamKind::Var => "var ",
        ParamKind::Take => "take ",
    }
}

fn sig(sig: &FnSig) -> String {
    let target = match &sig.target {
        FnTarget::Free => String::new(),
        FnTarget::Method { kind: k, ty: t } => {
            let t = t.as_ref().map(ty).unwrap_or_default();
            format!("(self: {}{}) ", kind(*k), t).replace(" )", ")")
        }
        FnTarget::TypeLevel(t) => format!("{}.", ty(t)),
    };
    let params = join(&sig.params, ", ", |p| {
        let dots = if p.variadic { "..." } else { "" };
        let default = p
            .default
            .as_ref()
            .map(|e| format!(" = {}", expr(e)))
            .unwrap_or_default();
        format!("{}: {dots}{}{}{default}", p.name.name, kind(p.kind), ty(&p.ty))
    });
    let ret = sig
        .ret
        .as_ref()
        .map(|t| format!(" -> {}", ret_ty(t)))
        .unwrap_or_default();
    let errors = match &sig.errors {
        ErrorSpec::None => String::new(),
        ErrorSpec::Inferred => " !".to_string(),
        ErrorSpec::Set(types) => format!(" ! {}", join(types, " | ", ty)),
    };
    format!(
        "fn {target}{}{}({params}){ret}{errors}",
        sig.name.name,
        type_params(&sig.type_params)
    )
}

fn fn_decl(decl: &FnDecl) -> String {
    match &decl.body {
        Some(body) => format!("({} {})", sig(&decl.sig), block(body)),
        None => format!("({})", sig(&decl.sig)),
    }
}

/// A return type. A function or result type is parenthesized, so that the `!` after it
/// is seen to belong to the function around it.
fn ret_ty(t: &Type) -> String {
    match &t.kind {
        TypeKind::Fn { .. } | TypeKind::Result { .. } => format!("({})", ty(t)),
        _ => ty(t),
    }
}

pub fn ty(t: &Type) -> String {
    match &t.kind {
        TypeKind::Named { path, args } => {
            let mut out = String::new();
            let mut args_done = args.is_empty();
            for (i, id) in path.iter().enumerate() {
                if i > 0 {
                    out.push('.');
                }
                out.push_str(&id.name);
                // The arguments belong to the first type name: the first segment, or the
                // second when the first is a package.
                if !args_done && (id.is_upper() || i > 0) {
                    out.push_str(&format!("<{}>", join(args, ", ", ty)));
                    args_done = true;
                }
            }
            out
        }
        TypeKind::List(e) => format!("[{}]", ty(e)),
        TypeKind::Map(k, v) => format!("[{}: {}]", ty(k), ty(v)),
        TypeKind::Array(e, n) => format!("[{}; {}]", ty(e), expr(n)),
        TypeKind::Tuple(types) => format!("({})", join(types, ", ", ty)),
        TypeKind::Optional(inner) => match &inner.kind {
            TypeKind::Fn { .. } | TypeKind::Result { .. } => format!("({})?", ty(inner)),
            _ => format!("{}?", ty(inner)),
        },
        TypeKind::Fn {
            resource,
            params,
            ret,
            errors,
        } => {
            let resource = if *resource { "resource " } else { "" };
            let params = join(params, ", ", |(k, t)| format!("{}{}", kind(*k), ty(t)));
            let ret = ret.as_ref().map(|t| format!(" -> {}", ret_ty(t))).unwrap_or_default();
            let errors = if errors.is_empty() {
                String::new()
            } else {
                format!(" ! {}", join(errors, " | ", ty))
            };
            format!("{resource}fn({params}){ret}{errors}")
        }
        TypeKind::Struct(fs) => fields(fs),
        TypeKind::Result { ok, errors } => {
            let ok = match &ok.kind {
                TypeKind::Fn { .. } | TypeKind::Result { .. } => format!("({})", ty(ok)),
                _ => ty(ok),
            };
            format!("{ok} ! {}", join(errors, " | ", ty))
        }
    }
}

pub fn block(b: &Block) -> String {
    if b.stmts.is_empty() {
        "{}".to_string()
    } else {
        format!("{{ {} }}", join(&b.stmts, "; ", stmt))
    }
}

fn body(b: &Body) -> String {
    match b {
        Body::Block(b) => block(b),
        Body::Stmt(s) => stmt(s),
    }
}

fn bind(p: &BindPattern) -> String {
    match p {
        BindPattern::Name(id) => id.name.clone(),
        BindPattern::Wildcard(_) => "_".to_string(),
        BindPattern::Tuple(parts, _) => format!("({})", join(parts, ", ", bind)),
    }
}

fn cond(c: &Cond) -> String {
    match c {
        Cond::Expr(e) => expr(e),
        Cond::Bind { pattern, value } => format!("(bind {} {})", bind(pattern), expr(value)),
    }
}

fn label(l: &Option<Ident>) -> String {
    l.as_ref().map(|l| format!("{}: ", l.name)).unwrap_or_default()
}

pub fn stmt(s: &Stmt) -> String {
    match &s.kind {
        StmtKind::Let {
            mutable,
            pattern,
            ty: t,
            value,
        } => {
            let word = if *mutable { "var" } else { "let" };
            let t = t.as_ref().map(|t| format!(": {}", ty(t))).unwrap_or_default();
            format!("({word} {}{t} {})", bind(pattern), expr(value))
        }
        StmtKind::Assign { target, op, value } => {
            let op = match op {
                AssignOp::Set => "=",
                AssignOp::Add => "+=",
                AssignOp::Sub => "-=",
                AssignOp::Mul => "*=",
                AssignOp::Div => "/=",
            };
            format!("({op} {} {})", expr(target), expr(value))
        }
        StmtKind::Expr(e) => expr(e),
        StmtKind::Return(None) => "(return)".to_string(),
        StmtKind::Return(Some(e)) => format!("(return {})", expr(e)),
        StmtKind::Fail(e) => format!("(fail {})", expr(e)),
        StmtKind::Break(None) => "(break)".to_string(),
        StmtKind::Break(Some(l)) => format!("(break {})", l.name),
        StmtKind::Continue(None) => "(continue)".to_string(),
        StmtKind::Continue(Some(l)) => format!("(continue {})", l.name),
        StmtKind::While {
            label: l,
            cond: c,
            body,
        } => format!("({}while {} {})", label(l), cond(c), block(body)),
        StmtKind::For {
            label: l,
            pattern,
            take,
            iter,
            body,
        } => {
            let take = if *take { "take " } else { "" };
            format!(
                "({}for {} in {take}{} {})",
                label(l),
                bind(pattern),
                expr(iter),
                block(body)
            )
        }
        StmtKind::Scope(b) => format!("(scope {})", block(b)),
        StmtKind::Spawn(b) => format!("(spawn {})", block(b)),
        StmtKind::Defer(b) => format!("(defer {})", block(b)),
        StmtKind::Unsafe(b) => format!("(unsafe {})", block(b)),
        StmtKind::Select(arms) => {
            let arms = join(arms, " ", |arm| {
                let op = match &arm.op {
                    SelectOp::After(e) => format!("after {}", expr(e)),
                    SelectOp::Bind { pattern, value } => {
                        format!("{} := {}", bind(pattern), expr(value))
                    }
                    SelectOp::Expr(e) => expr(e),
                };
                format!("({op} => {})", body(&arm.body))
            });
            format!("(select {arms})")
        }
    }
}

pub fn pattern(p: &Pattern) -> String {
    match &p.kind {
        PatternKind::Wildcard => "_".to_string(),
        PatternKind::None => "none".to_string(),
        PatternKind::Literal(e) => expr(e),
        PatternKind::Range { lo, hi, inclusive } => {
            format!("{}{}{}", expr(lo), if *inclusive { "..=" } else { ".." }, expr(hi))
        }
        PatternKind::Bind { name, ty: None } => name.name.clone(),
        PatternKind::Bind { name, ty: Some(t) } => format!("{}: {}", name.name, ty(t)),
        PatternKind::Ctor { path, fields, rest } => {
            let mut out = join(path, ".", |id| id.name.clone());
            if let Some(fields) = fields {
                let mut parts: Vec<String> = fields
                    .iter()
                    .map(|f| match &f.pattern {
                        Some(p) => format!("{}: {}", f.name.name, pattern(p)),
                        None => f.name.name.clone(),
                    })
                    .collect();
                if *rest {
                    parts.push("..".to_string());
                }
                out.push_str(&format!(" {{{}}}", parts.join(", ")));
            }
            out
        }
        PatternKind::Tuple(parts) => format!("({})", join(parts, ", ", pattern)),
    }
}

fn if_expr(i: &IfExpr) -> String {
    let mut out = format!("(if {} {}", cond(&i.cond), block(&i.then));
    match &i.otherwise {
        Some(ElseBranch::Block(b)) => out.push_str(&format!(" {}", block(b))),
        Some(ElseBranch::If(next)) => out.push_str(&format!(" {}", if_expr(next))),
        None => {}
    }
    out.push(')');
    out
}

pub fn expr(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Int(s) | ExprKind::Float(s) | ExprKind::Duration(s) => s.clone(),
        ExprKind::Decimal(s) => format!("{s}d"),
        ExprKind::Char(c) => format!("{c:?}"),
        ExprKind::Str(parts) => {
            if parts.iter().all(|p| matches!(p, StrPart::Lit(_))) {
                let text: String = parts
                    .iter()
                    .map(|p| if let StrPart::Lit(s) = p { s.as_str() } else { "" })
                    .collect();
                return format!("{text:?}");
            }
            let parts = join(parts, " ", |p| match p {
                StrPart::Lit(s) => format!("{s:?}"),
                StrPart::Interp { expr: e, format: None } => expr(e),
                StrPart::Interp {
                    expr: e,
                    format: Some(f),
                } => format!("(fmt {} {f:?})", expr(e)),
            });
            format!("(str {parts})")
        }
        ExprKind::RawStr(s) => format!("(raw {s:?})"),
        ExprKind::Bool(b) => b.to_string(),
        ExprKind::None => "none".to_string(),
        ExprKind::Name(id) => id.name.clone(),
        ExprKind::TypeApp { base, args } => format!("{}<{}>", expr(base), join(args, ", ", ty)),
        ExprKind::Field { base, name } => format!("(. {} {})", expr(base), name.name),
        ExprKind::Call { callee, args } => {
            let mut out = format!("(call {}", expr(callee));
            for arg in args {
                out.push(' ');
                if let Some(name) = &arg.name {
                    out.push_str(&format!("{}: ", name.name));
                }
                if arg.var {
                    out.push_str("var ");
                }
                out.push_str(&expr(&arg.value));
            }
            out.push(')');
            out
        }
        ExprKind::Index { base, index } => format!("(index {} {})", expr(base), expr(index)),
        ExprKind::Try(inner) => format!("(? {})", expr(inner)),
        ExprKind::Unary {
            op: UnaryOp::Neg,
            expr: inner,
        } => format!("(- {})", expr(inner)),
        ExprKind::Unary {
            op: UnaryOp::Not,
            expr: inner,
        } => format!("(not {})", expr(inner)),
        ExprKind::Binary { op, lhs, rhs } => {
            format!("({} {} {})", op.as_str(), expr(lhs), expr(rhs))
        }
        ExprKind::Is { expr: inner, ty: t } => format!("(is {} {})", expr(inner), ty(t)),
        ExprKind::Else {
            expr: inner,
            binding,
            alt,
        } => match binding {
            Some(name) => format!("(else {} {} => {})", expr(inner), name.name, body(alt)),
            None => format!("(else {} {})", expr(inner), body(alt)),
        },
        ExprKind::List(items) => format!("[{}]", join(items, " ", expr)),
        ExprKind::Map(entries) => {
            if entries.is_empty() {
                "[:]".to_string()
            } else {
                format!("[{}]", join(entries, " ", |(k, v)| format!("{}: {}", expr(k), expr(v))))
            }
        }
        ExprKind::Tuple(items) => {
            if items.is_empty() {
                "()".to_string()
            } else {
                format!("(tuple {})", join(items, " ", expr))
            }
        }
        ExprKind::Paren(inner) => expr(inner),
        ExprKind::StructLit { ty: t, fields } => {
            let mut out = format!("(new {}", expr(t));
            for field in fields {
                match &field.value {
                    Some(value) => out.push_str(&format!(" {}: {}", field.name.name, expr(value))),
                    None => out.push_str(&format!(" {}", field.name.name)),
                }
            }
            out.push(')');
            out
        }
        ExprKind::Lambda { params, body: b } => {
            let params = join(params, ", ", |p| {
                let name = p.name.as_ref().map(|n| n.name.as_str()).unwrap_or("_");
                match &p.ty {
                    Some(t) => format!("{name}: {}{}", kind(p.kind), ty(t)),
                    None => name.to_string(),
                }
            });
            format!("(fn ({params}) {})", body(b))
        }
        ExprKind::If(i) => if_expr(i),
        ExprKind::Match { scrutinee, arms } => {
            let arms = join(arms, " ", |arm| {
                let bang = if arm.error { "! " } else { "" };
                let guard = arm
                    .guard
                    .as_ref()
                    .map(|g| format!(" if {}", expr(g)))
                    .unwrap_or_default();
                format!(
                    "({bang}{}{guard} => {})",
                    join(&arm.patterns, " | ", pattern),
                    body(&arm.body)
                )
            });
            format!("(match {} {arms})", expr(scrutinee))
        }
    }
}
