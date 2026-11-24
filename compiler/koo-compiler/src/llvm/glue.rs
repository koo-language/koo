//! Functions the compiler writes for each type that needs them: dropping, retaining and
//! printing a value, and the list operations whose code depends on the element type.
//!
//! Each takes a pointer to the value. They are ordinary internal functions, and LLVM
//! inlines the small ones.

use super::{FnState, Gen, Glue, is_aggregate};
use crate::hir::*;

impl Gen<'_> {
    pub(super) fn emit_glue(&mut self, glue: &Glue) -> String {
        self.f = FnState::default();
        let name = self.glue(glue.clone());
        let params = match glue {
            Glue::Drop(ty) => {
                self.drop_glue(ty);
                "ptr %p".to_string()
            }
            Glue::Retain(ty) => {
                self.retain_glue(ty);
                "ptr %p".to_string()
            }
            Glue::Fmt(ty) => {
                self.fmt_glue(ty);
                "ptr %b, ptr %p".to_string()
            }
            Glue::Eq(ty) => {
                self.eq_at(ty, "%a", "%b", "differ", true);
                self.terminate("ret i1 true");
                self.label("differ");
                self.terminate("ret i1 false");
                "ptr %a, ptr %b".to_string()
            }
            Glue::Hash(ty) => {
                let hash = self.hash_at(ty, "%p", true);
                self.terminate(format!("ret i64 {hash}"));
                "ptr %p".to_string()
            }
            Glue::FreeError(id) => {
                let box_ty = self.error_box_ty(*id);
                let fields = self.p.errors[*id].fields.clone();
                for (index, field) in fields.iter().enumerate() {
                    if self.droppable(&field.ty) {
                        let place = self.field_ptr(&box_ty, "%p", index + 2);
                        self.drop_at(&field.ty, &place);
                    }
                }
                self.emit("call void @koo_free(ptr %p)");
                "ptr %p".to_string()
            }
            Glue::FreeList(ty) => {
                let Ty::List(element) = ty else { unreachable!() };
                if self.droppable(element) {
                    self.each_element("%p", element, |generator, place| generator.drop_at(element, place));
                }
                self.emit("call void @koo_free(ptr %p)");
                "ptr %p".to_string()
            }
            Glue::FreeClosure(id) => {
                self.free_closure_glue(*id);
                "ptr %p".to_string()
            }
            Glue::Unique(ty) => {
                self.unique_glue(ty);
                "ptr %slot".to_string()
            }
            Glue::Push(ty) | Glue::PushUnique(ty) => {
                let Ty::List(element) = ty else { unreachable!() };
                let hoisted = matches!(glue, Glue::PushUnique(_));
                self.push_glue(ty, element, hoisted);
                let len = if hoisted { "ptr %len, " } else { "" };
                if is_aggregate(element) {
                    format!("ptr %slot, {len}ptr %v")
                } else {
                    format!("ptr %slot, {len}{} %v", self.llty(element))
                }
            }
            Glue::Pop(ty) => {
                let Ty::List(element) = ty else { unreachable!() };
                self.pop_glue(ty, element);
                "ptr %slot, ptr %dest".to_string()
            }
        };
        self.terminate("ret void");
        let ret = match glue {
            Glue::Eq(_) => "i1",
            Glue::Hash(_) => "i64",
            _ => "void",
        };
        let mut text = format!("define internal {ret} {name}({params}) {{\nentry:\n");
        text.push_str(&self.f.entry);
        text.push_str("  br label %start\nstart:\n");
        text.push_str(&self.f.body);
        text.push_str("}\n\n");
        text
    }

    /// Runs `body` with a pointer to each element of the list `list`.
    fn each_element(&mut self, list: &str, element: &Ty, body: impl FnOnce(&mut Self, &str)) {
        let len_ptr = self.value(format!("getelementptr inbounds i64, ptr {list}, i64 1"));
        let len = self.value(format!("load i64, ptr {len_ptr}, !tbaa !2"));
        let index = self.alloca(&Ty::Int, false);
        self.emit(format!("store i64 0, ptr {index}"));
        let head = self.new_label("each");
        let step = self.new_label("each.body");
        let end = self.new_label("each.end");
        self.label(&head);
        let i = self.value(format!("load i64, ptr {index}"));
        let more = self.value(format!("icmp slt i64 {i}, {len}"));
        self.cond_br(&more, &step, &end);
        self.label(&step);
        let place = self.element_ptr(list, &i, element, None);
        body(self, &place);
        let next = self.value(format!("add nsw i64 {i}, 1"));
        self.emit(format!("store i64 {next}, ptr {index}"));
        self.br(&head);
        self.label(&end);
    }

    /// Branches on the tag of the enum at `%p`, running `body` for each variant that has
    /// a field `wanted` selects.
    fn each_variant(&mut self, id: EnumId, mut body: impl FnMut(&mut Self, &Ty, &str)) {
        let enum_ty = self.llty(&Ty::Enum(id));
        let variants = self.p.enums[id].variants.clone();
        let tag = self.value("load i64, ptr %p");
        let end = self.new_label("variants.end");
        let labels: Vec<String> = variants.iter().map(|_| self.new_label("variant")).collect();
        let cases: Vec<String> = labels
            .iter()
            .enumerate()
            .map(|(index, label)| format!("i64 {index}, label %{label}"))
            .collect();
        self.terminate(format!("switch i64 {tag}, label %{end} [ {} ]", cases.join(" ")));
        for (index, variant) in variants.iter().enumerate() {
            self.label(&labels[index]);
            let variant_ty = self.variant_ty(id, index);
            let payload = self.field_ptr(&enum_ty, "%p", 1);
            for (field_index, field) in variant.fields.iter().enumerate() {
                if self.droppable(&field.ty) {
                    let place = self.field_ptr(&variant_ty, &payload, field_index);
                    body(self, &field.ty, &place);
                }
            }
            self.br(&end);
        }
        self.label(&end);
    }

    /// Releases the counted value whose handle is at `%p`, calling `free` if that was the
    /// last reference. An empty slot holds null.
    fn release_handle(&mut self, free: impl FnOnce(&mut Self, &str)) {
        self.release_handle_with("@koo.release", free)
    }

    fn release_handle_with(&mut self, release: &str, free: impl FnOnce(&mut Self, &str)) {
        let value = self.value("load ptr, ptr %p");
        let empty = self.value(format!("icmp eq ptr {value}, null"));
        let live = self.new_label("live");
        let last = self.new_label("last");
        let end = self.new_label("end");
        self.cond_br(&empty, &end, &live);
        self.label(&live);
        let was_last = self.value(format!("call i1 {release}(ptr {value})"));
        self.cond_br(&was_last, &last, &end);
        self.label(&last);
        free(self, &value);
        self.br(&end);
        self.label(&end);
    }

    fn drop_glue(&mut self, ty: &Ty) {
        match ty {
            Ty::Fn(_) => {
                // A function value that owns what it captured says how to free it. A static
                // one is a constant, and its count is left alone.
                self.release_handle_with("@koo.release.fn", |generator, value| {
                    let free_ptr = generator.value(format!("getelementptr inbounds ptr, ptr {value}, i64 2"));
                    let free = generator.value(format!("load ptr, ptr {free_ptr}"));
                    generator.emit(format!("call void {free}(ptr {value})"));
                });
            }
            Ty::Tuple(types) => {
                let llty = self.llty(ty);
                for (index, element) in types.iter().enumerate() {
                    if self.droppable(element) {
                        let place = self.field_ptr(&llty, "%p", index);
                        self.drop_at(element, &place);
                    }
                }
            }
            Ty::Str => {
                self.release_handle(|generator, value| generator.emit(format!("call void @koo_free(ptr {value})")))
            }
            Ty::List(_) => {
                let free = self.glue(Glue::FreeList(ty.clone()));
                self.release_handle(|generator, value| generator.emit(format!("call void {free}(ptr {value})")));
            }
            Ty::Error(id) => {
                let free = self.glue(Glue::FreeError(*id));
                self.release_handle(|generator, value| generator.emit(format!("call void {free}(ptr {value})")));
            }
            Ty::Failure => {
                // The box says which error type it holds.
                let frees: Vec<String> = (0..self.p.errors.len())
                    .map(|id| self.glue(Glue::FreeError(id)))
                    .collect();
                self.release_handle(|generator, value| {
                    let id_ptr = generator.value(format!("getelementptr inbounds i64, ptr {value}, i64 1"));
                    let id = generator.value(format!("load i64, ptr {id_ptr}, !tbaa !2"));
                    let end = generator.new_label("freed");
                    let labels: Vec<String> = frees.iter().map(|_| generator.new_label("free")).collect();
                    let cases: Vec<String> = labels
                        .iter()
                        .enumerate()
                        .map(|(index, label)| format!("i64 {index}, label %{label}"))
                        .collect();
                    generator.terminate(format!("switch i64 {id}, label %{end} [ {} ]", cases.join(" ")));
                    for (label, free) in labels.iter().zip(&frees) {
                        generator.label(label);
                        generator.emit(format!("call void {free}(ptr {value})"));
                        generator.br(&end);
                    }
                    generator.label(&end);
                });
            }
            Ty::Struct(id) => {
                let struct_ty = self.llty(ty);
                let fields = self.p.structs[*id].fields.clone();
                for (index, field) in fields.iter().enumerate() {
                    if self.droppable(&field.ty) {
                        let place = self.field_ptr(&struct_ty, "%p", index);
                        self.drop_at(&field.ty, &place);
                    }
                }
            }
            Ty::Enum(id) => self.each_variant(*id, |generator, field_ty, place| generator.drop_at(field_ty, place)),
            Ty::Optional(inner) => {
                let llty = self.llty(ty);
                let present = self.value("load i1, ptr %p");
                let some = self.new_label("some");
                let end = self.new_label("end");
                self.cond_br(&present, &some, &end);
                self.label(&some);
                let payload = self.field_ptr(&llty, "%p", 1);
                self.drop_at(inner, &payload);
                self.br(&end);
                self.label(&end);
            }
            Ty::Result(ok, _) => {
                let llty = self.llty(ty);
                let error = self.value("load ptr, ptr %p");
                let failed = self.value(format!("icmp ne ptr {error}, null"));
                let err = self.new_label("err");
                let good = self.new_label("ok");
                let end = self.new_label("end");
                self.cond_br(&failed, &err, &good);
                self.label(&err);
                self.drop_at(&Ty::Failure, "%p");
                self.br(&end);
                self.label(&good);
                if self.droppable(ok) {
                    let payload = self.field_ptr(&llty, "%p", 1);
                    self.drop_at(ok, &payload);
                }
                self.br(&end);
                self.label(&end);
            }
            _ => {}
        }
    }

    fn retain_glue(&mut self, ty: &Ty) {
        match ty {
            Ty::Fn(_) => {
                let value = self.value("load ptr, ptr %p");
                self.emit(format!("call void @koo.retain.fn(ptr {value})"));
            }
            Ty::Str | Ty::List(_) | Ty::Error(_) | Ty::Failure => {
                let value = self.value("load ptr, ptr %p");
                self.emit(format!("call void @koo.retain(ptr {value})"));
            }
            Ty::Tuple(types) => {
                let llty = self.llty(ty);
                for (index, element) in types.iter().enumerate() {
                    if self.droppable(element) {
                        let place = self.field_ptr(&llty, "%p", index);
                        self.retain_at(element, &place);
                    }
                }
            }
            Ty::Struct(id) => {
                let struct_ty = self.llty(ty);
                let fields = self.p.structs[*id].fields.clone();
                for (index, field) in fields.iter().enumerate() {
                    if self.droppable(&field.ty) {
                        let place = self.field_ptr(&struct_ty, "%p", index);
                        self.retain_at(&field.ty, &place);
                    }
                }
            }
            Ty::Enum(id) => self.each_variant(*id, |generator, field_ty, place| generator.retain_at(field_ty, place)),
            Ty::Optional(inner) => {
                let llty = self.llty(ty);
                let present = self.value("load i1, ptr %p");
                let some = self.new_label("some");
                let end = self.new_label("end");
                self.cond_br(&present, &some, &end);
                self.label(&some);
                let payload = self.field_ptr(&llty, "%p", 1);
                self.retain_at(inner, &payload);
                self.br(&end);
                self.label(&end);
            }
            Ty::Result(ok, _) => {
                let llty = self.llty(ty);
                let error = self.value("load ptr, ptr %p");
                let failed = self.value(format!("icmp ne ptr {error}, null"));
                let err = self.new_label("err");
                let good = self.new_label("ok");
                let end = self.new_label("end");
                self.cond_br(&failed, &err, &good);
                self.label(&err);
                self.emit(format!("call void @koo.retain(ptr {error})"));
                self.br(&end);
                self.label(&good);
                if self.droppable(ok) {
                    let payload = self.field_ptr(&llty, "%p", 1);
                    self.retain_at(ok, &payload);
                }
                self.br(&end);
                self.label(&end);
            }
            _ => {}
        }
    }

    /// Appends `name { field: value, ... }` for the fields at `base`, a struct of `struct_ty`.
    fn fmt_fields(&mut self, name: &str, struct_ty: &str, base: &str, fields: &[FieldDef], offset: usize) {
        if fields.is_empty() {
            self.append_text("%b", name);
            return;
        }
        self.append_text("%b", &format!("{name} {{ "));
        for (index, field) in fields.iter().enumerate() {
            let separator = if index == 0 { "" } else { ", " };
            self.append_text("%b", &format!("{separator}{}: ", field.name));
            let place = self.field_ptr(struct_ty, base, index + offset);
            self.fmt_at(&field.ty, &place);
        }
        self.append_text("%b", " }");
    }

    /// Appends the text form of the value at `place` to the string at `%b`.
    fn fmt_at(&mut self, ty: &Ty, place: &str) {
        let fmt = self.glue(Glue::Fmt(ty.clone()));
        self.emit(format!("call void {fmt}(ptr %b, ptr {place})"));
    }

    /// The text form of a value (Section 4.6). Inside another value a string is quoted.
    fn fmt_glue(&mut self, ty: &Ty) {
        if let Some(&function) = self.p.to_string.get(ty) {
            // The type has a `to_string()` of its own.
            let name = self.fn_name(function);
            let receiver = if is_aggregate(ty) {
                "%p".to_string()
            } else {
                self.value("load ptr, ptr %p")
            };
            let text = self.value(format!("call ptr {name}(ptr {receiver})"));
            self.emit(format!("call void @koo_str_append(ptr %b, ptr {text})"));
            let slot = self.alloca(&Ty::Str, false);
            self.emit(format!("store ptr {text}, ptr {slot}"));
            self.drop_at(&Ty::Str, &slot);
            return;
        }
        match ty {
            Ty::Int => {
                let value = self.value("load i64, ptr %p");
                self.emit(format!("call void @koo_str_append_int(ptr %b, i64 {value})"));
            }
            Ty::Byte => {
                let value = self.value("load i8, ptr %p");
                let wide = self.value(format!("zext i8 {value} to i64"));
                self.emit(format!("call void @koo_str_append_int(ptr %b, i64 {wide})"));
            }
            Ty::Float => {
                let value = self.value("load double, ptr %p");
                self.emit(format!(
                    "call void @koo_str_append_float(ptr %b, double {value}, i32 -1)"
                ));
            }
            Ty::Bool => {
                let value = self.value("load i1, ptr %p");
                self.emit(format!("call void @koo_str_append_bool(ptr %b, i1 zeroext {value})"));
            }
            Ty::Str => {
                let value = self.value("load ptr, ptr %p");
                self.append_text("%b", "\"");
                self.emit(format!("call void @koo_str_append(ptr %b, ptr {value})"));
                self.append_text("%b", "\"");
            }
            Ty::Unit | Ty::Never | Ty::Unknown => self.append_text("%b", "()"),
            Ty::Fn(_) => self.append_text("%b", "fn"),
            Ty::Tuple(types) => {
                let llty = self.llty(ty);
                self.append_text("%b", "(");
                for (index, element) in types.iter().enumerate() {
                    if index > 0 {
                        self.append_text("%b", ", ");
                    }
                    let place = self.field_ptr(&llty, "%p", index);
                    self.fmt_at(element, &place);
                }
                self.append_text("%b", ")");
            }
            Ty::List(element) => {
                let list = self.value("load ptr, ptr %p");
                self.append_text("%b", "[");
                let first = self.alloca(&Ty::Bool, false);
                self.emit(format!("store i1 true, ptr {first}"));
                self.each_element(&list, element, |generator, place| {
                    let is_first = generator.value(format!("load i1, ptr {first}"));
                    let comma = generator.new_label("comma");
                    let item = generator.new_label("item");
                    generator.cond_br(&is_first, &item, &comma);
                    generator.label(&comma);
                    generator.append_text("%b", ", ");
                    generator.label(&item);
                    generator.emit(format!("store i1 false, ptr {first}"));
                    generator.fmt_at(element, place);
                });
                self.append_text("%b", "]");
            }
            Ty::Optional(inner) => {
                let llty = self.llty(ty);
                let present = self.value("load i1, ptr %p");
                let some = self.new_label("some");
                let none = self.new_label("none");
                let end = self.new_label("end");
                self.cond_br(&present, &some, &none);
                self.label(&some);
                let payload = self.field_ptr(&llty, "%p", 1);
                self.fmt_at(inner, &payload);
                self.br(&end);
                self.label(&none);
                self.append_text("%b", "none");
                self.br(&end);
                self.label(&end);
            }
            Ty::Result(ok, _) => {
                let llty = self.llty(ty);
                let error = self.value("load ptr, ptr %p");
                let failed = self.value(format!("icmp ne ptr {error}, null"));
                let err = self.new_label("err");
                let good = self.new_label("ok");
                let end = self.new_label("end");
                self.cond_br(&failed, &err, &good);
                self.label(&err);
                self.fmt_at(&Ty::Failure, "%p");
                self.br(&end);
                self.label(&good);
                let payload = self.field_ptr(&llty, "%p", 1);
                self.fmt_at(ok, &payload);
                self.br(&end);
                self.label(&end);
            }
            Ty::Struct(id) => {
                let def = self.p.structs[*id].clone();
                let struct_ty = self.llty(ty);
                self.fmt_fields(&def.name, &struct_ty, "%p", &def.fields, 0);
            }
            Ty::Enum(id) => {
                let enum_ty = self.llty(ty);
                let variants = self.p.enums[*id].variants.clone();
                let tag = self.value("load i64, ptr %p");
                let end = self.new_label("end");
                let labels: Vec<String> = variants.iter().map(|_| self.new_label("variant")).collect();
                let cases: Vec<String> = labels
                    .iter()
                    .enumerate()
                    .map(|(index, label)| format!("i64 {index}, label %{label}"))
                    .collect();
                self.terminate(format!("switch i64 {tag}, label %{end} [ {} ]", cases.join(" ")));
                for (index, variant) in variants.iter().enumerate() {
                    self.label(&labels[index]);
                    let variant_ty = self.variant_ty(*id, index);
                    let payload = self.field_ptr(&enum_ty, "%p", 1);
                    self.fmt_fields(&variant.name, &variant_ty, &payload, &variant.fields, 0);
                    self.br(&end);
                }
                self.label(&end);
            }
            Ty::Error(id) => {
                let def = self.p.errors[*id].clone();
                let box_ty = self.error_box_ty(*id);
                let value = self.value("load ptr, ptr %p");
                self.fmt_fields(&def.name, &box_ty, &value, &def.fields, 2);
            }
            Ty::Failure => {
                let value = self.value("load ptr, ptr %p");
                let id_ptr = self.value(format!("getelementptr inbounds i64, ptr {value}, i64 1"));
                let id = self.value(format!("load i64, ptr {id_ptr}, !tbaa !2"));
                let end = self.new_label("end");
                let count = self.p.errors.len();
                let labels: Vec<String> = (0..count).map(|_| self.new_label("error")).collect();
                let cases: Vec<String> = labels
                    .iter()
                    .enumerate()
                    .map(|(index, label)| format!("i64 {index}, label %{label}"))
                    .collect();
                self.terminate(format!("switch i64 {id}, label %{end} [ {} ]", cases.join(" ")));
                for (index, label) in labels.iter().enumerate() {
                    self.label(label);
                    self.fmt_at(&Ty::Error(index), "%p");
                    self.br(&end);
                }
                self.label(&end);
            }
        }
    }

    /// Makes the list in `%slot` uniquely owned, copying it if it is shared or static.
    fn unique_glue(&mut self, ty: &Ty) {
        let Ty::List(element) = ty else { unreachable!() };
        let list = self.value("load ptr, ptr %slot");
        let rc = self.value(format!("load i64, ptr {list}, !tbaa !2"));
        let shared = self.value(format!("icmp ne i64 {rc}, 1"));
        let copy = self.new_label("copy");
        let end = self.new_label("end");
        self.cond_br(&shared, &copy, &end);
        self.label(&copy);
        let len_ptr = self.value(format!("getelementptr inbounds i64, ptr {list}, i64 1"));
        let len = self.value(format!("load i64, ptr {len_ptr}, !tbaa !2"));
        let size = self.sizeof(element);
        let fresh = self.value(format!("call ptr @koo_list_new(i64 {len}, i64 {size})"));
        let fresh_len = self.value(format!("getelementptr inbounds i64, ptr {fresh}, i64 1"));
        self.emit(format!("store i64 {len}, ptr {fresh_len}, !tbaa !2"));
        let from = self.value(format!("getelementptr inbounds i8, ptr {list}, i64 24"));
        let to = self.value(format!("getelementptr inbounds i8, ptr {fresh}, i64 24"));
        let bytes = self.value(format!("mul i64 {len}, {size}"));
        self.emit(format!(
            "call void @llvm.memcpy.p0.p0.i64(ptr {to}, ptr {from}, i64 {bytes}, i1 false)"
        ));
        if self.droppable(element) {
            self.each_element(&fresh, element, |generator, place| generator.retain_at(element, place));
        }
        // The slot gives up its reference to the original, which others still hold.
        let less = self.value(format!("sub i64 {rc}, 1"));
        self.emit(format!("store i64 {less}, ptr {list}, !tbaa !2"));
        self.emit(format!("store ptr {fresh}, ptr %slot"));
        self.br(&end);
        self.label(&end);
    }

    /// Appends the value `%v` to the list in `%slot`, taking ownership of it. A `hoisted`
    /// push is to a list that is already uniquely owned and whose length is also kept in
    /// `%len`: it reads the length there and stores it in both places.
    fn push_glue(&mut self, ty: &Ty, element: &Ty, hoisted: bool) {
        if !hoisted {
            let unique = self.glue(Glue::Unique(ty.clone()));
            self.emit(format!("call void {unique}(ptr %slot)"));
        }
        let list = self.value("load ptr, ptr %slot");
        let len = if hoisted {
            self.value("load i64, ptr %len")
        } else {
            let len_ptr = self.value(format!("getelementptr inbounds i64, ptr {list}, i64 1"));
            self.value(format!("load i64, ptr {len_ptr}, !tbaa !2"))
        };
        let cap_ptr = self.value(format!("getelementptr inbounds i64, ptr {list}, i64 2"));
        let cap = self.value(format!("load i64, ptr {cap_ptr}, !tbaa !2"));
        let full = self.value(format!("icmp eq i64 {len}, {cap}"));
        let grow = self.new_label("grow");
        let room = self.new_label("room");
        self.cond_br(&full, &grow, &room);
        self.label(&grow);
        let size = self.sizeof(element);
        let needed = self.value(format!("add i64 {len}, 1"));
        let grown = self.value(format!("call ptr @koo_list_grow(ptr {list}, i64 {size}, i64 {needed})"));
        self.emit(format!("store ptr {grown}, ptr %slot"));
        self.br(&room);
        self.label(&room);
        let list = self.value("load ptr, ptr %slot");
        let place = self.element_ptr(&list, &len, element, None);
        if is_aggregate(element) {
            self.memcpy(element, &place, "%v");
        } else {
            self.store(element, "%v", &place);
        }
        let len_ptr = self.value(format!("getelementptr inbounds i64, ptr {list}, i64 1"));
        let more = self.value(format!("add i64 {len}, 1"));
        self.emit(format!("store i64 {more}, ptr {len_ptr}, !tbaa !2"));
        if hoisted {
            self.emit(format!("store i64 {more}, ptr %len"));
        }
    }

    /// Moves the last element of the list in `%slot` into the optional at `%dest`.
    fn pop_glue(&mut self, ty: &Ty, element: &Ty) {
        let optional = self.llty(&Ty::optional(element.clone()));
        let list = self.value("load ptr, ptr %slot");
        let len_ptr = self.value(format!("getelementptr inbounds i64, ptr {list}, i64 1"));
        let len = self.value(format!("load i64, ptr {len_ptr}, !tbaa !2"));
        let empty = self.value(format!("icmp eq i64 {len}, 0"));
        let none = self.new_label("none");
        let some = self.new_label("some");
        let end = self.new_label("end");
        self.cond_br(&empty, &none, &some);
        self.label(&none);
        self.emit("store i1 false, ptr %dest");
        self.br(&end);
        self.label(&some);
        let unique = self.glue(Glue::Unique(ty.clone()));
        self.emit(format!("call void {unique}(ptr %slot)"));
        let list = self.value("load ptr, ptr %slot");
        let last = self.value(format!("sub i64 {len}, 1"));
        let from = self.element_ptr(&list, &last, element, None);
        self.emit("store i1 true, ptr %dest");
        let to = self.field_ptr(&optional, "%dest", 1);
        self.move_value(element, &to, &from);
        let len_ptr = self.value(format!("getelementptr inbounds i64, ptr {list}, i64 1"));
        self.emit(format!("store i64 {last}, ptr {len_ptr}, !tbaa !2"));
        self.br(&end);
        self.label(&end);
    }
    /// Branches to `differ` unless `holds`.
    fn same_or(&mut self, holds: &str, differ: &str) {
        let next = self.new_label("same");
        self.cond_br(holds, &next, differ);
        self.label(&next);
    }

    /// Compares the values at two places, branching to `differ` if they are not equal
    /// (Section 3.4: `==` is structural). `inline` says to compare a composite value here
    /// and not through its own glue function, which is how that function's body is made.
    pub(super) fn eq_at(&mut self, ty: &Ty, a: &str, b: &str, differ: &str, inline: bool) {
        let llty = self.llty(ty);
        match ty {
            Ty::Unit | Ty::Never | Ty::Unknown => {}
            Ty::Int | Ty::Byte | Ty::Bool => {
                let x = self.load(ty, a);
                let y = self.load(ty, b);
                let same = self.value(format!("icmp eq {llty} {x}, {y}"));
                self.same_or(&same, differ);
            }
            Ty::Float => {
                let x = self.load(ty, a);
                let y = self.load(ty, b);
                let same = self.value(format!("fcmp oeq double {x}, {y}"));
                self.same_or(&same, differ);
            }
            Ty::Str => {
                let x = self.load(ty, a);
                let y = self.load(ty, b);
                let same = self.value(format!("call zeroext i1 @koo_str_eq(ptr {x}, ptr {y})"));
                self.same_or(&same, differ);
            }
            Ty::Fn(_) => {
                let x = self.load(ty, a);
                let y = self.load(ty, b);
                let same = self.value(format!("icmp eq ptr {x}, {y}"));
                self.same_or(&same, differ);
            }
            _ if !inline => {
                let eq = self.glue(Glue::Eq(ty.clone()));
                let same = self.value(format!("call i1 {eq}(ptr {a}, ptr {b})"));
                self.same_or(&same, differ);
            }
            Ty::List(element) => {
                let x = self.load(ty, a);
                let y = self.load(ty, b);
                let x_len_ptr = self.value(format!("getelementptr inbounds i64, ptr {x}, i64 1"));
                let x_len = self.value(format!("load i64, ptr {x_len_ptr}, !tbaa !2"));
                let y_len_ptr = self.value(format!("getelementptr inbounds i64, ptr {y}, i64 1"));
                let y_len = self.value(format!("load i64, ptr {y_len_ptr}, !tbaa !2"));
                let same_len = self.value(format!("icmp eq i64 {x_len}, {y_len}"));
                self.same_or(&same_len, differ);
                let differ = differ.to_string();
                let element = (**element).clone();
                self.each_element(&x, &element, |generator, place| {
                    // The matching element of the other list is at the same offset.
                    let offset = generator.value(format!("ptrtoint ptr {place} to i64"));
                    let base = generator.value(format!("ptrtoint ptr {x} to i64"));
                    let delta = generator.value(format!("sub i64 {offset}, {base}"));
                    let other = generator.value(format!("getelementptr inbounds i8, ptr {y}, i64 {delta}"));
                    generator.eq_at(&element, place, &other, &differ, false);
                });
            }
            Ty::Optional(inner) => {
                let x = self.value(format!("load i1, ptr {a}"));
                let y = self.value(format!("load i1, ptr {b}"));
                let same = self.value(format!("icmp eq i1 {x}, {y}"));
                self.same_or(&same, differ);
                let both = self.new_label("both");
                let end = self.new_label("end");
                self.cond_br(&x, &both, &end);
                self.label(&both);
                let pa = self.field_ptr(&llty, a, 1);
                let pb = self.field_ptr(&llty, b, 1);
                self.eq_at(inner, &pa, &pb, differ, false);
                self.label(&end);
            }
            Ty::Result(ok, _) => {
                let x = self.value(format!("load ptr, ptr {a}"));
                let y = self.value(format!("load ptr, ptr {b}"));
                let x_ok = self.value(format!("icmp eq ptr {x}, null"));
                let y_ok = self.value(format!("icmp eq ptr {y}, null"));
                let same = self.value(format!("icmp eq i1 {x_ok}, {y_ok}"));
                self.same_or(&same, differ);
                let values = self.new_label("values");
                let errors = self.new_label("errors");
                let end = self.new_label("end");
                self.cond_br(&x_ok, &values, &errors);
                self.label(&values);
                let pa = self.field_ptr(&llty, a, 1);
                let pb = self.field_ptr(&llty, b, 1);
                self.eq_at(ok, &pa, &pb, differ, false);
                self.br(&end);
                self.label(&errors);
                self.eq_at(&Ty::Failure, a, b, differ, false);
                self.label(&end);
            }
            Ty::Tuple(types) => {
                for (index, element) in types.iter().enumerate() {
                    let pa = self.field_ptr(&llty, a, index);
                    let pb = self.field_ptr(&llty, b, index);
                    self.eq_at(element, &pa, &pb, differ, false);
                }
            }
            Ty::Struct(id) => {
                let fields = self.p.structs[*id].fields.clone();
                for (index, field) in fields.iter().enumerate() {
                    let pa = self.field_ptr(&llty, a, index);
                    let pb = self.field_ptr(&llty, b, index);
                    self.eq_at(&field.ty, &pa, &pb, differ, false);
                }
            }
            Ty::Enum(id) => {
                let variants = self.p.enums[*id].variants.clone();
                let x = self.value(format!("load i64, ptr {a}"));
                let y = self.value(format!("load i64, ptr {b}"));
                let same = self.value(format!("icmp eq i64 {x}, {y}"));
                self.same_or(&same, differ);
                let end = self.new_label("end");
                let labels: Vec<String> = variants.iter().map(|_| self.new_label("variant")).collect();
                let cases: Vec<String> = labels
                    .iter()
                    .enumerate()
                    .map(|(index, label)| format!("i64 {index}, label %{label}"))
                    .collect();
                self.terminate(format!("switch i64 {x}, label %{end} [ {} ]", cases.join(" ")));
                for (index, variant) in variants.iter().enumerate() {
                    self.label(&labels[index]);
                    let variant_ty = self.variant_ty(*id, index);
                    let pa = self.field_ptr(&llty, a, 1);
                    let pb = self.field_ptr(&llty, b, 1);
                    for (field_index, field) in variant.fields.iter().enumerate() {
                        let fa = self.field_ptr(&variant_ty, &pa, field_index);
                        let fb = self.field_ptr(&variant_ty, &pb, field_index);
                        self.eq_at(&field.ty, &fa, &fb, differ, false);
                    }
                    self.br(&end);
                }
                self.label(&end);
            }
            Ty::Error(id) => {
                let fields = self.p.errors[*id].fields.clone();
                let box_ty = self.error_box_ty(*id);
                let x = self.value(format!("load ptr, ptr {a}"));
                let y = self.value(format!("load ptr, ptr {b}"));
                for (index, field) in fields.iter().enumerate() {
                    let fa = self.field_ptr(&box_ty, &x, index + 2);
                    let fb = self.field_ptr(&box_ty, &y, index + 2);
                    self.eq_at(&field.ty, &fa, &fb, differ, false);
                }
            }
            Ty::Failure => {
                let x = self.value(format!("load ptr, ptr {a}"));
                let y = self.value(format!("load ptr, ptr {b}"));
                let x_id_ptr = self.value(format!("getelementptr inbounds i64, ptr {x}, i64 1"));
                let x_id = self.value(format!("load i64, ptr {x_id_ptr}, !tbaa !2"));
                let y_id_ptr = self.value(format!("getelementptr inbounds i64, ptr {y}, i64 1"));
                let y_id = self.value(format!("load i64, ptr {y_id_ptr}, !tbaa !2"));
                let same = self.value(format!("icmp eq i64 {x_id}, {y_id}"));
                self.same_or(&same, differ);
                let end = self.new_label("end");
                let count = self.p.errors.len();
                let labels: Vec<String> = (0..count).map(|_| self.new_label("error")).collect();
                let cases: Vec<String> = labels
                    .iter()
                    .enumerate()
                    .map(|(index, label)| format!("i64 {index}, label %{label}"))
                    .collect();
                self.terminate(format!("switch i64 {x_id}, label %{end} [ {} ]", cases.join(" ")));
                for (index, label) in labels.iter().enumerate() {
                    self.label(label);
                    self.eq_at(&Ty::Error(index), a, b, differ, false);
                    self.br(&end);
                }
                self.label(&end);
            }
        }
    }

    /// Folds one more value into a hash.
    fn mix(&mut self, hash: &str, value: &str) -> String {
        let mixed = self.value(format!("xor i64 {hash}, {value}"));
        self.value(format!("mul i64 {mixed}, 1099511628211"))
    }

    /// The hash of the value at a place, which agrees with `==` (Section 4.2).
    pub(super) fn hash_at(&mut self, ty: &Ty, place: &str, inline: bool) -> String {
        let llty = self.llty(ty);
        let seed = "-3750763034362895579";
        match ty {
            Ty::Int => {
                let value = self.load(ty, place);
                let mixed = self.value(format!("mul i64 {value}, -7046029254386353131"));
                let shifted = self.value(format!("lshr i64 {mixed}, 29"));
                self.value(format!("xor i64 {mixed}, {shifted}"))
            }
            Ty::Byte | Ty::Bool => {
                let value = self.load(ty, place);
                let wide = self.value(format!("zext {llty} {value} to i64"));
                self.mix(seed, &wide)
            }
            Ty::Str => {
                let value = self.load(ty, place);
                self.value(format!("call i64 @koo_str_hash(ptr {value})"))
            }
            _ if !inline => {
                let hash = self.glue(Glue::Hash(ty.clone()));
                self.value(format!("call i64 {hash}(ptr {place})"))
            }
            Ty::List(element) => {
                let list = self.load(ty, place);
                let acc = self.alloca(&Ty::Int, false);
                self.emit(format!("store i64 {seed}, ptr {acc}"));
                let element = (**element).clone();
                self.each_element(&list, &element, |generator, item| {
                    let hash = generator.hash_at(&element, item, false);
                    let so_far = generator.value(format!("load i64, ptr {acc}"));
                    let mixed = generator.mix(&so_far, &hash);
                    generator.emit(format!("store i64 {mixed}, ptr {acc}"));
                });
                self.value(format!("load i64, ptr {acc}"))
            }
            Ty::Optional(inner) => {
                let acc = self.alloca(&Ty::Int, false);
                self.emit(format!("store i64 {seed}, ptr {acc}"));
                let present = self.value(format!("load i1, ptr {place}"));
                let some = self.new_label("some");
                let end = self.new_label("end");
                self.cond_br(&present, &some, &end);
                self.label(&some);
                let payload = self.field_ptr(&llty, place, 1);
                let hash = self.hash_at(inner, &payload, false);
                let mixed = self.mix("1", &hash);
                self.emit(format!("store i64 {mixed}, ptr {acc}"));
                self.label(&end);
                self.value(format!("load i64, ptr {acc}"))
            }
            Ty::Tuple(types) => {
                let mut hash = seed.to_string();
                for (index, element) in types.iter().enumerate() {
                    let field = self.field_ptr(&llty, place, index);
                    let next = self.hash_at(element, &field, false);
                    hash = self.mix(&hash, &next);
                }
                hash
            }
            Ty::Struct(id) => {
                let fields = self.p.structs[*id].fields.clone();
                let mut hash = seed.to_string();
                for (index, field) in fields.iter().enumerate() {
                    let field_ptr = self.field_ptr(&llty, place, index);
                    let next = self.hash_at(&field.ty, &field_ptr, false);
                    hash = self.mix(&hash, &next);
                }
                hash
            }
            Ty::Enum(id) => {
                let variants = self.p.enums[*id].variants.clone();
                let acc = self.alloca(&Ty::Int, false);
                let tag = self.value(format!("load i64, ptr {place}"));
                let start = self.mix(seed, &tag);
                self.emit(format!("store i64 {start}, ptr {acc}"));
                let end = self.new_label("end");
                let labels: Vec<String> = variants.iter().map(|_| self.new_label("variant")).collect();
                let cases: Vec<String> = labels
                    .iter()
                    .enumerate()
                    .map(|(index, label)| format!("i64 {index}, label %{label}"))
                    .collect();
                self.terminate(format!("switch i64 {tag}, label %{end} [ {} ]", cases.join(" ")));
                for (index, variant) in variants.iter().enumerate() {
                    self.label(&labels[index]);
                    let variant_ty = self.variant_ty(*id, index);
                    let payload = self.field_ptr(&llty, place, 1);
                    let mut hash = start.clone();
                    for (field_index, field) in variant.fields.iter().enumerate() {
                        let field_ptr = self.field_ptr(&variant_ty, &payload, field_index);
                        let next = self.hash_at(&field.ty, &field_ptr, false);
                        hash = self.mix(&hash, &next);
                    }
                    self.emit(format!("store i64 {hash}, ptr {acc}"));
                    self.br(&end);
                }
                self.label(&end);
                self.value(format!("load i64, ptr {acc}"))
            }
            _ => "0".to_string(),
        }
    }
}
