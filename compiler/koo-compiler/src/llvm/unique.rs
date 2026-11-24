//! Finding the lists whose uniqueness check can be made once before a loop.
//!
//! Changing an element first makes the list uniquely owned, which is where copy-on-write
//! copies (Section 5.1). Inside a loop that is one load and one branch per change, and the
//! specification asks for it to be hoisted (Section 5.11). A list stays uniquely owned for
//! as long as nobody takes another reference to it, and a reference can only be taken by
//! using the variable as a value. So if every use of a list variable in a loop is an index
//! (`xs[i]`), `xs.len` or `xs.push(v)`, one check before the loop covers every change
//! inside it.

use std::collections::HashSet;

use crate::hir::*;

#[derive(Default)]
pub(super) struct Uses {
    /// Variables used as the list of an index expression.
    pub indexed: HashSet<LocalId>,
    /// Variables with an element that is changed.
    pub written: HashSet<LocalId>,
    /// Variables that are pushed to.
    pub pushed: HashSet<LocalId>,
    /// Whether the loop calls a function. A variable that a block shares with the
    /// function around it can then be copied by another block of that function.
    pub calls: bool,
    /// Variables used in any other way.
    pub other: HashSet<LocalId>,
}

impl Uses {
    pub fn block(&mut self, block: &Block) {
        for stmt in &block.stmts {
            match stmt {
                Stmt::Let { init, .. } => self.expr(init),
                Stmt::Assign { target, value, .. } => {
                    self.write(target);
                    self.expr(target);
                    self.expr(value);
                }
                Stmt::Expr(expr) => self.expr(expr),
                Stmt::While { cond, body } => {
                    self.cond(cond);
                    self.block(body);
                }
                Stmt::ForRange { lo, hi, body, .. } => {
                    self.expr(lo);
                    self.expr(hi);
                    self.block(body);
                }
                Stmt::ForList { list, body, .. } => {
                    self.expr(list);
                    self.block(body);
                }
            }
        }
        if let Some(value) = &block.value {
            self.expr(value);
        }
    }

    pub fn cond(&mut self, cond: &Cond) {
        match cond {
            Cond::Expr(expr) => self.expr(expr),
            Cond::Bind { value, .. } => self.expr(value),
        }
    }

    /// Records a place that is about to be changed: the variable at the bottom of it, if
    /// the path from that variable begins with an index.
    fn write(&mut self, place: &Expr) {
        let mut current = place;
        loop {
            match &current.kind {
                ExprKind::Index(base, _) => {
                    if let ExprKind::Local(local) = &base.kind {
                        self.written.insert(*local);
                        return;
                    }
                    current = base;
                }
                ExprKind::Field(base, _) | ExprKind::Retype(base) => current = base,
                _ => return,
            }
        }
    }

    pub fn expr(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Bool(_)
            | ExprKind::Str(_)
            | ExprKind::Unit
            | ExprKind::NoneLit
            | ExprKind::Break
            | ExprKind::Continue
            | ExprKind::Return(None) => {}
            ExprKind::Local(local) => {
                self.other.insert(*local);
            }
            ExprKind::Index(base, index) => {
                match &base.kind {
                    ExprKind::Local(local) => {
                        self.indexed.insert(*local);
                    }
                    _ => self.expr(base),
                }
                self.expr(index);
            }
            ExprKind::Builtin(Builtin::Len, args) => {
                if !matches!(args[0].kind, ExprKind::Local(_)) {
                    self.expr(&args[0]);
                }
            }
            ExprKind::Builtin(Builtin::ListPush, args) => {
                // Pushing to a variable changes the list and shares nothing.
                match &args[0].kind {
                    ExprKind::Local(local) => {
                        self.pushed.insert(*local);
                    }
                    _ => {
                        self.write(&args[0]);
                        self.expr(&args[0]);
                    }
                }
                self.expr(&args[1]);
            }
            ExprKind::Builtin(builtin, args) => {
                if matches!(
                    builtin,
                    Builtin::ListPop | Builtin::ListSwap | Builtin::ListClear | Builtin::ListSortInts
                ) {
                    self.write(&args[0]);
                }
                for arg in args {
                    self.expr(arg);
                }
            }
            ExprKind::Call { args, .. } => {
                self.calls = true;
                for arg in args {
                    if arg.kind == ParamKind::Var {
                        self.write(&arg.expr);
                    }
                    self.expr(&arg.expr);
                }
            }
            ExprKind::CallValue { callee, args } => {
                self.calls = true;
                self.expr(callee);
                for arg in args {
                    if arg.kind == ParamKind::Var {
                        self.write(&arg.expr);
                    }
                    self.expr(&arg.expr);
                }
            }
            // A lambda may share or change any variable it captures.
            ExprKind::Closure { captures, .. } => self.other.extend(captures.iter().copied()),
            ExprKind::FnRef(_) | ExprKind::Location => {}
            ExprKind::Intrinsic { args, .. } => {
                for arg in args {
                    self.expr(arg);
                }
            }
            ExprKind::Field(inner, _)
            | ExprKind::Unary(_, inner)
            | ExprKind::WrapSome(inner)
            | ExprKind::WrapOk(inner)
            | ExprKind::Retype(inner)
            | ExprKind::Try(inner)
            | ExprKind::Fail(inner)
            | ExprKind::Panic(inner)
            | ExprKind::Return(Some(inner)) => self.expr(inner),
            ExprKind::Binary(_, a, b) | ExprKind::And(a, b) | ExprKind::Or(a, b) => {
                self.expr(a);
                self.expr(b);
            }
            ExprKind::StructLit(_, values)
            | ExprKind::EnumLit(_, _, values)
            | ExprKind::ErrorLit(_, values)
            | ExprKind::ListLit(values)
            | ExprKind::TupleLit(values) => {
                for value in values {
                    self.expr(value);
                }
            }
            ExprKind::Interp(parts) => {
                for part in parts {
                    if let InterpPart::Value(value, _) = part {
                        self.expr(value);
                    }
                }
            }
            ExprKind::Else { expr, alt, .. } => {
                self.expr(expr);
                self.block(alt);
            }
            ExprKind::If { cond, then, otherwise } => {
                self.cond(cond);
                self.block(then);
                if let Some(otherwise) = otherwise {
                    self.block(otherwise);
                }
            }
            ExprKind::Match { scrutinee, arms } => {
                self.expr(scrutinee);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.expr(guard);
                    }
                    self.block(&arm.body);
                }
            }
        }
    }
}
