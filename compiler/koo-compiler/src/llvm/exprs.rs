//! Code generation for expressions.

use koo_syntax::Span;

use super::{Gen, Glue, ScopeKind, is_aggregate, library_name};
use crate::hir::*;
use crate::intrinsics::I;

/// Whether `f` holds for the expression or anything inside it.
fn any(expr: &Expr, f: &mut dyn FnMut(&Expr) -> bool) -> bool {
    fn in_block(block: &Block, f: &mut dyn FnMut(&Expr) -> bool) -> bool {
        block.stmts.iter().any(|stmt| match stmt {
            Stmt::Let { init, .. } => any(init, f),
            Stmt::Assign { target, value, .. } => any(target, f) || any(value, f),
            Stmt::Expr(expr) => any(expr, f),
            Stmt::While { cond, body } => in_cond(cond, f) || in_block(body, f),
            Stmt::ForRange { lo, hi, body, .. } => any(lo, f) || any(hi, f) || in_block(body, f),
            Stmt::ForList { list, body, .. } => any(list, f) || in_block(body, f),
        }) || block.value.as_ref().is_some_and(|value| any(value, f))
    }
    fn in_cond(cond: &Cond, f: &mut dyn FnMut(&Expr) -> bool) -> bool {
        match cond {
            Cond::Expr(expr) => any(expr, f),
            Cond::Bind { value, .. } => any(value, f),
        }
    }
    if f(expr) {
        return true;
    }
    match &expr.kind {
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::Str(_)
        | ExprKind::Unit
        | ExprKind::Local(_)
        | ExprKind::NoneLit
        | ExprKind::Break
        | ExprKind::Continue
        | ExprKind::Return(None) => false,
        ExprKind::Field(inner, _)
        | ExprKind::Unary(_, inner)
        | ExprKind::WrapSome(inner)
        | ExprKind::WrapOk(inner)
        | ExprKind::Retype(inner)
        | ExprKind::Try(inner)
        | ExprKind::Fail(inner)
        | ExprKind::Panic(inner)
        | ExprKind::Return(Some(inner)) => any(inner, f),
        ExprKind::Index(a, b) | ExprKind::Binary(_, a, b) | ExprKind::And(a, b) | ExprKind::Or(a, b) => {
            any(a, f) || any(b, f)
        }
        ExprKind::Call { args, .. } => args.iter().any(|arg| any(&arg.expr, f)),
        ExprKind::CallValue { callee, args } => any(callee, f) || args.iter().any(|arg| any(&arg.expr, f)),
        ExprKind::Intrinsic { args, .. } => args.iter().any(|arg| any(arg, f)),
        ExprKind::Closure { .. } | ExprKind::FnRef(_) | ExprKind::Location => false,
        ExprKind::Builtin(_, values)
        | ExprKind::StructLit(_, values)
        | ExprKind::EnumLit(_, _, values)
        | ExprKind::ErrorLit(_, values)
        | ExprKind::ListLit(values)
        | ExprKind::TupleLit(values) => values.iter().any(|value| any(value, f)),
        ExprKind::Interp(parts) => parts.iter().any(|part| match part {
            InterpPart::Lit(_) => false,
            InterpPart::Value(value, _) => any(value, f),
        }),
        ExprKind::Else { expr, alt, .. } => any(expr, f) || in_block(alt, f),
        ExprKind::If { cond, then, otherwise } => {
            in_cond(cond, f) || in_block(then, f) || otherwise.as_ref().is_some_and(|block| in_block(block, f))
        }
        ExprKind::Match { scrutinee, arms } => {
            any(scrutinee, f)
                || arms
                    .iter()
                    .any(|arm| arm.guard.as_ref().is_some_and(|guard| any(guard, f)) || in_block(&arm.body, f))
        }
    }
}

/// Whether evaluating the expression may change a variable: a read made before it must
/// then hold its own reference (Section 5.4).
pub(super) fn may_write(expr: &Expr) -> bool {
    any(expr, &mut |e| match &e.kind {
        // A function value that is passed or called may be a block, which changes the
        // variables it uses.
        ExprKind::Call { args, .. } => args
            .iter()
            .any(|arg| arg.kind == ParamKind::Var || matches!(arg.expr.ty, Ty::Fn(_))),
        ExprKind::CallValue { .. } => true,
        ExprKind::Builtin(
            Builtin::ListPush | Builtin::ListPop | Builtin::ListSwap | Builtin::ListClear | Builtin::ListSortInts,
            _,
        ) => true,
        ExprKind::Else { alt, .. } => !alt.stmts.is_empty(),
        ExprKind::If { then, otherwise, .. } => {
            !then.stmts.is_empty() || otherwise.as_ref().is_some_and(|block| !block.stmts.is_empty())
        }
        ExprKind::Match { arms, .. } => arms.iter().any(|arm| !arm.body.stmts.is_empty()),
        _ => false,
    })
}

/// Whether evaluating the expression may leave the function or a loop part-way.
fn may_exit(expr: &Expr) -> bool {
    any(expr, &mut |e| {
        matches!(
            e.kind,
            ExprKind::Try(_) | ExprKind::Return(_) | ExprKind::Fail(_) | ExprKind::Break | ExprKind::Continue
        )
    })
}

/// Whether the expression names storage: a variable, or a field or element.
fn is_place(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Local(_) | ExprKind::Field(..) | ExprKind::Index(..) => true,
        ExprKind::Retype(inner) => is_place(inner),
        _ => false,
    }
}

fn root_local(expr: &Expr) -> Option<LocalId> {
    match &expr.kind {
        ExprKind::Local(id) => Some(*id),
        ExprKind::Field(base, _) | ExprKind::Index(base, _) | ExprKind::Retype(base) => root_local(base),
        _ => None,
    }
}

/// The order in which the operands of a call or a literal are evaluated: as written, left
/// to right (Section 5.4), which is not always the order of the parameters or fields.
fn source_order<'e>(exprs: impl Iterator<Item = &'e Expr>) -> Vec<usize> {
    let starts: Vec<u32> = exprs.map(|expr| expr.span.start).collect();
    let mut order: Vec<usize> = (0..starts.len()).collect();
    order.sort_by_key(|&index| starts[index]);
    order
}

fn float_literal(value: f64) -> String {
    format!("0x{:016X}", value.to_bits())
}

impl Gen<'_> {
    /// `ptrtoint (getelementptr T, null, 1)`: the size LLVM gives the type.
    pub(super) fn sizeof(&self, ty: &Ty) -> String {
        format!(
            "ptrtoint (ptr getelementptr ({}, ptr null, i32 1) to i64)",
            self.llty(ty)
        )
    }

    // ---- reading ------------------------------------------------------------------------

    /// The address of the value of an expression. It is valid until the end of the
    /// statement; nothing is retained.
    pub(super) fn addr(&mut self, expr: &Expr) -> String {
        match &expr.kind {
            ExprKind::Local(id) => self.f.locals[*id]
                .clone()
                .expect("a local is declared before it is used"),
            ExprKind::Retype(inner) => self.addr(inner),
            ExprKind::Field(base, index) => match &base.ty {
                Ty::Struct(_) | Ty::Tuple(_) => {
                    let struct_ty = self.llty(&base.ty);
                    let base = self.addr(base);
                    self.field_ptr(&struct_ty, &base, *index)
                }
                Ty::Error(id) => {
                    let box_ty = self.error_box_ty(*id);
                    let value = self.borrow(base);
                    self.field_ptr(&box_ty, &value, *index + 2)
                }
                _ => self.eval_temp(expr),
            },
            ExprKind::Index(base, index) => {
                let Ty::List(element) = &base.ty else {
                    return self.eval_temp(expr);
                };
                let list = if may_write(index) {
                    self.own_operand(base)
                } else {
                    self.borrow(base)
                };
                let i = self.gen_scalar(index);
                self.element_ptr(&list, &i, element, Some(expr.span))
            }
            _ => self.eval_temp(expr),
        }
    }

    /// A pointer to element `i` of a list, after checking the index if a span is given.
    pub(super) fn element_ptr(&mut self, list: &str, i: &str, element: &Ty, check: Option<Span>) -> String {
        if let Some(span) = check {
            let len_ptr = self.value(format!("getelementptr inbounds i64, ptr {list}, i64 1"));
            let len = self.value(format!("load i64, ptr {len_ptr}, !tbaa !2"));
            let ok = self.value(format!("icmp ult i64 {i}, {len}"));
            self.check(&ok, "index out of range", span);
        }
        let data = self.value(format!("getelementptr inbounds i8, ptr {list}, i64 24"));
        let llty = self.llty(element);
        self.value(format!("getelementptr inbounds {llty}, ptr {data}, i64 {i}"))
    }

    /// The address of storage that is about to be changed. A list on the way is made
    /// uniquely owned first, which is where copy-on-write copies.
    pub(super) fn addr_mut(&mut self, expr: &Expr) -> String {
        let indices = self.place_indices(expr);
        self.addr_mut_at(expr, &mut indices.into_iter())
    }

    /// Evaluates the index expressions of a place, left to right, and nothing else. The
    /// address is computed from them later, with `addr_mut_at`, so that whatever is
    /// evaluated in between cannot leave the address pointing at storage that has moved.
    pub(super) fn place_indices(&mut self, place: &Expr) -> Vec<String> {
        fn collect(generator: &mut Gen<'_>, place: &Expr, out: &mut Vec<String>) {
            match &place.kind {
                ExprKind::Retype(inner) => collect(generator, inner, out),
                ExprKind::Field(base, _) if matches!(base.ty, Ty::Struct(_) | Ty::Tuple(_)) => {
                    collect(generator, base, out)
                }
                ExprKind::Index(base, index) if matches!(base.ty, Ty::List(_)) => {
                    collect(generator, base, out);
                    out.push(generator.gen_scalar(index));
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        collect(self, place, &mut out);
        out
    }

    /// The address of a place whose index expressions `place_indices` already evaluated.
    pub(super) fn addr_mut_at(&mut self, expr: &Expr, indices: &mut std::vec::IntoIter<String>) -> String {
        match &expr.kind {
            ExprKind::Local(id) => self.f.locals[*id]
                .clone()
                .expect("a local is declared before it is used"),
            ExprKind::Retype(inner) => self.addr_mut_at(inner, indices),
            ExprKind::Field(base, index) => {
                if !matches!(base.ty, Ty::Struct(_) | Ty::Tuple(_)) {
                    return self.eval_temp(expr);
                }
                let struct_ty = self.llty(&base.ty);
                let base = self.addr_mut_at(base, indices);
                self.field_ptr(&struct_ty, &base, *index)
            }
            ExprKind::Index(base, _) => {
                let Ty::List(element) = &base.ty else {
                    return self.eval_temp(expr);
                };
                let slot = self.addr_mut_at(base, indices);
                let i = indices.next().expect("one index per index expression");
                // A list the enclosing loop already made unique needs no check here.
                let known = matches!(&base.kind, ExprKind::Local(local) if self.f.unique.contains(local));
                if !known {
                    let unique = self.glue(Glue::Unique(base.ty.clone()));
                    self.emit(format!("call void {unique}(ptr {slot})"));
                }
                let list = self.load(&base.ty, &slot);
                self.element_ptr(&list, &i, element, Some(expr.span))
            }
            _ => self.eval_temp(expr),
        }
    }

    /// The value of an expression, to read and not to keep: a scalar value, or the address
    /// of an aggregate. A place is read where it is; anything else is evaluated into a
    /// temporary that the statement owns.
    pub(super) fn borrow(&mut self, expr: &Expr) -> String {
        if is_aggregate(&expr.ty) {
            return self.addr(expr);
        }
        if is_place(expr) {
            let place = self.addr(expr);
            return self.load(&expr.ty, &place);
        }
        if self.droppable(&expr.ty) {
            let slot = self.eval_temp(expr);
            return self.load(&expr.ty, &slot);
        }
        self.gen_scalar(expr)
    }

    /// Like `borrow`, but the value is a copy the statement owns, so that it stays what it
    /// was whatever is changed afterwards.
    pub(super) fn own_operand(&mut self, expr: &Expr) -> String {
        if is_aggregate(&expr.ty) {
            return self.eval_temp(expr);
        }
        if self.droppable(&expr.ty) {
            let slot = self.eval_temp(expr);
            return self.load(&expr.ty, &slot);
        }
        self.gen_scalar(expr)
    }

    /// Evaluates an expression into a temporary that the innermost scope owns.
    pub(super) fn eval_temp(&mut self, expr: &Expr) -> String {
        let slot = self.temp(&expr.ty);
        self.eval(expr, Some(&slot));
        slot
    }

    // ---- evaluation ---------------------------------------------------------------------

    /// Evaluates an expression. The owned result is stored at `dest`; with no `dest` it is
    /// dropped at the end of the statement.
    pub(super) fn eval(&mut self, expr: &Expr, dest: Option<&str>) {
        match &expr.kind {
            ExprKind::If { cond, then, otherwise } => self.gen_if(cond, then, otherwise.as_ref(), dest),
            ExprKind::Match { scrutinee, arms } => self.gen_match(scrutinee, arms, dest),
            ExprKind::Else {
                expr: inner,
                binding,
                alt,
            } => self.gen_else(inner, *binding, alt, dest, &expr.ty),
            ExprKind::Return(value) => self.gen_return(value.as_deref()),
            ExprKind::Fail(value) => {
                let error = self.gen_scalar(value);
                self.emit(format!("store ptr {error}, ptr %ret"));
                self.cleanup_from(0);
                self.terminate("ret void");
            }
            ExprKind::Break => self.gen_break(false),
            ExprKind::Continue => self.gen_break(true),
            ExprKind::Panic(message) => {
                let text = self.borrow(message);
                // A panic the library raises says which of its functions the program
                // called; the line of the library would only be noise.
                if let Some(slot) = self.f.called_from.clone() {
                    // The function was told where the program called it.
                    let name = library_name(&self.p.functions[self.f.id].name).unwrap_or_default();
                    let name = self.string(&name);
                    let place = self.value(format!("load ptr, ptr {slot}"));
                    self.emit(format!(
                        "call void @koo_panic_str_from(ptr {text}, ptr {name}, ptr {place})"
                    ));
                } else {
                    let (file, line) = if self.f.prelude {
                        self.library_place()
                    } else {
                        (self.file_string(), self.line(expr.span) as i64)
                    };
                    self.emit(format!("call void @koo_panic_str(ptr {text}, ptr {file}, i64 {line})"));
                }
                self.terminate("unreachable");
            }
            _ if matches!(expr.ty, Ty::Unit | Ty::Never | Ty::Unknown) => {
                self.gen_scalar(expr);
            }
            _ => {
                // A result that nobody takes still has an owner, so that it is dropped.
                let temp;
                let dest = match dest {
                    Some(dest) => dest,
                    None if !self.droppable(&expr.ty) && !is_aggregate(&expr.ty) => {
                        self.gen_scalar(expr);
                        return;
                    }
                    None => {
                        temp = self.temp(&expr.ty);
                        &temp
                    }
                };
                if is_aggregate(&expr.ty) {
                    self.eval_aggregate(expr, dest);
                } else {
                    let value = self.gen_scalar(expr);
                    self.store(&expr.ty, &value, dest);
                }
            }
        }
    }

    fn eval_aggregate(&mut self, expr: &Expr, dest: &str) {
        let ty = &expr.ty;
        match &expr.kind {
            ExprKind::Retype(inner) if !is_place(inner) => self.eval(inner, Some(dest)),
            _ if is_place(expr) => {
                let place = self.addr(expr);
                self.memcpy(ty, dest, &place);
                self.retain_at(ty, dest);
            }
            ExprKind::StructLit(id, values) => {
                let struct_ty = self.llty(&Ty::Struct(*id));
                for index in source_order(values.iter()) {
                    let field = self.field_ptr(&struct_ty, dest, index);
                    self.eval(&values[index], Some(&field));
                }
            }
            ExprKind::TupleLit(values) => {
                let llty = self.llty(ty);
                for index in source_order(values.iter()) {
                    let field = self.field_ptr(&llty, dest, index);
                    self.eval(&values[index], Some(&field));
                }
            }
            ExprKind::EnumLit(id, variant, values) => {
                self.emit(format!("store i64 {variant}, ptr {dest}"));
                if !values.is_empty() {
                    let enum_ty = self.llty(&Ty::Enum(*id));
                    let variant_ty = self.variant_ty(*id, *variant);
                    let payload = self.field_ptr(&enum_ty, dest, 1);
                    for index in source_order(values.iter()) {
                        let field = self.field_ptr(&variant_ty, &payload, index);
                        self.eval(&values[index], Some(&field));
                    }
                }
            }
            ExprKind::WrapSome(inner) => {
                self.emit(format!("store i1 true, ptr {dest}"));
                let llty = self.llty(ty);
                let payload = self.field_ptr(&llty, dest, 1);
                self.eval(inner, Some(&payload));
            }
            ExprKind::NoneLit => self.emit(format!("store i1 false, ptr {dest}")),
            ExprKind::WrapOk(inner) => {
                self.emit(format!("store ptr null, ptr {dest}"));
                let llty = self.llty(ty);
                let payload = self.field_ptr(&llty, dest, 1);
                self.eval(inner, Some(&payload));
            }
            ExprKind::Call { func, args } => {
                self.gen_call(*func, args, Some(dest));
            }
            ExprKind::CallValue { callee, args } => {
                self.gen_call_value(callee, args, ty, Some(dest));
            }
            ExprKind::Try(inner) => {
                self.gen_try(inner, ty, Some(dest));
            }
            ExprKind::Builtin(builtin, args) => {
                self.gen_builtin(*builtin, args, expr, Some(dest));
            }
            other => unreachable!("not an aggregate expression: {other:?}"),
        }
    }

    /// Evaluates an expression of a scalar type and returns the owned value.
    pub(super) fn gen_scalar(&mut self, expr: &Expr) -> String {
        match &expr.kind {
            ExprKind::Int(value) => value.to_string(),
            ExprKind::Float(value) => float_literal(*value),
            ExprKind::Bool(value) => value.to_string(),
            ExprKind::Str(text) => self.string(text),
            ExprKind::Unit => "zeroinitializer".to_string(),
            ExprKind::Retype(inner) => self.gen_scalar(inner),
            ExprKind::Local(_) | ExprKind::Field(..) | ExprKind::Index(..) => {
                let place = self.addr(expr);
                let value = self.load(&expr.ty, &place);
                self.retain_value(&expr.ty, &value);
                value
            }
            ExprKind::Unary(UnOp::Not, inner) => {
                let value = self.gen_scalar(inner);
                self.value(format!("xor i1 {value}, true"))
            }
            ExprKind::Unary(UnOp::Neg, inner) => {
                let value = self.gen_scalar(inner);
                if expr.ty == Ty::Float {
                    self.value(format!("fneg double {value}"))
                } else {
                    self.arithmetic(BinOp::Sub, &Ty::Int, "0", &value, expr.span)
                }
            }
            ExprKind::Binary(op, lhs, rhs) => self.binary(*op, lhs, rhs, expr.span),
            ExprKind::And(lhs, rhs) | ExprKind::Or(lhs, rhs) => {
                let is_and = matches!(expr.kind, ExprKind::And(..));
                let result = self.alloca(&Ty::Bool, false);
                let first = self.gen_scalar(lhs);
                self.emit(format!("store i1 {first}, ptr {result}"));
                let more = self.new_label("rhs");
                let end = self.new_label("short");
                if is_and {
                    self.cond_br(&first, &more, &end);
                } else {
                    self.cond_br(&first, &end, &more);
                }
                self.label(&more);
                self.push_scope(ScopeKind::Statement);
                let second = self.gen_scalar(rhs);
                self.emit(format!("store i1 {second}, ptr {result}"));
                self.pop_scope();
                self.label(&end);
                self.value(format!("load i1, ptr {result}"))
            }
            ExprKind::Call { func, args } => self.gen_call(*func, args, None),
            ExprKind::CallValue { callee, args } => self.gen_call_value(callee, args, &expr.ty, None),
            ExprKind::Closure { func, captures, block } => self.closure(*func, captures, *block),
            ExprKind::FnRef(func) => self.fn_ref(*func),
            ExprKind::Location => {
                let (file, line) = self.sources.location(expr.span.start);
                let text = format!("{file}:{line}");
                self.string(&text)
            }
            // Marks the place a panic comes back to (`koo-rt/src/actor.rs`). The mark is
            // of this very function, so it is the C library's `_setjmp` that is called,
            // here and not from the support library: it is true now, and false when a
            // panic has come back.
            ExprKind::Intrinsic { name: "guard", .. } => {
                let mark = self.value("call ptr @koo_guard_enter()");
                let came_back = self.value(format!("call i32 @_setjmp(ptr {mark})"));
                self.value(format!("icmp eq i32 {came_back}, 0"))
            }
            // Names what a letter holds first, and does nothing (`derive.rs`).
            ExprKind::Intrinsic {
                name: "actor_letter", ..
            } => "zeroinitializer".to_string(),
            // What crosses from one actor to another is frozen first, where it is.
            ExprKind::Intrinsic { name: "freeze", args } => {
                let place = if is_place(&args[0]) {
                    self.addr(&args[0])
                } else {
                    self.eval_temp(&args[0])
                };
                self.freeze_at(&args[0].ty, &place);
                "zeroinitializer".to_string()
            }
            ExprKind::Intrinsic { name, args } => {
                let mut operands = Vec::new();
                // What the support library keeps is moved to it: the place it was in is
                // left empty, so that nothing here frees it.
                let given = crate::intrinsics::INTRINSICS
                    .iter()
                    .find(|intrinsic| intrinsic.name == *name)
                    .and_then(|intrinsic| intrinsic.params.iter().position(|param| *param == I::Given));
                let mut emptied = None;
                for (index, arg) in args.iter().enumerate() {
                    if given == Some(index) {
                        let place = if is_place(arg) {
                            self.addr(arg)
                        } else {
                            self.eval_temp(arg)
                        };
                        let value = self.load(&arg.ty, &place);
                        operands.push(format!("ptr {value}"));
                        emptied = Some((arg.ty.clone(), place));
                        continue;
                    }
                    // An argument that a later one may change is held, not just read.
                    let value = if is_place(arg) && args[index + 1..].iter().any(may_write) {
                        self.own_operand(arg)
                    } else {
                        self.borrow(arg)
                    };
                    let llty = match arg.ty {
                        Ty::Bool => "i1 zeroext".to_string(),
                        _ => self.llty(&arg.ty),
                    };
                    operands.push(format!("{llty} {value}"));
                }
                let operands = operands.join(", ");
                let result = match &expr.ty {
                    Ty::Unit => {
                        self.emit(format!("call void @koo_{name}({operands})"));
                        "zeroinitializer".to_string()
                    }
                    Ty::Bool => self.value(format!("call zeroext i1 @koo_{name}({operands})")),
                    ty => {
                        let llty = self.llty(ty);
                        self.value(format!("call {llty} @koo_{name}({operands})"))
                    }
                };
                if let Some((ty, place)) = emptied {
                    self.zero(&ty, &place);
                }
                result
            }
            ExprKind::Builtin(builtin, args) => self.gen_builtin(*builtin, args, expr, None),
            ExprKind::ErrorLit(id, values) => {
                if values.is_empty() {
                    return format!("@\"error.{}\"", self.p.errors[*id].name);
                }
                let box_ty = self.error_box_ty(*id);
                let slot = self.temp(&expr.ty);
                let size = format!("ptrtoint (ptr getelementptr ({box_ty}, ptr null, i32 1) to i64)");
                let value = self.value(format!("call ptr @koo_alloc_zeroed(i64 {size})"));
                self.emit(format!("store i64 1, ptr {value}, !tbaa !2"));
                let id_ptr = self.field_ptr(&box_ty, &value, 1);
                self.emit(format!("store i64 {id}, ptr {id_ptr}, !tbaa !2"));
                self.emit(format!("store ptr {value}, ptr {slot}"));
                for index in source_order(values.iter()) {
                    let place = self.field_ptr(&box_ty, &value, index + 2);
                    self.eval(&values[index], Some(&place));
                }
                self.take(&expr.ty, &slot)
            }
            ExprKind::ListLit(values) => {
                if values.is_empty() {
                    return "@koo.empty".to_string();
                }
                let Ty::List(element) = &expr.ty else {
                    return "@koo.empty".to_string();
                };
                let slot = self.temp(&expr.ty);
                let size = self.sizeof(element);
                let list = self.value(format!("call ptr @koo_list_new(i64 {}, i64 {size})", values.len()));
                self.emit(format!("store ptr {list}, ptr {slot}"));
                let len_ptr = self.value(format!("getelementptr inbounds i64, ptr {list}, i64 1"));
                for (index, value) in values.iter().enumerate() {
                    let place = self.element_ptr(&list, &index.to_string(), element, None);
                    if may_exit(value) && self.droppable(element) {
                        // The buffer is not zero, so a half-built element in it could not
                        // be dropped: build it where it can be, then move it in.
                        let built = self.eval_temp(value);
                        self.move_value(element, &place, &built);
                        self.zero(element, &built);
                    } else {
                        self.eval(value, Some(&place));
                    }
                    // The length grows with each element, so that a list left half-built
                    // by an early exit drops exactly what it holds.
                    self.emit(format!("store i64 {}, ptr {len_ptr}, !tbaa !2", index + 1));
                }
                self.take(&expr.ty, &slot)
            }
            ExprKind::Interp(parts) => {
                let slot = self.temp(&Ty::Str);
                let hint: usize = parts
                    .iter()
                    .map(|part| match part {
                        InterpPart::Lit(text) => text.len(),
                        InterpPart::Value(..) => 16,
                    })
                    .sum();
                let builder = self.value(format!("call ptr @koo_str_new(i64 {hint})"));
                self.emit(format!("store ptr {builder}, ptr {slot}"));
                for part in parts {
                    match part {
                        InterpPart::Lit(text) => self.append_text(&slot, text),
                        InterpPart::Value(value, format) if format.width == 0 => {
                            self.append_value(&slot, value, format.precision)
                        }
                        InterpPart::Value(value, format) => {
                            // The text of the value by itself, then padded to the width.
                            let piece = self.temp(&Ty::Str);
                            let empty = self.value("call ptr @koo_str_new(i64 16)");
                            self.emit(format!("store ptr {empty}, ptr {piece}"));
                            self.append_value(&piece, value, format.precision);
                            let text = self.load(&Ty::Str, &piece);
                            let number = matches!(value.ty, Ty::Int | Ty::Float | Ty::Byte) || format.number;
                            let align = match format.align {
                                Align::Left => 0,
                                Align::Right => 1,
                                Align::Center => 2,
                                Align::Natural => number as i32,
                            };
                            self.emit(format!(
                                "call void @koo_str_append_padded(ptr {slot}, ptr {text}, i64 {}, i32 {align}, i1 zeroext {})",
                                format.width, format.zero
                            ));
                        }
                    }
                }
                self.take(&Ty::Str, &slot)
            }
            ExprKind::Try(inner) => self.gen_try(inner, &expr.ty, None),
            ExprKind::If { .. } | ExprKind::Match { .. } | ExprKind::Else { .. } => {
                if matches!(expr.ty, Ty::Unit | Ty::Never | Ty::Unknown) {
                    self.eval(expr, None);
                    return "zeroinitializer".to_string();
                }
                let result = self.alloca(&expr.ty, false);
                self.eval(expr, Some(&result));
                self.load(&expr.ty, &result)
            }
            ExprKind::Return(_) | ExprKind::Fail(_) | ExprKind::Break | ExprKind::Continue | ExprKind::Panic(_) => {
                self.eval(expr, None);
                "poison".to_string()
            }
            ExprKind::StructLit(..)
            | ExprKind::TupleLit(_)
            | ExprKind::EnumLit(..)
            | ExprKind::WrapSome(_)
            | ExprKind::WrapOk(_)
            | ExprKind::NoneLit => {
                // An aggregate of a type that failed to check; nothing to compute.
                "poison".to_string()
            }
        }
    }

    /// Moves the value out of a slot, leaving the slot empty.
    pub(super) fn take(&mut self, ty: &Ty, slot: &str) -> String {
        let value = self.load(ty, slot);
        if self.droppable(ty) {
            self.zero(ty, slot);
        }
        value
    }

    // ---- operators ----------------------------------------------------------------------

    /// Checked arithmetic on two values of one numeric type (Section 4.2).
    pub(super) fn arithmetic(&mut self, op: BinOp, ty: &Ty, a: &str, b: &str, span: Span) -> String {
        if *ty == Ty::Float {
            let instruction = match op {
                BinOp::Add => "fadd",
                BinOp::Sub => "fsub",
                BinOp::Mul => "fmul",
                BinOp::Div => "fdiv",
                _ => "frem",
            };
            return self.value(format!("{instruction} double {a}, {b}"));
        }
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul => {
                let intrinsic = match op {
                    BinOp::Add => "sadd",
                    BinOp::Sub => "ssub",
                    _ => "smul",
                };
                let pair = self.value(format!(
                    "call {{ i64, i1 }} @llvm.{intrinsic}.with.overflow.i64(i64 {a}, i64 {b})"
                ));
                let result = self.value(format!("extractvalue {{ i64, i1 }} {pair}, 0"));
                let overflow = self.value(format!("extractvalue {{ i64, i1 }} {pair}, 1"));
                let ok = self.value(format!("xor i1 {overflow}, true"));
                self.check(&ok, "integer overflow", span);
                result
            }
            _ => {
                let nonzero = self.value(format!("icmp ne i64 {b}, 0"));
                self.check(&nonzero, "division by zero", span);
                // The one quotient that does not fit: the smallest integer over -1.
                let minus_one = self.value(format!("icmp eq i64 {b}, -1"));
                if op == BinOp::Div {
                    let smallest = self.value(format!("icmp eq i64 {a}, -9223372036854775808"));
                    let overflow = self.value(format!("and i1 {minus_one}, {smallest}"));
                    let ok = self.value(format!("xor i1 {overflow}, true"));
                    self.check(&ok, "integer overflow", span);
                    self.value(format!("sdiv i64 {a}, {b}"))
                } else {
                    // `x % -1` is zero for every `x`; computing it directly would trap for
                    // the smallest integer.
                    let divisor = self.value(format!("select i1 {minus_one}, i64 1, i64 {b}"));
                    self.value(format!("srem i64 {a}, {divisor}"))
                }
            }
        }
    }

    fn binary(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr, span: Span) -> String {
        let ty = lhs.ty.clone();
        // A plain enum compares by the order of its variants. One whose variants have
        // fields is ordered so too, but is equal only to a value whose fields are equal.
        if let Ty::Enum(id) = ty
            && (!matches!(op, BinOp::Eq | BinOp::Ne)
                || self.p.enums[id]
                    .variants
                    .iter()
                    .all(|variant| variant.fields.is_empty()))
        {
            let a = self.addr(lhs);
            let a = self.value(format!("load i64, ptr {a}"));
            let b = self.addr(rhs);
            let b = self.value(format!("load i64, ptr {b}"));
            return self.compare(op, "i64", true, &a, &b);
        }
        if matches!(op, BinOp::Eq | BinOp::Ne)
            && !matches!(
                ty,
                Ty::Int | Ty::Float | Ty::Str | Ty::Byte | Ty::Bool | Ty::Never | Ty::Unknown
            )
        {
            // `==` is structural (Section 3.4): the two values are compared part by part.
            let a = if may_write(rhs) && is_place(lhs) {
                self.eval_temp(lhs)
            } else {
                self.addr(lhs)
            };
            let b = self.addr(rhs);
            let eq = self.glue(Glue::Eq(ty.clone()));
            let same = self.value(format!("call i1 {eq}(ptr {a}, ptr {b})"));
            return if op == BinOp::Eq {
                same
            } else {
                self.value(format!("xor i1 {same}, true"))
            };
        }
        let a = if may_write(rhs) && is_place(lhs) {
            self.own_operand(lhs)
        } else {
            self.borrow(lhs)
        };
        let b = self.borrow(rhs);
        match ty {
            Ty::Str => match op {
                BinOp::Add => self.value(format!("call ptr @koo_str_concat(ptr {a}, ptr {b})")),
                BinOp::Eq => self.value(format!("call zeroext i1 @koo_str_eq(ptr {a}, ptr {b})")),
                BinOp::Ne => {
                    let equal = self.value(format!("call zeroext i1 @koo_str_eq(ptr {a}, ptr {b})"));
                    self.value(format!("xor i1 {equal}, true"))
                }
                _ => {
                    let order = self.value(format!("call i32 @koo_str_cmp(ptr {a}, ptr {b})"));
                    self.compare(op, "i32", true, &order, "0")
                }
            },
            Ty::Float => match op {
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => {
                    self.arithmetic(op, &ty, &a, &b, span)
                }
                _ => {
                    // IEEE 754 comparisons: every one is false for NaN except `!=`.
                    let predicate = match op {
                        BinOp::Eq => "oeq",
                        BinOp::Ne => "une",
                        BinOp::Lt => "olt",
                        BinOp::Le => "ole",
                        BinOp::Gt => "ogt",
                        _ => "oge",
                    };
                    self.value(format!("fcmp {predicate} double {a}, {b}"))
                }
            },
            Ty::Int => match op {
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => {
                    self.arithmetic(op, &ty, &a, &b, span)
                }
                _ => self.compare(op, "i64", true, &a, &b),
            },
            Ty::Byte => self.compare(op, "i8", false, &a, &b),
            Ty::Bool => self.compare(op, "i1", false, &a, &b),
            _ => "poison".to_string(),
        }
    }

    fn compare(&mut self, op: BinOp, llty: &str, signed: bool, a: &str, b: &str) -> String {
        let predicate = match (op, signed) {
            (BinOp::Eq, _) => "eq",
            (BinOp::Ne, _) => "ne",
            (BinOp::Lt, true) => "slt",
            (BinOp::Le, true) => "sle",
            (BinOp::Gt, true) => "sgt",
            (BinOp::Ge, true) => "sge",
            (BinOp::Lt, false) => "ult",
            (BinOp::Le, false) => "ule",
            (BinOp::Gt, false) => "ugt",
            _ => "uge",
        };
        self.value(format!("icmp {predicate} {llty} {a}, {b}"))
    }

    // ---- strings ------------------------------------------------------------------------

    fn bytes_constant(&mut self, text: &str) -> usize {
        match self.messages.iter().position(|m| m == text) {
            Some(id) => id,
            None => {
                self.messages.push(text.to_string());
                self.messages.len() - 1
            }
        }
    }

    /// Appends fixed text to the string being built in `slot`.
    pub(super) fn append_text(&mut self, slot: &str, text: &str) {
        if text.is_empty() {
            return;
        }
        let id = self.bytes_constant(text);
        self.emit(format!(
            "call void @koo_str_append_bytes(ptr {slot}, ptr @msg.{id}, i64 {})",
            text.len()
        ));
    }

    /// Appends the text form of a value to the string being built in `slot`.
    fn append_value(&mut self, slot: &str, value: &Expr, precision: Option<u32>) {
        match &value.ty {
            Ty::Int => {
                let v = self.gen_scalar(value);
                self.emit(format!("call void @koo_str_append_int(ptr {slot}, i64 {v})"));
            }
            Ty::Byte => {
                let v = self.gen_scalar(value);
                let wide = self.value(format!("zext i8 {v} to i64"));
                self.emit(format!("call void @koo_str_append_int(ptr {slot}, i64 {wide})"));
            }
            Ty::Float => {
                let v = self.gen_scalar(value);
                let precision = precision.map(|p| p as i64).unwrap_or(-1);
                self.emit(format!(
                    "call void @koo_str_append_float(ptr {slot}, double {v}, i32 {precision})"
                ));
            }
            Ty::Bool => {
                let v = self.gen_scalar(value);
                self.emit(format!("call void @koo_str_append_bool(ptr {slot}, i1 zeroext {v})"));
            }
            Ty::Str => {
                let v = self.borrow(value);
                self.emit(format!("call void @koo_str_append(ptr {slot}, ptr {v})"));
            }
            Ty::Unit | Ty::Never | Ty::Unknown => {
                self.eval(value, None);
            }
            ty => {
                let place = self.addr(value);
                self.append_at(slot, ty, &place);
            }
        }
    }

    /// Appends the text form of the value at `place`. A string is quoted only inside a
    /// struct, enum or list; an optional is not one of those, it is its value or `none`.
    fn append_at(&mut self, slot: &str, ty: &Ty, place: &str) {
        match ty {
            Ty::Str => {
                let text = self.value(format!("load ptr, ptr {place}"));
                self.emit(format!("call void @koo_str_append(ptr {slot}, ptr {text})"));
            }
            Ty::Optional(inner) => {
                let llty = self.llty(ty);
                let present = self.value(format!("load i1, ptr {place}"));
                let some = self.new_label("some");
                let none = self.new_label("none");
                let end = self.new_label("shown");
                self.cond_br(&present, &some, &none);
                self.label(&some);
                let payload = self.field_ptr(&llty, place, 1);
                self.append_at(slot, inner, &payload);
                self.br(&end);
                self.label(&none);
                self.append_text(slot, "none");
                self.label(&end);
            }
            _ => {
                let fmt = self.glue(Glue::Fmt(ty.clone()));
                self.emit(format!("call void {fmt}(ptr {slot}, ptr {place})"));
            }
        }
    }

    // ---- calls --------------------------------------------------------------------------

    fn gen_call(&mut self, func: FnId, args: &[Arg], dest: Option<&str>) -> String {
        let ret = self.p.functions[func].ret.clone();
        let name = self.fn_name(func);
        self.call(&name, None, args, &ret, dest)
    }

    /// A call of a function value: its code, with the value itself as the hidden argument.
    fn gen_call_value(&mut self, callee: &Expr, args: &[Arg], ret: &Ty, dest: Option<&str>) -> String {
        // The function being called must outlive the call. A variable that cannot be
        // assigned holds it for that long; anything else (a `var`, a field, an element)
        // may be overwritten by the call itself, through a `var` argument or a block, so
        // the call keeps a reference of its own.
        let locals = &self.p.functions[self.f.id].locals;
        let stable = matches!(&callee.kind, ExprKind::Local(local) if !locals[*local].mutable);
        let value = if stable {
            self.borrow(callee)
        } else {
            self.own_operand(callee)
        };
        let code_ptr = self.value(format!("getelementptr inbounds ptr, ptr {value}, i64 1"));
        // The code of a function value never changes once the value is made. Saying so
        // lets LLVM see which function a block passed to an inlined callee is, and then
        // inline it too.
        let code = self.value(format!("load ptr, ptr {code_ptr}, !invariant.load !13"));
        self.call(&code, Some(&value), args, ret, dest)
    }

    /// Emits a call. `env` is the function value being called, if the call is of one.
    fn call(&mut self, target: &str, env: Option<&str>, args: &[Arg], ret: &Ty, dest: Option<&str>) -> String {
        let ret = ret.clone();
        // What the call may change: its `var` arguments, and whatever a block passed to it
        // uses, since a block works on the caller's variables (Section 5.4).
        let mut var_roots: Vec<LocalId> = args
            .iter()
            .filter(|arg| arg.kind == ParamKind::Var)
            .filter_map(|arg| root_local(&arg.expr))
            .collect();
        for arg in args {
            if let ExprKind::Closure {
                captures, block: true, ..
            } = &arg.expr.kind
            {
                var_roots.extend(captures.iter().copied());
            }
        }
        // A function value that is not a block written here may be a block written in the
        // enclosing function, and that one works on the enclosing function's variables:
        // the ones this function takes by reference.
        let locals = &self.p.functions[self.f.id].locals;
        let foreign = env.is_some()
            || args.iter().any(|arg| match &arg.expr.kind {
                // A block written here reaches further only through a function it uses.
                ExprKind::Closure {
                    captures, block: true, ..
                } => captures.iter().any(|&capture| matches!(locals[capture].ty, Ty::Fn(_))),
                _ => matches!(arg.expr.ty, Ty::Fn(_)),
            });
        if foreign {
            var_roots.extend(
                (0..locals.len())
                    .filter(|&id| locals[id].mutable && locals[id].kind == LocalKind::Capture { by_ref: true }),
            );
        }
        let mut operands: Vec<(String, String)> = vec![(String::new(), String::new()); args.len()];
        // Values that move into the callee: taken from their slots just before the call.
        let mut moved: Vec<(usize, String, Ty)> = Vec::new();
        let mut emptied: Vec<(String, Ty)> = Vec::new();
        // The index expressions of `var` arguments, evaluated in their turn.
        let mut var_indices: Vec<Option<Vec<String>>> = vec![None; args.len()];
        // Arguments are evaluated as written, left to right, whatever order the parameters
        // are declared in (Section 5.4).
        let order = source_order(args.iter().map(|arg| &arg.expr));
        for (position, &index) in order.iter().enumerate() {
            let arg = &args[index];
            let ty = &arg.expr.ty;
            let llty = if is_aggregate(ty) {
                "ptr".to_string()
            } else {
                self.llty(ty)
            };
            match arg.kind {
                ParamKind::Var => var_indices[index] = Some(self.place_indices(&arg.expr)),
                ParamKind::Borrow => {
                    // A read that overlaps a write is copied first (Section 5.4): the
                    // callee may change a `var` argument that this argument is part of,
                    // and a later argument may change it before the call.
                    let overlaps = root_local(&arg.expr).is_some_and(|root| var_roots.contains(&root));
                    let changed_later = order[position + 1..].iter().any(|&later| may_write(&args[later].expr));
                    let value = if is_place(&arg.expr) && (overlaps || changed_later) {
                        self.own_operand(&arg.expr)
                    } else {
                        self.borrow(&arg.expr)
                    };
                    operands[index] = (llty, value);
                }
                ParamKind::Take => {
                    if is_aggregate(ty) {
                        let slot = self.eval_temp(&arg.expr);
                        if self.droppable(ty) {
                            emptied.push((slot.clone(), ty.clone()));
                        }
                        operands[index] = (llty, slot);
                    } else if self.droppable(ty) {
                        let slot = self.eval_temp(&arg.expr);
                        moved.push((index, slot, ty.clone()));
                        operands[index].0 = llty;
                    } else {
                        let value = self.gen_scalar(&arg.expr);
                        operands[index] = (llty, value);
                    }
                }
            }
        }
        // The addresses of `var` arguments come last: evaluating another argument may move
        // the storage they point into.
        for (index, arg) in args.iter().enumerate() {
            if let Some(indices) = var_indices[index].take() {
                operands[index] = ("ptr".to_string(), self.addr_mut_at(&arg.expr, &mut indices.into_iter()));
            }
        }
        for (index, slot, ty) in moved {
            operands[index].1 = self.take(&ty, &slot);
        }

        let mut list: Vec<String> = Vec::new();
        let temp;
        if is_aggregate(&ret) {
            let dest = match dest {
                Some(dest) => dest,
                None => {
                    temp = self.temp(&ret);
                    &temp
                }
            };
            list.push(format!("ptr {dest}"));
        }
        if let Some(env) = env {
            list.push(format!("ptr {env}"));
        }
        list.extend(operands.iter().map(|(llty, value)| format!("{llty} {value}")));
        let name = target;
        let result = if is_aggregate(&ret) || matches!(ret, Ty::Unit | Ty::Never | Ty::Unknown) {
            self.emit(format!("call void {name}({})", list.join(", ")));
            "zeroinitializer".to_string()
        } else {
            let llty = self.llty(&ret);
            self.value(format!("call {llty} {name}({})", list.join(", ")))
        };
        // The callee dropped what it took; the slots it took from are empty now.
        for (slot, ty) in emptied {
            self.zero(&ty, &slot);
        }
        result
    }

    fn gen_return(&mut self, value: Option<&Expr>) {
        let ret = self.f.ret.clone();
        let Some(value) = value else {
            self.cleanup_from(0);
            self.terminate("ret void");
            return;
        };
        // A local that the function owns is moved out, not copied.
        let movable = match &value.kind {
            ExprKind::Local(id) if self.droppable(&ret) => {
                let local = &self.p.functions[self.f.id].locals[*id];
                matches!(local.kind, LocalKind::Owned | LocalKind::Param(ParamKind::Take))
                    .then(|| self.f.locals[*id].clone())
                    .flatten()
            }
            _ => None,
        };
        if is_aggregate(&ret) {
            if let Some(slot) = movable {
                self.memcpy(&ret, "%ret", &slot);
                self.zero(&ret, &slot);
            } else if may_exit(value) {
                // An early exit from the middle of the expression must not leave a
                // half-built value in the caller's storage.
                let temp = self.eval_temp(value);
                self.memcpy(&ret, "%ret", &temp);
                if self.droppable(&ret) {
                    self.zero(&ret, &temp);
                }
            } else {
                self.eval(value, Some("%ret"));
            }
            self.cleanup_from(0);
            self.terminate("ret void");
        } else if matches!(ret, Ty::Unit | Ty::Never | Ty::Unknown) {
            self.eval(value, None);
            self.cleanup_from(0);
            self.terminate("ret void");
        } else {
            let result = match movable {
                Some(slot) => self.take(&ret, &slot),
                None => self.gen_scalar(value),
            };
            self.cleanup_from(0);
            let llty = self.llty(&ret);
            self.terminate(format!("ret {llty} {result}"));
        }
    }

    // ---- `?`, `else` and `match` ---------------------------------------------------------

    /// `x?`: the value inside an optional or a result, or an early return (Section 6.4).
    fn gen_try(&mut self, inner: &Expr, ty: &Ty, dest: Option<&str>) -> String {
        let slot = self.eval_temp(inner);
        let llty = self.llty(&inner.ty);
        let leave = self.new_label("propagate");
        let stay = self.new_label("value");
        match &inner.ty {
            Ty::Result(..) => {
                let error = self.value(format!("load ptr, ptr {slot}"));
                let failed = self.value(format!("icmp ne ptr {error}, null"));
                self.cond_br(&failed, &leave, &stay);
                self.label(&leave);
                // The error moves into the function's own result.
                self.emit(format!("store ptr {error}, ptr %ret"));
                self.zero(&inner.ty, &slot);
            }
            _ => {
                let present = self.value(format!("load i1, ptr {slot}"));
                self.cond_br(&present, &stay, &leave);
                self.label(&leave);
                self.emit("store i1 false, ptr %ret");
            }
        }
        self.cleanup_from(0);
        self.terminate("ret void");
        self.label(&stay);
        let payload = self.field_ptr(&llty, &slot, 1);
        let result = if is_aggregate(ty) {
            let temp;
            let dest = match dest {
                Some(dest) => dest,
                None => {
                    temp = self.temp(ty);
                    &temp
                }
            };
            self.memcpy(ty, dest, &payload);
            "zeroinitializer".to_string()
        } else {
            self.load(ty, &payload)
        };
        if self.droppable(&inner.ty) {
            self.zero(&inner.ty, &slot);
        }
        result
    }

    /// `x else alt`, with the result at `dest`.
    fn gen_else(&mut self, inner: &Expr, binding: Option<LocalId>, alt: &Block, dest: Option<&str>, whole: &Ty) {
        let slot = self.eval_temp(inner);
        let llty = self.llty(&inner.ty);
        let (ok, is_result) = match &inner.ty {
            Ty::Result(ok, _) => ((**ok).clone(), true),
            Ty::Optional(inner) => ((**inner).clone(), false),
            _ => (Ty::Unknown, false),
        };
        let have = self.new_label("have");
        let lack = self.new_label("lack");
        let end = self.new_label("else.end");
        if is_result {
            let error = self.value(format!("load ptr, ptr {slot}"));
            let failed = self.value(format!("icmp ne ptr {error}, null"));
            self.cond_br(&failed, &lack, &have);
        } else {
            let present = self.value(format!("load i1, ptr {slot}"));
            self.cond_br(&present, &have, &lack);
        }
        self.label(&have);
        if let Some(dest) = dest {
            let payload = self.field_ptr(&llty, &slot, 1);
            // Where the alternative is an optional or a result of what the operand
            // holds, so is the whole, and what the operand held is one that is there.
            let dest = match whole {
                Ty::Optional(held) if **held == ok && *whole != ok => {
                    self.emit(format!("store i1 true, ptr {dest}"));
                    let wider = self.llty(whole);
                    self.field_ptr(&wider, dest, 1)
                }
                Ty::Result(held, _) if **held == ok && *whole != ok => {
                    self.emit(format!("store ptr null, ptr {dest}"));
                    let wider = self.llty(whole);
                    self.field_ptr(&wider, dest, 1)
                }
                _ => dest.to_string(),
            };
            let dest = dest.as_str();
            self.move_value(&ok, dest, &payload);
            if self.droppable(&inner.ty) {
                self.zero(&inner.ty, &slot);
            }
        }
        self.br(&end);
        self.label(&lack);
        self.push_scope(ScopeKind::Block);
        if let Some(local) = binding {
            // The binding takes the error from the result.
            let place = self.declare(local);
            if is_result {
                let error = self.value(format!("load ptr, ptr {slot}"));
                self.emit(format!("store ptr {error}, ptr {place}"));
                self.zero(&inner.ty, &slot);
            }
        }
        self.block_body(alt, dest);
        self.pop_scope();
        self.label(&end);
    }

    fn gen_match(&mut self, scrutinee: &Expr, arms: &[Arm], dest: Option<&str>) {
        // The scrutinee is evaluated once. A guard that may change it must not be seen
        // by the arms after it, so then the match works on a copy.
        let guard_writes = arms.iter().any(|arm| arm.guard.as_ref().is_some_and(may_write));
        let place = if guard_writes && is_place(scrutinee) {
            self.eval_temp(scrutinee)
        } else {
            self.addr(scrutinee)
        };
        let ty = scrutinee.ty.clone();
        let end = self.new_label("match.end");
        for arm in arms {
            let next = self.new_label("arm.next");
            let body = self.new_label("arm");
            self.push_scope(ScopeKind::Block);
            let scope = self.f.scopes.len() - 1;
            for (index, pattern) in arm.patterns.iter().enumerate() {
                let last = index + 1 == arm.patterns.len();
                let fail = if last { next.clone() } else { self.new_label("alt") };
                self.test(pattern, &ty, &place, &fail);
                self.br(&body);
                if !last {
                    self.label(&fail);
                }
            }
            self.label(&body);
            // A name bound by a pattern holds a copy of the part it matched (Section 6.6).
            if let [pattern] = arm.patterns.as_slice() {
                self.bind(pattern, &ty, &place);
            }
            if let Some(guard) = &arm.guard {
                self.push_scope(ScopeKind::Statement);
                let holds = self.gen_scalar(guard);
                self.pop_scope();
                let pass = self.new_label("guard");
                let skip = self.new_label("guard.no");
                self.cond_br(&holds, &pass, &skip);
                self.label(&skip);
                self.cleanup_from(scope);
                self.br(&next);
                self.label(&pass);
            }
            self.block_body(&arm.body, dest);
            self.pop_scope();
            self.br(&end);
            self.label(&next);
        }
        // The checker made the match exhaustive.
        self.terminate("unreachable");
        self.label(&end);
    }

    fn test_or(&mut self, holds: &str, fail: &str) {
        let next = self.new_label("test");
        self.cond_br(holds, &next, fail);
        self.label(&next);
    }

    /// Branches to `fail` unless the value at `place` matches the pattern.
    fn test(&mut self, pattern: &Pattern, ty: &Ty, place: &str, fail: &str) {
        let llty = self.llty(ty);
        match pattern {
            Pattern::Wildcard | Pattern::Bind(_) => {}
            Pattern::Int(n) => {
                let value = self.load(ty, place);
                let holds = self.value(format!("icmp eq {llty} {value}, {n}"));
                self.test_or(&holds, fail);
            }
            Pattern::IntRange(lo, hi) => {
                let mut value = self.load(ty, place);
                if *ty == Ty::Byte {
                    value = self.value(format!("zext i8 {value} to i64"));
                }
                let above = self.value(format!("icmp sge i64 {value}, {lo}"));
                self.test_or(&above, fail);
                let below = self.value(format!("icmp sle i64 {value}, {hi}"));
                self.test_or(&below, fail);
            }
            Pattern::Bool(b) => {
                let value = self.load(ty, place);
                let holds = self.value(format!("icmp eq i1 {value}, {b}"));
                self.test_or(&holds, fail);
            }
            Pattern::Str(text) => {
                let value = self.load(ty, place);
                let literal = self.string(text);
                let holds = self.value(format!("call zeroext i1 @koo_str_eq(ptr {value}, ptr {literal})"));
                self.test_or(&holds, fail);
            }
            Pattern::None => {
                let present = self.value(format!("load i1, ptr {place}"));
                let absent = self.value(format!("xor i1 {present}, true"));
                self.test_or(&absent, fail);
            }
            Pattern::Some(inner) => {
                let Ty::Optional(inner_ty) = ty else { return };
                let present = self.value(format!("load i1, ptr {place}"));
                self.test_or(&present, fail);
                let payload = self.field_ptr(&llty, place, 1);
                self.test(inner, inner_ty, &payload, fail);
            }
            Pattern::Ok(inner) => {
                let Ty::Result(ok, _) = ty else { return };
                let error = self.value(format!("load ptr, ptr {place}"));
                let succeeded = self.value(format!("icmp eq ptr {error}, null"));
                self.test_or(&succeeded, fail);
                let payload = self.field_ptr(&llty, place, 1);
                self.test(inner, ok, &payload, fail);
            }
            Pattern::Err { error, fields, .. } => {
                let value = self.value(format!("load ptr, ptr {place}"));
                let failed = self.value(format!("icmp ne ptr {value}, null"));
                self.test_or(&failed, fail);
                if let Some(id) = error {
                    let id_ptr = self.value(format!("getelementptr inbounds i64, ptr {value}, i64 1"));
                    let found = self.value(format!("load i64, ptr {id_ptr}, !tbaa !2"));
                    let same = self.value(format!("icmp eq i64 {found}, {id}"));
                    self.test_or(&same, fail);
                    let box_ty = self.error_box_ty(*id);
                    for (index, pattern) in fields {
                        let field_ty = self.p.errors[*id].fields[*index].ty.clone();
                        let field = self.field_ptr(&box_ty, &value, index + 2);
                        self.test(pattern, &field_ty, &field, fail);
                    }
                }
            }
            Pattern::Variant { variant, fields } => {
                let Ty::Enum(id) = ty else { return };
                let tag = self.value(format!("load i64, ptr {place}"));
                let same = self.value(format!("icmp eq i64 {tag}, {variant}"));
                self.test_or(&same, fail);
                if !fields.is_empty() {
                    let variant_ty = self.variant_ty(*id, *variant);
                    let payload = self.field_ptr(&llty, place, 1);
                    for (index, pattern) in fields {
                        let field_ty = self.p.enums[*id].variants[*variant].fields[*index].ty.clone();
                        let field = self.field_ptr(&variant_ty, &payload, *index);
                        self.test(pattern, &field_ty, &field, fail);
                    }
                }
            }
            Pattern::Struct { fields } => {
                let Ty::Struct(id) = ty else { return };
                for (index, pattern) in fields {
                    let field_ty = self.p.structs[*id].fields[*index].ty.clone();
                    let field = self.field_ptr(&llty, place, *index);
                    self.test(pattern, &field_ty, &field, fail);
                }
            }
            Pattern::Tuple(patterns) => {
                let Ty::Tuple(types) = ty else { return };
                for (index, (pattern, element)) in patterns.iter().zip(types).enumerate() {
                    let field = self.field_ptr(&llty, place, index);
                    self.test(pattern, element, &field, fail);
                }
            }
        }
    }

    /// Declares the names a matched pattern binds, each holding a copy of its part.
    fn bind(&mut self, pattern: &Pattern, ty: &Ty, place: &str) {
        let llty = self.llty(ty);
        match pattern {
            Pattern::Bind(local) => {
                let slot = self.declare(*local);
                if is_aggregate(ty) {
                    self.memcpy(ty, &slot, place);
                    self.retain_at(ty, &slot);
                } else {
                    let value = self.load(ty, place);
                    self.retain_value(ty, &value);
                    self.store(ty, &value, &slot);
                }
            }
            Pattern::Some(inner) => {
                let Ty::Optional(inner_ty) = ty else { return };
                let payload = self.field_ptr(&llty, place, 1);
                self.bind(inner, inner_ty, &payload);
            }
            Pattern::Ok(inner) => {
                let Ty::Result(ok, _) = ty else { return };
                let payload = self.field_ptr(&llty, place, 1);
                self.bind(inner, ok, &payload);
            }
            Pattern::Err { error, binding, fields } => {
                let value = self.value(format!("load ptr, ptr {place}"));
                if let Some(local) = binding {
                    let slot = self.declare(*local);
                    self.emit(format!("call void @koo.retain(ptr {value})"));
                    self.emit(format!("store ptr {value}, ptr {slot}"));
                }
                if let Some(id) = error {
                    let box_ty = self.error_box_ty(*id);
                    for (index, pattern) in fields {
                        let field_ty = self.p.errors[*id].fields[*index].ty.clone();
                        let field = self.field_ptr(&box_ty, &value, index + 2);
                        self.bind(pattern, &field_ty, &field);
                    }
                }
            }
            Pattern::Variant { variant, fields } => {
                let Ty::Enum(id) = ty else { return };
                if fields.is_empty() {
                    return;
                }
                let variant_ty = self.variant_ty(*id, *variant);
                let payload = self.field_ptr(&llty, place, 1);
                for (index, pattern) in fields {
                    let field_ty = self.p.enums[*id].variants[*variant].fields[*index].ty.clone();
                    let field = self.field_ptr(&variant_ty, &payload, *index);
                    self.bind(pattern, &field_ty, &field);
                }
            }
            Pattern::Struct { fields } => {
                let Ty::Struct(id) = ty else { return };
                for (index, pattern) in fields {
                    let field_ty = self.p.structs[*id].fields[*index].ty.clone();
                    let field = self.field_ptr(&llty, place, *index);
                    self.bind(pattern, &field_ty, &field);
                }
            }
            Pattern::Tuple(patterns) => {
                let Ty::Tuple(types) = ty else { return };
                for (index, (pattern, element)) in patterns.iter().zip(types).enumerate() {
                    let field = self.field_ptr(&llty, place, index);
                    self.bind(pattern, element, &field);
                }
            }
            _ => {}
        }
    }

    // ---- built-ins ----------------------------------------------------------------------

    /// Stores an error value with no fields into the result at `dest`.
    fn fail_into(&mut self, dest: &str, error: ErrorId) {
        self.emit(format!(
            "store ptr @\"error.{}\", ptr {dest}",
            self.p.errors[error].name
        ));
    }

    fn gen_builtin(&mut self, builtin: Builtin, args: &[Expr], expr: &Expr, dest: Option<&str>) -> String {
        let span = expr.span;
        let unit = "zeroinitializer".to_string();
        match builtin {
            Builtin::Print => {
                let value = &args[0];
                if value.ty == Ty::Str {
                    let text = self.borrow(value);
                    self.emit(format!("call void @koo_print(ptr {text})"));
                } else {
                    let slot = self.temp(&Ty::Str);
                    let builder = self.value("call ptr @koo_str_new(i64 32)");
                    self.emit(format!("store ptr {builder}, ptr {slot}"));
                    self.append_value(&slot, value, None);
                    let text = self.value(format!("load ptr, ptr {slot}"));
                    self.emit(format!("call void @koo_print(ptr {text})"));
                }
                unit
            }
            Builtin::Len => {
                let value = self.borrow(&args[0]);
                let len_ptr = self.value(format!("getelementptr inbounds i64, ptr {value}, i64 1"));
                self.value(format!("load i64, ptr {len_ptr}, !tbaa !2"))
            }
            Builtin::ListPush => {
                let Ty::List(element) = &args[0].ty else { return unit };
                // A list the enclosing loop already made unique needs no check here, and
                // its length is at hand.
                let len = match &args[0].kind {
                    ExprKind::Local(local) if self.f.unique.contains(local) => self.f.lens.get(local).cloned(),
                    _ => None,
                };
                let (push, len) = match len {
                    Some(len) => (self.glue(Glue::PushUnique(args[0].ty.clone())), format!("ptr {len}, ")),
                    None => (self.glue(Glue::Push(args[0].ty.clone())), String::new()),
                };
                // Left to right: the indices of the list's place, then the value. The list
                // itself is touched only when the value is complete.
                let mut indices = self.place_indices(&args[0]).into_iter();
                if is_aggregate(element) || self.droppable(element) {
                    let value = self.eval_temp(&args[1]);
                    let slot = self.addr_mut_at(&args[0], &mut indices);
                    if is_aggregate(element) {
                        self.emit(format!("call void {push}(ptr {slot}, {len}ptr {value})"));
                        if self.droppable(element) {
                            self.zero(element, &value);
                        }
                    } else {
                        let moved = self.take(element, &value);
                        self.emit(format!("call void {push}(ptr {slot}, {len}ptr {moved})"));
                    }
                } else {
                    let value = self.gen_scalar(&args[1]);
                    let slot = self.addr_mut_at(&args[0], &mut indices);
                    let llty = self.llty(element);
                    self.emit(format!("call void {push}(ptr {slot}, {len}{llty} {value})"));
                }
                unit
            }
            Builtin::ListPop => {
                let pop = self.glue(Glue::Pop(args[0].ty.clone()));
                let slot = self.addr_mut(&args[0]);
                let temp;
                let dest = match dest {
                    Some(dest) => dest,
                    None => {
                        temp = self.temp(&expr.ty);
                        &temp
                    }
                };
                self.emit(format!("call void {pop}(ptr {slot}, ptr {dest})"));
                unit
            }
            Builtin::ListGet => {
                let Ty::List(element) = args[0].ty.clone() else {
                    return unit;
                };
                let Some(dest) = dest else {
                    self.eval(&args[0], None);
                    self.eval(&args[1], None);
                    return unit;
                };
                let list = if may_write(&args[1]) {
                    self.own_operand(&args[0])
                } else {
                    self.borrow(&args[0])
                };
                let i = self.gen_scalar(&args[1]);
                let len_ptr = self.value(format!("getelementptr inbounds i64, ptr {list}, i64 1"));
                let len = self.value(format!("load i64, ptr {len_ptr}, !tbaa !2"));
                let inside = self.value(format!("icmp ult i64 {i}, {len}"));
                self.emit(format!("store i1 {inside}, ptr {dest}"));
                let copy = self.new_label("get");
                let end = self.new_label("get.end");
                self.cond_br(&inside, &copy, &end);
                self.label(&copy);
                let from = self.element_ptr(&list, &i, &element, None);
                let llty = self.llty(&expr.ty);
                let to = self.field_ptr(&llty, dest, 1);
                self.move_value(&element, &to, &from);
                self.retain_at(&element, &to);
                self.label(&end);
                unit
            }
            Builtin::ListSwap => {
                let Ty::List(element) = args[0].ty.clone() else {
                    return unit;
                };
                let mut indices = self.place_indices(&args[0]).into_iter();
                let i = self.gen_scalar(&args[1]);
                let j = self.gen_scalar(&args[2]);
                let slot = self.addr_mut_at(&args[0], &mut indices);
                let unique = self.glue(Glue::Unique(args[0].ty.clone()));
                self.emit(format!("call void {unique}(ptr {slot})"));
                let list = self.value(format!("load ptr, ptr {slot}"));
                let a = self.element_ptr(&list, &i, &element, Some(span));
                let b = self.element_ptr(&list, &j, &element, Some(span));
                let hold = self.alloca(&element, false);
                // Three moves; `a` and `b` may be the same element.
                self.move_value(&element, &hold, &a);
                if is_aggregate(&element) {
                    let size = self.sizeof(&element);
                    self.emit(format!(
                        "call void @llvm.memmove.p0.p0.i64(ptr {a}, ptr {b}, i64 {size}, i1 false)"
                    ));
                } else {
                    self.move_value(&element, &a, &b);
                }
                self.move_value(&element, &b, &hold);
                unit
            }
            Builtin::ListClear => {
                let slot = self.addr_mut(&args[0]);
                self.drop_at(&args[0].ty, &slot);
                self.emit(format!("store ptr @koo.empty, ptr {slot}"));
                unit
            }
            Builtin::ListSortInts => {
                let slot = self.addr_mut(&args[0]);
                let unique = self.glue(Glue::Unique(args[0].ty.clone()));
                self.emit(format!("call void {unique}(ptr {slot})"));
                let list = self.value(format!("load ptr, ptr {slot}"));
                self.emit(format!("call void @koo_list_sort_ints(ptr {list})"));
                unit
            }
            Builtin::ListWithSize | Builtin::ListWithCapacity => {
                let Ty::List(element) = &expr.ty else { return unit };
                let n = self.gen_scalar(&args[0]);
                let ok = self.value(format!("icmp sge i64 {n}, 0"));
                self.check(&ok, "negative size", span);
                let size = self.sizeof(element);
                // More elements than there are bytes to hold them is a panic here, at
                // the line that asked, and not in the support library, which has no
                // line to give.
                let empty = self.value(format!("icmp eq i64 {size}, 0"));
                let each = self.value(format!("select i1 {empty}, i64 1, i64 {size}"));
                let most = self.value(format!("sdiv i64 {}, {each}", i64::MAX - 64));
                let fits = self.value(format!("icmp sle i64 {n}, {most}"));
                self.check(&fits, "a list this large cannot be allocated", span);
                let function = if builtin == Builtin::ListWithSize {
                    "koo_list_zeroed"
                } else {
                    "koo_list_new"
                };
                self.value(format!("call ptr @{function}(i64 {n}, i64 {size})"))
            }
            Builtin::IntParse | Builtin::FloatParse => {
                let Ty::Result(ok, ErrSet::Of(errors)) = &expr.ty else {
                    return unit;
                };
                let error = errors[0];
                let Some(dest) = dest else {
                    self.eval(&args[0], None);
                    return unit;
                };
                let text = self.borrow(&args[0]);
                let llty = self.llty(&expr.ty);
                let out = self.field_ptr(&llty, dest, 1);
                let function = if builtin == Builtin::IntParse {
                    "koo_int_parse"
                } else {
                    "koo_float_parse"
                };
                let _ = ok;
                let parsed = self.value(format!("call zeroext i1 @{function}(ptr {text}, ptr {out})"));
                let good = self.new_label("parsed");
                let bad = self.new_label("unparsed");
                let end = self.new_label("parse.end");
                self.cond_br(&parsed, &good, &bad);
                self.label(&good);
                self.emit(format!("store ptr null, ptr {dest}"));
                self.br(&end);
                self.label(&bad);
                // ParseError { input }
                let box_ty = self.error_box_ty(error);
                let size = format!("ptrtoint (ptr getelementptr ({box_ty}, ptr null, i32 1) to i64)");
                let value = self.value(format!("call ptr @koo_alloc_zeroed(i64 {size})"));
                self.emit(format!("store i64 1, ptr {value}, !tbaa !2"));
                let id_ptr = self.field_ptr(&box_ty, &value, 1);
                self.emit(format!("store i64 {error}, ptr {id_ptr}, !tbaa !2"));
                let input = self.field_ptr(&box_ty, &value, 2);
                self.emit(format!("call void @koo.retain(ptr {text})"));
                self.emit(format!("store ptr {text}, ptr {input}"));
                self.emit(format!("store ptr {value}, ptr {dest}"));
                self.label(&end);
                unit
            }
            Builtin::FloatFromInt => {
                let value = self.gen_scalar(&args[0]);
                self.value(format!("sitofp i64 {value} to double"))
            }
            Builtin::IntFromByte => {
                let value = self.gen_scalar(&args[0]);
                self.value(format!("zext i8 {value} to i64"))
            }
            Builtin::ByteFromInt => {
                let Ty::Result(_, ErrSet::Of(errors)) = &expr.ty else {
                    return unit;
                };
                let error = errors[0];
                let value = self.gen_scalar(&args[0]);
                let Some(dest) = dest else { return unit };
                let fits = self.value(format!("icmp ult i64 {value}, 256"));
                let good = self.new_label("fits");
                let bad = self.new_label("overflow");
                let end = self.new_label("byte.end");
                self.cond_br(&fits, &good, &bad);
                self.label(&good);
                let narrow = self.value(format!("trunc i64 {value} to i8"));
                let llty = self.llty(&expr.ty);
                let out = self.field_ptr(&llty, dest, 1);
                self.emit(format!("store ptr null, ptr {dest}"));
                self.emit(format!("store i8 {narrow}, ptr {out}"));
                self.br(&end);
                self.label(&bad);
                self.fail_into(dest, error);
                self.label(&end);
                unit
            }
            Builtin::FloatSqrt => {
                let value = self.gen_scalar(&args[0]);
                self.value(format!("call double @llvm.sqrt.f64(double {value})"))
            }
            Builtin::Abs => {
                let value = self.gen_scalar(&args[0]);
                if args[0].ty == Ty::Float {
                    self.value(format!("call double @llvm.fabs.f64(double {value})"))
                } else {
                    let ok = self.value(format!("icmp ne i64 {value}, -9223372036854775808"));
                    self.check(&ok, "integer overflow", span);
                    self.value(format!("call i64 @llvm.abs.i64(i64 {value}, i1 true)"))
                }
            }
            Builtin::FloatFloor | Builtin::FloatCeil | Builtin::FloatRound | Builtin::FloatTrunc => {
                let Ty::Result(_, ErrSet::Of(errors)) = &expr.ty else {
                    return unit;
                };
                let error = errors[0];
                let value = self.gen_scalar(&args[0]);
                let Some(dest) = dest else { return unit };
                let intrinsic = match builtin {
                    Builtin::FloatFloor => "floor",
                    Builtin::FloatCeil => "ceil",
                    Builtin::FloatRound => "round",
                    _ => "trunc",
                };
                let rounded = self.value(format!("call double @llvm.{intrinsic}.f64(double {value})"));
                // In range means -2^63 <= x < 2^63; both tests are false for NaN.
                let low = self.value(format!(
                    "fcmp oge double {rounded}, {}",
                    float_literal(-9223372036854775808.0)
                ));
                let high = self.value(format!(
                    "fcmp olt double {rounded}, {}",
                    float_literal(9223372036854775808.0)
                ));
                let fits = self.value(format!("and i1 {low}, {high}"));
                let good = self.new_label("fits");
                let bad = self.new_label("overflow");
                let end = self.new_label("round.end");
                self.cond_br(&fits, &good, &bad);
                self.label(&good);
                let whole = self.value(format!("fptosi double {rounded} to i64"));
                let llty = self.llty(&expr.ty);
                let out = self.field_ptr(&llty, dest, 1);
                self.emit(format!("store ptr null, ptr {dest}"));
                self.emit(format!("store i64 {whole}, ptr {out}"));
                self.br(&end);
                self.label(&bad);
                self.fail_into(dest, error);
                self.label(&end);
                unit
            }
            Builtin::Compare => {
                let ty = args[0].ty.clone();
                // The second value may change the variable the first is read from.
                let a = if may_write(&args[1]) && is_place(&args[0]) {
                    self.own_operand(&args[0])
                } else {
                    self.borrow(&args[0])
                };
                let b = self.borrow(&args[1]);
                match ty {
                    Ty::Float => self.value(format!("call i64 @koo_float_compare(double {a}, double {b})")),
                    Ty::Str => {
                        let order = self.value(format!("call i32 @koo_str_cmp(ptr {a}, ptr {b})"));
                        self.value(format!("sext i32 {order} to i64"))
                    }
                    _ => {
                        let (llty, less, greater) = match ty {
                            Ty::Int => ("i64", "slt", "sgt"),
                            Ty::Byte => ("i8", "ult", "ugt"),
                            _ => ("i1", "ult", "ugt"),
                        };
                        let below = self.value(format!("icmp {less} {llty} {a}, {b}"));
                        let above = self.value(format!("icmp {greater} {llty} {a}, {b}"));
                        let below = self.value(format!("zext i1 {below} to i64"));
                        let above = self.value(format!("zext i1 {above} to i64"));
                        self.value(format!("sub i64 {above}, {below}"))
                    }
                }
            }
            Builtin::Repr => {
                let slot = self.temp(&Ty::Str);
                let builder = self.value("call ptr @koo_str_new(i64 32)");
                self.emit(format!("store ptr {builder}, ptr {slot}"));
                let fmt = self.glue(Glue::Fmt(args[0].ty.clone()));
                let place = self.addr(&args[0]);
                self.emit(format!("call void {fmt}(ptr {slot}, ptr {place})"));
                self.take(&Ty::Str, &slot)
            }
            Builtin::Hash => {
                let place = self.addr(&args[0]);
                self.hash_at(&args[0].ty.clone(), &place, false)
            }
            Builtin::Min | Builtin::Max => {
                let a = self.gen_scalar(&args[0]);
                let b = self.gen_scalar(&args[1]);
                if args[0].ty == Ty::Float {
                    // `min` and `max` use the total order, in which NaN is last and
                    // -0.0 is before 0.0 (Section 4.2).
                    if builtin == Builtin::Max {
                        return self.value(format!("call double @llvm.maximum.f64(double {a}, double {b})"));
                    }
                    let least = self.value(format!("call double @llvm.minimum.f64(double {a}, double {b})"));
                    let a_nan = self.value(format!("fcmp uno double {a}, {a}"));
                    let b_nan = self.value(format!("fcmp uno double {b}, {b}"));
                    let or_a = self.value(format!("select i1 {b_nan}, double {a}, double {least}"));
                    return self.value(format!("select i1 {a_nan}, double {b}, double {or_a}"));
                }
                let intrinsic = if builtin == Builtin::Min { "smin" } else { "smax" };
                self.value(format!("call i64 @llvm.{intrinsic}.i64(i64 {a}, i64 {b})"))
            }
            Builtin::WrappingAdd
            | Builtin::WrappingSub
            | Builtin::WrappingMul
            | Builtin::BitAnd
            | Builtin::BitOr
            | Builtin::BitXor => {
                let a = self.gen_scalar(&args[0]);
                let b = self.gen_scalar(&args[1]);
                let instruction = match builtin {
                    Builtin::WrappingAdd => "add",
                    Builtin::WrappingSub => "sub",
                    Builtin::WrappingMul => "mul",
                    Builtin::BitAnd => "and",
                    Builtin::BitOr => "or",
                    _ => "xor",
                };
                self.value(format!("{instruction} i64 {a}, {b}"))
            }
            Builtin::Shl | Builtin::Shr => {
                let a = self.gen_scalar(&args[0]);
                let b = self.gen_scalar(&args[1]);
                let ok = self.value(format!("icmp ult i64 {b}, 64"));
                self.check(&ok, "shift amount out of range", span);
                let instruction = if builtin == Builtin::Shl { "shl" } else { "ashr" };
                self.value(format!("{instruction} i64 {a}, {b}"))
            }
        }
    }
}
