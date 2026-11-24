//! Code generation for blocks, statements and control flow.

use koo_syntax::Span;

use super::exprs::may_write;
use super::unique::Uses;
use super::{Gen, Glue, LoopLabels, ScopeKind, is_aggregate};
use crate::hir::*;

impl Gen<'_> {
    pub(super) fn local_ty(&self, local: LocalId) -> Ty {
        self.p.functions[self.f.id].locals[local].ty.clone()
    }

    /// Gives a local variable a slot, owned by the innermost block.
    pub(super) fn declare(&mut self, local: LocalId) -> String {
        let ty = self.local_ty(local);
        let droppable = self.droppable(&ty);
        let slot = self.alloca(&ty, droppable);
        self.own_in_block(&slot, &ty);
        self.f.locals[local] = Some(slot.clone());
        slot
    }

    /// A block in a scope of its own. Its value, if it has one, goes to `dest`.
    pub(super) fn block(&mut self, block: &Block, dest: Option<&str>) {
        self.push_scope(ScopeKind::Block);
        self.block_body(block, dest);
        self.pop_scope();
    }

    pub(super) fn block_body(&mut self, block: &Block, dest: Option<&str>) {
        for stmt in &block.stmts {
            self.stmt(stmt);
        }
        if let Some(value) = &block.value {
            self.push_scope(ScopeKind::Statement);
            self.eval(value, dest);
            self.pop_scope();
        }
    }

    fn stmt(&mut self, stmt: &Stmt) {
        self.push_scope(ScopeKind::Statement);
        match stmt {
            Stmt::Let { local, init } => {
                let slot = self.declare(*local);
                self.eval(init, Some(&slot));
            }
            Stmt::Assign {
                target,
                op,
                value,
                span,
            } => self.assign(target, *op, value, *span),
            Stmt::Expr(expr) => self.eval(expr, None),
            Stmt::While { cond, body } => {
                let mut uses = Uses::default();
                uses.cond(cond);
                uses.block(body);
                let mark = self.hoist_unique(&uses);
                let head = self.new_label("while");
                let body_label = self.new_label("while.body");
                let end = self.new_label("while.end");
                self.label(&head);
                let (test, bound) = self.cond(cond);
                self.cond_br(&test, &body_label, &end);
                self.label(&body_label);
                self.push_scope(ScopeKind::Loop);
                if let Some((local, slot)) = bound {
                    let ty = self.local_ty(local);
                    self.own(&slot, &ty);
                }
                self.f.loops.push(LoopLabels {
                    break_to: end.clone(),
                    continue_to: head.clone(),
                    scope: self.f.scopes.len() - 1,
                });
                self.block_body(body, None);
                self.f.loops.pop();
                self.pop_scope();
                self.br(&head);
                self.label(&end);
                self.end_hoist(mark);
            }
            Stmt::ForRange {
                local,
                lo,
                hi,
                inclusive,
                body,
            } => {
                let lo = self.gen_scalar(lo);
                let hi = self.gen_scalar(hi);
                let slot = self.declare(*local);
                self.emit(format!("store i64 {lo}, ptr {slot}"));
                let mut uses = Uses::default();
                uses.block(body);
                let mark = self.hoist_unique(&uses);
                let head = self.new_label("for");
                let body_label = self.new_label("for.body");
                let step = self.new_label("for.step");
                let end = self.new_label("for.end");
                if *inclusive {
                    // `lo..=hi` may end at the largest integer, so the test comes after the
                    // body and before the increment.
                    let any = self.value(format!("icmp sle i64 {lo}, {hi}"));
                    self.cond_br(&any, &body_label, &end);
                } else {
                    self.label(&head);
                    let i = self.value(format!("load i64, ptr {slot}"));
                    let more = self.value(format!("icmp slt i64 {i}, {hi}"));
                    self.cond_br(&more, &body_label, &end);
                }
                self.label(&body_label);
                self.push_scope(ScopeKind::Loop);
                self.f.loops.push(LoopLabels {
                    break_to: end.clone(),
                    continue_to: step.clone(),
                    scope: self.f.scopes.len() - 1,
                });
                self.block_body(body, None);
                self.f.loops.pop();
                self.pop_scope();
                self.label(&step);
                let i = self.value(format!("load i64, ptr {slot}"));
                if *inclusive {
                    let last = self.value(format!("icmp eq i64 {i}, {hi}"));
                    let again = self.new_label("for.again");
                    self.cond_br(&last, &end, &again);
                    self.label(&again);
                    let next = self.value(format!("add nsw i64 {i}, 1"));
                    self.emit(format!("store i64 {next}, ptr {slot}"));
                    self.br(&body_label);
                } else {
                    let next = self.value(format!("add nsw i64 {i}, 1"));
                    self.emit(format!("store i64 {next}, ptr {slot}"));
                    self.br(&head);
                }
                self.label(&end);
                self.end_hoist(mark);
            }
            Stmt::ForList { local, list, body } => {
                // The loop walks the list as it was when the loop began (Section 5.4): it
                // holds a reference of its own, so a change in the body copies first.
                let Ty::List(element) = list.ty.clone() else {
                    self.eval(list, None);
                    self.pop_scope();
                    return;
                };
                let snapshot = self.eval_temp(list);
                // The loop's own reference to its list is a use of the list as a value.
                let mut uses = Uses::default();
                uses.expr(list);
                uses.block(body);
                let mark = self.hoist_unique(&uses);
                let handle = self.value(format!("load ptr, ptr {snapshot}"));
                let len_ptr = self.value(format!("getelementptr inbounds i64, ptr {handle}, i64 1"));
                let len = self.value(format!("load i64, ptr {len_ptr}, !tbaa !2"));
                let data = self.value(format!("getelementptr inbounds i8, ptr {handle}, i64 24"));
                let index = self.alloca(&Ty::Int, false);
                self.emit(format!("store i64 0, ptr {index}"));
                let head = self.new_label("each");
                let body_label = self.new_label("each.body");
                let step = self.new_label("each.step");
                let end = self.new_label("each.end");
                self.label(&head);
                let i = self.value(format!("load i64, ptr {index}"));
                let more = self.value(format!("icmp slt i64 {i}, {len}"));
                self.cond_br(&more, &body_label, &end);
                self.label(&body_label);
                // The loop variable names the element in place.
                let llty = self.llty(&element);
                let place = self.value(format!("getelementptr inbounds {llty}, ptr {data}, i64 {i}"));
                self.f.locals[*local] = Some(place);
                self.push_scope(ScopeKind::Loop);
                self.f.loops.push(LoopLabels {
                    break_to: end.clone(),
                    continue_to: step.clone(),
                    scope: self.f.scopes.len() - 1,
                });
                self.block_body(body, None);
                self.f.loops.pop();
                self.pop_scope();
                self.label(&step);
                let i = self.value(format!("load i64, ptr {index}"));
                let next = self.value(format!("add nsw i64 {i}, 1"));
                self.emit(format!("store i64 {next}, ptr {index}"));
                self.br(&head);
                self.label(&end);
                self.end_hoist(mark);
            }
        }
        self.pop_scope();
    }

    /// Makes unique, once, every list that the loop about to start changes by index and
    /// uses in no way that could share it. Returns what to truncate `unique` to afterwards.
    fn hoist_unique(&mut self, uses: &Uses) -> usize {
        let mark = self.f.unique.len();
        let mut locals: Vec<LocalId> = uses
            .written
            .iter()
            .filter(|local| uses.indexed.contains(local))
            .chain(&uses.pushed)
            .copied()
            .filter(|local| !uses.other.contains(local))
            .collect();
        locals.sort_unstable();
        locals.dedup();
        for local in locals {
            let info = &self.p.functions[self.f.id].locals[local];
            let (ty, mutable) = (info.ty.clone(), info.mutable);
            let Some(slot) = self.f.locals[local].clone() else {
                continue;
            };
            if !mutable || !matches!(ty, Ty::List(_)) || self.f.unique.contains(&local) {
                continue;
            }
            // A variable taken by reference from the enclosing function is shared with
            // every block written there, and a call may run one of them.
            if uses.calls && matches!(info.kind, LocalKind::Capture { by_ref: true }) {
                continue;
            }
            let unique = self.glue(Glue::Unique(ty));
            self.emit(format!("call void {unique}(ptr {slot})"));
            self.f.unique.push(local);
            if uses.pushed.contains(&local) {
                // Every use of the list in the loop is an index, its length or a push, so
                // only a push changes the length, and each keeps this slot equal to it.
                let len = self.alloca(&Ty::Int, false);
                let list = self.value(format!("load ptr, ptr {slot}"));
                let len_ptr = self.value(format!("getelementptr inbounds i64, ptr {list}, i64 1"));
                let now = self.value(format!("load i64, ptr {len_ptr}, !tbaa !2"));
                self.emit(format!("store i64 {now}, ptr {len}"));
                self.f.lens.insert(local, len);
            }
        }
        mark
    }

    /// Ends what `hoist_unique` began: the loop is over.
    fn end_hoist(&mut self, mark: usize) {
        for local in self.f.unique.split_off(mark) {
            self.f.lens.remove(&local);
        }
    }

    /// Evaluates a condition. For `x := e` it also moves the value into the slot of `x`,
    /// which the caller hands to the scope of the block that the condition guards.
    pub(super) fn cond(&mut self, cond: &Cond) -> (String, Option<(LocalId, String)>) {
        self.push_scope(ScopeKind::Statement);
        let result = match cond {
            Cond::Expr(expr) => (self.gen_scalar(expr), None),
            Cond::Bind { local, value } => {
                let inner = self.local_ty(*local);
                let droppable = self.droppable(&inner);
                let slot = self.alloca(&inner, droppable);
                self.f.locals[*local] = Some(slot.clone());
                let optional = self.eval_temp(value);
                let llty = self.llty(&value.ty);
                let present = self.value(format!("load i1, ptr {optional}"));
                let take = self.new_label("bind");
                let after = self.new_label("bind.end");
                self.cond_br(&present, &take, &after);
                self.label(&take);
                let payload = self.field_ptr(&llty, &optional, 1);
                self.move_value(&inner, &slot, &payload);
                if self.droppable(&value.ty) {
                    self.zero(&value.ty, &optional);
                }
                self.label(&after);
                (present, Some((*local, slot)))
            }
        };
        self.pop_scope();
        result
    }

    /// Moves a value from one place to another: the bits, with no change of ownership.
    pub(super) fn move_value(&mut self, ty: &Ty, dest: &str, src: &str) {
        if is_aggregate(ty) {
            self.memcpy(ty, dest, src);
        } else {
            let value = self.load(ty, src);
            self.store(ty, &value, dest);
        }
    }

    pub(super) fn gen_if(&mut self, cond: &Cond, then: &Block, otherwise: Option<&Block>, dest: Option<&str>) {
        let (test, bound) = self.cond(cond);
        let then_label = self.new_label("then");
        let else_label = self.new_label("else");
        let end = self.new_label("endif");
        self.cond_br(&test, &then_label, if otherwise.is_some() { &else_label } else { &end });
        self.label(&then_label);
        self.push_scope(ScopeKind::Block);
        if let Some((local, slot)) = bound {
            let ty = self.local_ty(local);
            self.own(&slot, &ty);
        }
        self.block_body(then, dest);
        self.pop_scope();
        self.br(&end);
        if let Some(otherwise) = otherwise {
            self.label(&else_label);
            self.block(otherwise, dest);
            self.br(&end);
        }
        self.label(&end);
    }

    /// An assignment. Evaluation is left to right: the index expressions of the target,
    /// then for a compound assignment the target's value, then the right-hand side. The
    /// address is computed last, from the indices, because the right-hand side may move
    /// the storage it would point into.
    fn assign(&mut self, target: &Expr, op: Option<BinOp>, value: &Expr, span: Span) {
        let ty = target.ty.clone();
        let indices = self.place_indices(target);
        let reads_first = op.is_some() && may_write(value);
        match op {
            None => {
                if self.droppable(&ty) || is_aggregate(&ty) {
                    // The new value is complete before the old one is dropped.
                    let new = self.eval_temp(value);
                    let place = self.addr_mut_at(target, &mut indices.into_iter());
                    self.drop_at(&ty, &place);
                    self.move_value(&ty, &place, &new);
                    if self.droppable(&ty) {
                        self.zero(&ty, &new);
                    }
                } else {
                    let new = self.gen_scalar(value);
                    let place = self.addr_mut_at(target, &mut indices.into_iter());
                    self.store(&ty, &new, &place);
                }
            }
            Some(BinOp::Add) if ty == Ty::Str => {
                if reads_first {
                    // `s += f(var s)`: the text `s` had before `f` ran, then the result.
                    let before = self.temp(&Ty::Str);
                    let place = self.addr_mut_at(target, &mut indices.clone().into_iter());
                    let old = self.load(&ty, &place);
                    self.retain_value(&ty, &old);
                    self.store(&ty, &old, &before);
                    let operand = self.eval_temp(value);
                    let text = self.load(&ty, &operand);
                    let joined = self.value(format!("call ptr @koo_str_concat(ptr {old}, ptr {text})"));
                    let place = self.addr_mut_at(target, &mut indices.into_iter());
                    self.drop_at(&ty, &place);
                    self.store(&ty, &joined, &place);
                } else {
                    // `s += t` appends in place when `s` is uniquely owned (Section 4.4).
                    // The operand is held by its own reference, so it cannot be the
                    // buffer that the append grows.
                    let operand = self.eval_temp(value);
                    let place = self.addr_mut_at(target, &mut indices.into_iter());
                    let text = self.load(&ty, &operand);
                    self.emit(format!("call void @koo_str_append(ptr {place}, ptr {text})"));
                }
            }
            Some(op) => {
                let before = if reads_first {
                    let place = self.addr_mut_at(target, &mut indices.clone().into_iter());
                    Some(self.load(&ty, &place))
                } else {
                    None
                };
                let operand = self.gen_scalar(value);
                let place = self.addr_mut_at(target, &mut indices.into_iter());
                let current = match before {
                    Some(before) => before,
                    None => self.load(&ty, &place),
                };
                let result = self.arithmetic(op, &ty, &current, &operand, span);
                self.store(&ty, &result, &place);
            }
        }
    }

    pub(super) fn gen_break(&mut self, to_continue: bool) {
        let Some(labels) = self.f.loops.last() else {
            return;
        };
        let (scope, target) = (
            labels.scope,
            if to_continue {
                labels.continue_to.clone()
            } else {
                labels.break_to.clone()
            },
        );
        self.cleanup_from(scope);
        self.br(&target);
    }
}
