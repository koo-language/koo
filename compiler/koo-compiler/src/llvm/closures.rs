//! Code generation for function values (Sections 4.11 and 5.7).
//!
//! A function value is a pointer to `{ rc, code, free, captured... }`. `code` is the
//! function to call; it takes the value itself as a hidden first parameter, through which
//! a lambda reaches what it captured. `free` releases what a closure captured.
//!
//! * A lambda that captures nothing, and a named function, are one static value each.
//! * A block (a lambda passed to a borrowed function parameter) lives on the stack of the
//!   function that wrote it and holds the addresses of the variables it uses.
//! * A closure lives on the heap and holds copies.

use std::fmt::Write;

use super::{Gen, Glue, IMMORTAL, is_aggregate};
use crate::hir::*;

impl Gen<'_> {
    pub(super) fn closure_ty(&self, func: FnId) -> String {
        format!("%\"C.{}\"", self.p.functions[func].name)
    }

    /// Whether a captured variable of a lambda is held by address.
    fn by_ref(&self, func: FnId, local: LocalId) -> bool {
        matches!(
            self.p.functions[func].locals[local].kind,
            LocalKind::Capture { by_ref: true }
        )
    }

    /// The definition of the type of a lambda's function value.
    pub(super) fn closure_type_def(&self, func: FnId) -> String {
        let function = &self.p.functions[func];
        let mut fields = vec!["i64".to_string(), "ptr".to_string(), "ptr".to_string()];
        for &local in &function.captures {
            if self.by_ref(func, local) {
                fields.push("ptr".to_string());
            } else {
                fields.push(self.llty(&function.locals[local].ty));
            }
        }
        format!("{} = type {{ {} }}\n", self.closure_ty(func), fields.join(", "))
    }

    /// In the entry of a lambda's function: where each captured variable lives.
    pub(super) fn bind_captures(&mut self, func: FnId) {
        let closure_ty = self.closure_ty(func);
        let captures = self.p.functions[func].captures.clone();
        for (index, local) in captures.into_iter().enumerate() {
            let field = self.tmp();
            let _ = writeln!(
                self.f.entry,
                "  {field} = getelementptr inbounds {closure_ty}, ptr %env, i32 0, i32 {}",
                index + 3
            );
            let place = if self.by_ref(func, local) {
                let place = self.tmp();
                let _ = writeln!(self.f.entry, "  {place} = load ptr, ptr {field}, !invariant.load !13");
                place
            } else {
                field
            };
            self.f.locals[local] = Some(place);
        }
    }

    /// The value of a lambda written in the current function.
    pub(super) fn closure(&mut self, func: FnId, captures: &[LocalId], block: bool) -> String {
        let closure_ty = self.closure_ty(func);
        let code = self.fn_name(func);
        if captures.is_empty() {
            if !self.static_closures.contains(&func) {
                self.static_closures.push(func);
            }
            return format!("@\"closure.{}\"", self.p.functions[func].name);
        }
        let inner = self.p.functions[func].captures.clone();
        if block {
            // On the stack: the block does not outlive the call it is passed to.
            let object = self.tmp();
            let _ = writeln!(self.f.entry, "  {object} = alloca {closure_ty}");
            self.emit(format!("store i64 {IMMORTAL}, ptr {object}, !tbaa !2"));
            let code_ptr = self.field_ptr(&closure_ty, &object, 1);
            self.emit(format!("store ptr {code}, ptr {code_ptr}"));
            for (index, &outer) in captures.iter().enumerate() {
                let field = self.field_ptr(&closure_ty, &object, index + 3);
                let place = self.f.locals[outer].clone().expect("a captured variable is declared");
                self.emit(format!("store ptr {place}, ptr {field}"));
            }
            return object;
        }
        let size = format!("ptrtoint (ptr getelementptr ({closure_ty}, ptr null, i32 1) to i64)");
        let object = self.value(format!("call ptr @koo_alloc(i64 {size})"));
        self.emit(format!("store i64 1, ptr {object}, !tbaa !2"));
        let code_ptr = self.field_ptr(&closure_ty, &object, 1);
        self.emit(format!("store ptr {code}, ptr {code_ptr}"));
        let free = self.glue(Glue::FreeClosure(func));
        let free_ptr = self.field_ptr(&closure_ty, &object, 2);
        self.emit(format!("store ptr {free}, ptr {free_ptr}"));
        for (index, (&outer, &local)) in captures.iter().zip(&inner).enumerate() {
            let ty = self.p.functions[func].locals[local].ty.clone();
            let field = self.field_ptr(&closure_ty, &object, index + 3);
            let place = self.f.locals[outer].clone().expect("a captured variable is declared");
            // A copy of the variable as it is now.
            self.move_value(&ty, &field, &place);
            self.retain_at(&ty, &field);
        }
        object
    }

    /// Frees what a closure captured, and the closure.
    pub(super) fn free_closure_glue(&mut self, func: FnId) {
        let closure_ty = self.closure_ty(func);
        let captures = self.p.functions[func].captures.clone();
        for (index, local) in captures.into_iter().enumerate() {
            let ty = self.p.functions[func].locals[local].ty.clone();
            if !self.by_ref(func, local) && self.droppable(&ty) {
                let field = self.field_ptr(&closure_ty, "%p", index + 3);
                self.drop_at(&ty, &field);
            }
        }
        self.emit("call void @koo_free(ptr %p)");
    }

    /// The value of a named function.
    pub(super) fn fn_ref(&mut self, func: FnId) -> String {
        if !self.fn_refs.contains(&func) {
            self.fn_refs.push(func);
        }
        format!("@\"fnref.{}\"", self.p.functions[func].name)
    }

    /// The static values of the lambdas that capture nothing and of the named functions
    /// used as values, with the thunks that give a named function the calling convention
    /// of a function value.
    pub(super) fn static_function_values(&mut self) -> String {
        let mut out = String::new();
        for &func in &self.static_closures {
            let _ = writeln!(
                out,
                "@\"closure.{}\" = private unnamed_addr constant {{ i64, ptr, ptr }} {{ i64 {IMMORTAL}, ptr {}, ptr null }}",
                self.p.functions[func].name,
                self.fn_name(func)
            );
        }
        for &func in &self.fn_refs {
            let function = &self.p.functions[func];
            let name = &function.name;
            let _ = writeln!(
                out,
                "@\"fnref.{name}\" = private unnamed_addr constant {{ i64, ptr, ptr }} {{ i64 {IMMORTAL}, ptr @\"thunk.{name}\", ptr null }}"
            );
            let mut params: Vec<String> = Vec::new();
            let mut args: Vec<String> = Vec::new();
            let sret = is_aggregate(&function.ret);
            if sret {
                params.push("ptr %ret".to_string());
                args.push("ptr %ret".to_string());
            }
            params.push("ptr %env".to_string());
            for (index, local) in function.locals[..function.params].iter().enumerate() {
                let (llty, _) = self.param_abi(local);
                params.push(format!("{llty} %p{index}"));
                args.push(format!("{llty} %p{index}"));
            }
            let target = self.fn_name(func);
            let (params, args) = (params.join(", "), args.join(", "));
            if sret || matches!(function.ret, Ty::Unit | Ty::Never | Ty::Unknown) {
                let _ = writeln!(
                    out,
                    "define internal void @\"thunk.{name}\"({params}) {{\n  call void {target}({args})\n  ret void\n}}"
                );
            } else {
                let llty = self.llty(&function.ret);
                let _ = writeln!(
                    out,
                    "define internal {llty} @\"thunk.{name}\"({params}) {{\n  %r = call {llty} {target}({args})\n  ret {llty} %r\n}}"
                );
            }
        }
        out
    }
}
