//! Lambdas and function values (Sections 4.11 and 5.7).
//!
//! A lambda is one of two things, and the parameter it is passed to says which. Passed to a
//! borrow parameter whose declared type is a function type, it is a *block*: it cannot
//! outlive the call, so it works on the enclosing function's variables directly. Anywhere
//! else it is a *closure*, which captures the variables it uses by value.

use std::collections::HashMap;

use koo_syntax::Span;
use koo_syntax::ast;

use super::{Checker, FnSig, diverges};
use crate::hir::*;

/// The state of the function being checked, set aside while a lambda inside it is checked.
#[derive(Default)]
pub(super) struct Enclosing {
    locals: Vec<Local>,
    used: Vec<bool>,
    declared_at: Vec<Span>,
    caught: HashMap<LocalId, ErrSet>,
    scopes: Vec<HashMap<String, LocalId>>,
    ret: Ty,
    loops: usize,
    here: Option<super::FnAt>,
    lambda: Option<LambdaState>,
    /// The function's own function parameters that are only lent to it.
    borrowed_fns: std::collections::HashSet<LocalId>,
}

impl Enclosing {
    /// The type of the variable `name` of this function, if it has one in scope.
    pub(super) fn local_type(&self, name: &str) -> Option<Ty> {
        let local = self.scopes.iter().rev().find_map(|scope| scope.get(name).copied())?;
        Some(self.locals[local].ty.clone())
    }
}

/// What is known about the lambda being checked.
#[derive(Default)]
pub(super) struct LambdaState {
    /// Whether the lambda is a block, which uses the enclosing variables in place.
    pub block: bool,
    /// The variables it uses from the enclosing function: theirs, and its own name for each.
    pub captures: Vec<(LocalId, LocalId)>,
    /// When the return type is being inferred: the types returned and the errors propagated.
    pub infer: Option<Inferred>,
}

#[derive(Default, Clone)]
pub(super) struct Inferred {
    pub ok: Option<Ty>,
    pub errors: Vec<ErrorId>,
    pub any_error: bool,
}

impl<'a> Checker<'a> {
    fn swap_enclosing(&mut self, other: &mut Enclosing) {
        std::mem::swap(&mut self.locals, &mut other.locals);
        std::mem::swap(&mut self.used, &mut other.used);
        std::mem::swap(&mut self.declared_at, &mut other.declared_at);
        std::mem::swap(&mut self.caught, &mut other.caught);
        std::mem::swap(&mut self.here, &mut other.here);
        std::mem::swap(&mut self.scopes, &mut other.scopes);
        std::mem::swap(&mut self.ret, &mut other.ret);
        std::mem::swap(&mut self.loops, &mut other.loops);
        std::mem::swap(&mut self.lambda, &mut other.lambda);
        std::mem::swap(&mut self.borrowed_fns, &mut other.borrowed_fns);
    }

    /// A local variable by name: one of this function's, or one a lambda takes from the
    /// function around it.
    pub(super) fn resolve_local(&mut self, name: &str) -> Option<LocalId> {
        self.lookup(name).or_else(|| self.capture(name))
    }

    fn capture(&mut self, name: &str) -> Option<LocalId> {
        let block = self.lambda.as_ref()?.block;
        let mut enclosing = self.enclosing.pop()?;
        self.swap_enclosing(&mut enclosing);
        // Now `self` is the enclosing function, which may itself be a lambda.
        let found = self.resolve_local(name);
        let info = found.map(|id| {
            self.used[id] = true;
            (id, self.locals[id].clone(), self.borrowed_fns.contains(&id))
        });
        self.swap_enclosing(&mut enclosing);
        self.enclosing.push(enclosing);
        let (outer, local, borrowed_fn) = info?;

        let inner = self.locals.len();
        self.locals.push(Local {
            name: name.to_string(),
            ty: local.ty,
            // A block changes the enclosing variable itself. A closure holds a copy, and
            // assigning to a copy would change nothing the enclosing function can see.
            mutable: block && local.mutable,
            kind: LocalKind::Capture { by_ref: block },
        });
        self.used.push(true);
        self.declared_at.push(Span::default());
        self.scopes[0].insert(name.to_string(), inner);
        if borrowed_fn {
            // A block may call a lent function or pass it on, as the function it is
            // written in may. A closure may outlive the call, so it may not use one at
            // all: the use is reported where it is written.
            self.borrowed_fns.insert(inner);
            if !block {
                self.lent_in_closure.insert(inner);
            }
        }
        self.lambda.as_mut().unwrap().captures.push((outer, inner));
        Some(inner)
    }

    /// Checks a lambda. `expected` is the function type the context wants, if it is known;
    /// its return type may be unknown, and is then inferred from the body.
    pub(super) fn lambda(
        &mut self,
        params: &[ast::LambdaParam],
        body: &ast::Body,
        expected: Option<&Ty>,
        span: Span,
    ) -> Expr {
        let block = std::mem::take(&mut self.next_lambda_is_block);
        let expected_fn = match expected.map(super::exprs::peel) {
            Some(Ty::Fn(function)) => Some((**function).clone()),
            _ => None,
        };
        if let Some(function) = &expected_fn
            && function.params.len() != params.len()
        {
            self.error(
                span,
                format!(
                    "this lambda takes {} parameter{}, but {} {} expected",
                    params.len(),
                    if params.len() == 1 { "" } else { "s" },
                    function.params.len(),
                    if function.params.len() == 1 { "is" } else { "are" },
                ),
            );
            let wanted = self.ty_name(&Ty::Fn(Box::new(function.clone())));
            self.label(format!("a `{wanted}` is expected here"));
            if function.params.len() == 1 && params.len() == 2 {
                self.help("a function of one parameter is written `x -> ...`; if the one value is a pair, take it apart inside: `pair -> { (a, b) := pair ... }`");
            }
            return self.unknown(span);
        }
        // The parameters: from their annotations, or from the expected type.
        let mut param_types: Vec<(ParamKind, Ty)> = Vec::new();
        for (index, param) in params.iter().enumerate() {
            let from_context = expected_fn.as_ref().map(|f| f.params[index].clone());
            match (&param.ty, from_context) {
                (Some(ty), context) => {
                    let kind = super::param_kind(param.kind);
                    let resolved = self.resolve_type(ty);
                    if let Some((_, wanted)) = &context
                        && *wanted != resolved
                        && !wanted.is_unknown()
                        && !resolved.is_unknown()
                    {
                        let (want, found) = (self.ty_name(wanted), self.ty_name(&resolved));
                        self.error(ty.span, format!("expected `{want}`, found `{found}`"));
                    }
                    param_types.push((context.map(|(kind, _)| kind).unwrap_or(kind), resolved));
                }
                (None, Some(context)) => param_types.push(context),
                (None, None) => {
                    self.error_help(
                        param.span,
                        "the type of this parameter is not known here",
                        "write it: `(x: int) -> ...` (Section 3.9)",
                    );
                    param_types.push((ParamKind::Borrow, Ty::Unknown));
                }
            }
        }
        let known_ret = expected_fn
            .as_ref()
            .map(|f| f.ret.clone())
            .filter(|ret| !ret.is_unknown());

        // An expression body needs one pass: its type is the return type. A block body
        // whose return type is not known is checked twice, first to find that type.
        let allowed = self.lambda_errors.take();
        let ret = match (&known_ret, body) {
            (Some(ret), _) => ret.clone(),
            (None, ast::Body::Stmt(stmt)) if allowed.is_none() && matches!(stmt.kind, ast::StmtKind::Expr(_)) => {
                Ty::Unknown
            }
            (None, _) => {
                let diags = self.diags.len();
                let probe = self.lambda_body(params, &param_types, body, Ty::Unknown, block, true);
                self.diags.truncate(diags);
                // A lambda that fails with some of the errors it is allowed, or with
                // none, is a function that may fail with them all.
                match (probe.ret, allowed) {
                    (Ty::Result(ok, set), Some(allowed)) if set.within(&allowed) => Ty::Result(ok, allowed),
                    (ret, Some(allowed)) if !matches!(ret, Ty::Result(..) | Ty::Unknown) => {
                        Ty::Result(Box::new(if ret == Ty::Never { Ty::Unit } else { ret }), allowed)
                    }
                    (ret, _) => ret,
                }
            }
        };
        let function = self.lambda_body(params, &param_types, body, ret, block, false);

        let ty = Ty::Fn(Box::new(FnTy {
            params: param_types,
            ret: function.ret.clone(),
        }));
        let captures = function.outer_captures.clone();
        let id = self.sigs.len();
        self.lambda_count += 1;
        let name = format!("lambda {}", self.lambda_count);
        self.sigs.push(FnSig {
            name: name.clone(),
            receiver: None,
            params: Vec::new(),
            ret: function.ret.clone(),
            at: None,
        });
        self.program.functions.push(Function {
            name,
            params: params.len(),
            locals: function.locals,
            ret: function.ret,
            body: function.body,
            span,
            closure: true,
            captures: function.inner_captures,
            prelude: self.body_in_prelude,
        });
        Expr {
            kind: ExprKind::Closure {
                func: id,
                captures,
                block,
            },
            ty,
            span,
        }
    }

    fn lambda_body(
        &mut self,
        params: &[ast::LambdaParam],
        param_types: &[(ParamKind, Ty)],
        body: &ast::Body,
        ret: Ty,
        block: bool,
        probe: bool,
    ) -> CheckedLambda {
        let infer = ret.is_unknown();
        let mut enclosing = Enclosing {
            ret: ret.clone(),
            lambda: Some(LambdaState {
                block,
                captures: Vec::new(),
                infer: infer.then(Inferred::default),
            }),
            scopes: vec![HashMap::new()],
            ..Enclosing::default()
        };
        self.swap_enclosing(&mut enclosing);
        self.enclosing.push(enclosing);
        // Whether an enclosing loop exists does not matter inside: `break` cannot leave a lambda.

        for (param, (kind, ty)) in params.iter().zip(param_types) {
            let local = self.locals.len();
            let name = param
                .name
                .as_ref()
                .map(|n| n.name.clone())
                .unwrap_or_else(|| "_".to_string());
            self.locals.push(Local {
                name: name.clone(),
                ty: ty.clone(),
                mutable: *kind == ParamKind::Var,
                kind: LocalKind::Param(*kind),
            });
            self.used.push(true);
            self.declared_at.push(param.span);
            if param.name.is_some() && self.scopes[0].insert(name.clone(), local).is_some() {
                self.error(param.span, format!("the parameter `{name}` is declared twice"));
            }
            if matches!(ty, Ty::Fn(_)) && *kind == ParamKind::Borrow {
                self.borrowed_fns.insert(local);
            }
        }

        let (mut checked, mut value_ty) = match body {
            ast::Body::Stmt(stmt) if matches!(stmt.kind, ast::StmtKind::Expr(_)) => {
                let ast::StmtKind::Expr(expr) = &stmt.kind else {
                    unreachable!()
                };
                let hint = match &ret {
                    Ty::Result(ok, _) => Some((**ok).clone()),
                    Ty::Unknown => None,
                    other => Some(other.clone()),
                };
                let value = self.expr(expr, hint.as_ref());
                let ty = value.ty.clone();
                (
                    Block {
                        stmts: Vec::new(),
                        value: Some(Box::new(value)),
                    },
                    Some(ty),
                )
            }
            ast::Body::Stmt(stmt) => {
                let mut stmts = Vec::new();
                self.stmt(stmt, &mut stmts);
                (Block { stmts, value: None }, None)
            }
            ast::Body::Block(block) => (self.block(block), None),
        };

        // The return type, if it was to be inferred.
        let state = self.lambda.as_ref().unwrap();
        let ret = match &state.infer {
            None => ret,
            Some(inferred) => {
                let ok = match (&value_ty, &inferred.ok) {
                    (Some(ty), _) => ty.clone(),
                    (None, Some(ty)) => ty.clone(),
                    (None, None) => Ty::Unit,
                };
                let ok = if ok == Ty::Never { Ty::Unit } else { ok };
                if inferred.any_error {
                    Ty::Result(Box::new(ok), ErrSet::Any)
                } else if !inferred.errors.is_empty() && !matches!(ok, Ty::Result(..)) {
                    Ty::Result(Box::new(ok), ErrSet::of(inferred.errors.clone()))
                } else {
                    ok
                }
            }
        };
        // The value of an expression body is what the lambda returns.
        if let Some(value) = checked.value.take() {
            let span = value.span;
            let value = if probe { *value } else { self.coerce(*value, &ret) };
            value_ty = None;
            checked.stmts.push(Stmt::Expr(Expr {
                kind: ExprKind::Return(Some(Box::new(value))),
                ty: Ty::Never,
                span,
            }));
        }
        let _ = value_ty;
        let ok = match &ret {
            Ty::Result(ok, _) => (**ok).clone(),
            other => other.clone(),
        };
        if !probe && !diverges(&checked) && !matches!(ok, Ty::Unit | Ty::Unknown) {
            let span = match body {
                ast::Body::Block(block) => block.span,
                ast::Body::Stmt(stmt) => stmt.span,
            };
            let wanted = self.ty_name(&ret);
            self.error_help(
                span,
                "this lambda can reach its end without returning a value",
                format!("it returns `{wanted}`: end every path with `return` or `fail`"),
            );
        }

        let mut enclosing = self.enclosing.pop().unwrap();
        self.swap_enclosing(&mut enclosing);
        let state = enclosing.lambda.unwrap_or_default();
        CheckedLambda {
            ret,
            body: checked,
            locals: enclosing.locals,
            outer_captures: state.captures.iter().map(|(outer, _)| *outer).collect(),
            inner_captures: state.captures.iter().map(|(_, inner)| *inner).collect(),
        }
    }

    /// Records what a `return`, `?` or `fail` says about the return type being inferred.
    pub(super) fn note_return(&mut self, ty: &Ty) {
        if let Some(inferred) = self.lambda.as_mut().and_then(|l| l.infer.as_mut())
            && inferred.ok.is_none()
            && !ty.is_unknown()
            && *ty != Ty::Never
        {
            inferred.ok = Some(ty.clone());
        }
    }

    pub(super) fn note_errors(&mut self, set: &ErrSet) {
        if let Some(inferred) = self.lambda.as_mut().and_then(|l| l.infer.as_mut()) {
            match set {
                ErrSet::Any => inferred.any_error = true,
                ErrSet::Of(ids) => {
                    for id in ids {
                        if !inferred.errors.contains(id) {
                            inferred.errors.push(*id);
                        }
                    }
                }
            }
        }
    }
}

struct CheckedLambda {
    ret: Ty,
    body: Block,
    locals: Vec<Local>,
    outer_captures: Vec<LocalId>,
    inner_captures: Vec<LocalId>,
}
