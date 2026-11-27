//! Code generation: the typed program to LLVM IR, as text.
//!
//! # Representation
//!
//! | Koo | LLVM |
//! | --- | --- |
//! | `int`, `float`, `bool`, `byte` | `i64`, `double`, `i1`, `i8` |
//! | `string`, `[T]` | pointer to `{ rc, len, cap, data... }` |
//! | error value, `Failure` | pointer to `{ rc, type id, fields... }` |
//! | struct | named struct, fields in order |
//! | enum | `{ i64 tag, [n x i64] payload }` |
//! | `T?` | `{ i1 present, T }` |
//! | `T ! E` | `{ ptr error, T }`; the error pointer is null on success |
//!
//! Structs, enums, optionals and results are *aggregates*: they live in memory and are
//! passed by pointer. Everything else is a *scalar* and is passed by value.
//!
//! # Ownership
//!
//! Every evaluated expression yields an owned value, which someone must drop. A value read
//! in place (`borrow`, `addr`) is not owned and is not dropped. Whatever owns a value that
//! needs dropping is a memory slot registered with a scope, and every such slot is zero
//! before it is first written and after it is dropped or moved from. All-zero is a valid
//! empty state for every type, so any path out of a scope may drop every slot registered
//! in it, and there are no drop flags.
//!
//! This is the unoptimized form of the ownership model of Section 5: the optimizations of
//! Section 13.1 step 7 (count elision, last-use moves) are not implemented yet, except for
//! borrowed arguments and moves out of temporaries.

use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use koo_syntax::{LineIndex, SourceMap, Span};

use crate::hir::*;

mod closures;
mod exprs;
mod glue;
mod stmts;
mod unique;

/// The reference count of a static value. It is never freed.
/// The count of a constant, which is never counted (see `@koo.retain` below).
const IMMORTAL: i64 = -1;
/// What a count that is frozen has added to it: the smallest integer.
const FROZEN: i64 = i64::MIN;

/// Generates the module for a program. `src` and `prelude_src` are the texts of the
/// program and of the prelude, for the line numbers in panic reports.
pub fn emit(program: &Program, sources: &SourceMap, prelude_src: &str) -> String {
    let mut generator = Gen {
        p: program,
        sources,
        prelude_index: LineIndex::new(prelude_src),
        namesakes: {
            let mut seen = HashSet::new();
            program
                .functions
                .iter()
                .filter(|function| !seen.insert(function.name.as_str()))
                .map(|function| function.name.clone())
                .collect()
        },
        functions: String::new(),
        strings: Vec::new(),
        string_ids: HashMap::new(),
        messages: Vec::new(),
        glue_queue: Vec::new(),
        glue_seen: HashSet::new(),
        static_closures: Vec::new(),
        fn_refs: Vec::new(),
        f: FnState::default(),
    };
    generator.module()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ScopeKind {
    Function,
    Block,
    Loop,
    Statement,
}

struct Scope {
    kind: ScopeKind,
    /// Slots to drop when the scope is left, in the order they were registered.
    drops: Vec<(String, Ty)>,
}

struct LoopLabels {
    break_to: String,
    continue_to: String,
    /// The index in `scopes` of the scope of the loop's body.
    scope: usize,
}

#[derive(Default)]
struct FnState {
    /// The function being generated.
    id: FnId,
    /// Whether it comes from the prelude.
    prelude: bool,
    /// For a function of the prelude that is told where the program called it: where
    /// that is kept, a string of the file and the line.
    called_from: Option<String>,
    /// The source file it is in, for panic reports.
    file: String,
    /// Instructions for the entry block: every `alloca` and its zeroing.
    entry: String,
    body: String,
    next_tmp: usize,
    next_label: usize,
    /// Whether the current block already ends in a terminator.
    terminated: bool,
    /// Where each local variable lives: a pointer to its storage.
    locals: Vec<Option<String>>,
    scopes: Vec<Scope>,
    loops: Vec<LoopLabels>,
    /// Lists that an enclosing loop made uniquely owned and that stay so inside it.
    unique: Vec<LocalId>,
    /// For the lists among them that the loop pushes to: a slot that holds the length.
    /// Nothing else in the loop changes it, so `push` reads it there, not from the list,
    /// and the next `push` need not wait for the length this one stored.
    lens: HashMap<LocalId, String>,
    ret: Ty,
    /// Blocks that report a panic, by message and line.
    panics: Vec<(String, usize, usize)>,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum Glue {
    Drop(Ty),
    Retain(Ty),
    /// Marks a value and all it holds as shared between threads.
    Freeze(Ty),
    Fmt(Ty),
    /// Structural equality of two values (Section 3.4).
    Eq(Ty),
    /// A hash that agrees with `==`.
    Hash(Ty),
    /// Frees the box of an error value whose count reached zero.
    FreeError(ErrorId),
    Unique(Ty),
    Push(Ty),
    /// `Push` for a list that is known to be uniquely owned, and whose length the
    /// function also keeps in a slot of its own.
    PushUnique(Ty),
    Pop(Ty),
    FreeList(Ty),
    /// Frees what a closure captured, and the closure.
    FreeClosure(FnId),
}

struct Gen<'a> {
    p: &'a Program,
    sources: &'a SourceMap,
    prelude_index: LineIndex,
    /// The names that more than one function has.
    namesakes: HashSet<String>,
    functions: String,
    strings: Vec<String>,
    string_ids: HashMap<String, usize>,
    /// Panic messages, as constant byte arrays.
    messages: Vec<String>,
    glue_queue: Vec<Glue>,
    glue_seen: HashSet<Glue>,
    /// Lambdas that capture nothing, and named functions used as values: one static
    /// value each.
    static_closures: Vec<FnId>,
    fn_refs: Vec<FnId>,
    f: FnState,
}

fn is_aggregate(ty: &Ty) -> bool {
    matches!(
        ty,
        Ty::Struct(_) | Ty::Enum(_) | Ty::Optional(_) | Ty::Result(..) | Ty::Tuple(_)
    )
}

/// The alias metadata for a load or store of a scalar of the type.
///
/// Koo has no way to read memory as another type, so a store of one kind of scalar cannot
/// change a value of another kind. Telling LLVM so lets it keep a list's length and
/// reference count in registers across stores to the list's elements. The kinds are the
/// header words of strings, lists and error values (`!2`), `int` (`!4`), `float` (`!6`),
/// `bool` (`!8`), `byte` (`!10`), and handles to strings, lists and errors (`!12`).
fn tbaa(ty: &Ty) -> &'static str {
    match ty {
        Ty::Int => ", !tbaa !4",
        Ty::Float => ", !tbaa !6",
        Ty::Bool => ", !tbaa !8",
        Ty::Byte => ", !tbaa !10",
        Ty::Str | Ty::List(_) | Ty::Error(_) | Ty::Failure | Ty::Fn(_) => ", !tbaa !12",
        _ => "",
    }
}

/// What a load of a value of the type may also say about the value. A handle that is read
/// as a value always points to a header: only a slot that holds no value is null, and such
/// a slot is read by the drop glue alone, which does not use this. Knowing that the header
/// can be read is what lets LLVM move a load of a list's length out of a loop.
fn load_facts(ty: &Ty) -> &'static str {
    match ty {
        Ty::Str | Ty::List(_) | Ty::Fn(_) => ", !nonnull !13, !dereferenceable !14, !align !15",
        Ty::Error(_) | Ty::Failure => ", !nonnull !13, !dereferenceable !16, !align !15",
        _ => "",
    }
}

const TBAA: &str = r#"
!0 = !{!"koo"}
!1 = !{!"header", !0, i64 0}
!2 = !{!1, !1, i64 0}
!3 = !{!"int", !0, i64 0}
!4 = !{!3, !3, i64 0}
!5 = !{!"float", !0, i64 0}
!6 = !{!5, !5, i64 0}
!7 = !{!"bool", !0, i64 0}
!8 = !{!7, !7, i64 0}
!9 = !{!"byte", !0, i64 0}
!10 = !{!9, !9, i64 0}
!11 = !{!"handle", !0, i64 0}
!12 = !{!11, !11, i64 0}
!13 = !{}
!14 = !{i64 24}
!15 = !{i64 8}
!16 = !{i64 16}
"#;

fn escape_bytes(bytes: &[u8]) -> String {
    let mut out = String::new();
    for &b in bytes {
        if (b' '..=b'~').contains(&b) && b != b'"' && b != b'\\' {
            out.push(b as char);
        } else {
            let _ = write!(out, "\\{b:02X}");
        }
    }
    out
}

impl<'a> Gen<'a> {
    // ---- types --------------------------------------------------------------------------

    fn llty(&self, ty: &Ty) -> String {
        match ty {
            Ty::Int => "i64".to_string(),
            Ty::Float => "double".to_string(),
            Ty::Bool => "i1".to_string(),
            Ty::Byte => "i8".to_string(),
            Ty::Str | Ty::List(_) | Ty::Error(_) | Ty::Failure | Ty::Fn(_) => "ptr".to_string(),
            Ty::Unit | Ty::Never | Ty::Unknown => "{}".to_string(),
            Ty::Tuple(types) => {
                let fields: Vec<String> = types.iter().map(|t| self.llty(t)).collect();
                format!("{{ {} }}", fields.join(", "))
            }
            Ty::Struct(id) => format!("%\"S.{}\"", self.p.structs[*id].name),
            Ty::Enum(id) => format!("%\"E.{}\"", self.p.enums[*id].name),
            Ty::Optional(inner) => format!("{{ i1, {} }}", self.llty(inner)),
            Ty::Result(ok, _) => format!("{{ ptr, {} }}", self.llty(ok)),
        }
    }

    /// The size and alignment of a type, by the layout rules LLVM applies.
    fn size_align(&self, ty: &Ty) -> (u64, u64) {
        fn layout(fields: impl Iterator<Item = (u64, u64)>) -> (u64, u64) {
            let (mut size, mut align) = (0u64, 1u64);
            for (field_size, field_align) in fields {
                size = size.div_ceil(field_align) * field_align + field_size;
                align = align.max(field_align);
            }
            (size.div_ceil(align) * align, align)
        }
        match ty {
            Ty::Int | Ty::Float => (8, 8),
            Ty::Bool | Ty::Byte => (1, 1),
            Ty::Str | Ty::List(_) | Ty::Error(_) | Ty::Failure | Ty::Fn(_) => (8, 8),
            Ty::Unit | Ty::Never | Ty::Unknown => (0, 1),
            Ty::Tuple(types) => layout(types.iter().map(|t| self.size_align(t))),
            Ty::Struct(id) => layout(self.p.structs[*id].fields.iter().map(|f| self.size_align(&f.ty))),
            Ty::Enum(id) => (8 + 8 * self.payload_words(*id), 8),
            Ty::Optional(inner) => layout([(1, 1), self.size_align(inner)].into_iter()),
            Ty::Result(ok, _) => layout([(8, 8), self.size_align(ok)].into_iter()),
        }
    }

    /// How many 8-byte words the largest variant of an enum needs.
    fn payload_words(&self, id: EnumId) -> u64 {
        self.p.enums[id]
            .variants
            .iter()
            .map(|variant| {
                let (mut size, mut align) = (0u64, 1u64);
                for field in &variant.fields {
                    let (field_size, field_align) = self.size_align(&field.ty);
                    size = size.div_ceil(field_align) * field_align + field_size;
                    align = align.max(field_align);
                }
                size.div_ceil(align) * align
            })
            .max()
            .unwrap_or(0)
            .div_ceil(8)
    }

    fn size(&self, ty: &Ty) -> u64 {
        self.size_align(ty).0
    }

    /// Whether dropping a value of the type does anything.
    fn droppable(&self, ty: &Ty) -> bool {
        match ty {
            Ty::Str | Ty::List(_) | Ty::Error(_) | Ty::Failure | Ty::Result(..) | Ty::Fn(_) => true,
            Ty::Tuple(types) => types.iter().any(|t| self.droppable(t)),
            Ty::Struct(id) => self.p.structs[*id].fields.iter().any(|f| self.droppable(&f.ty)),
            Ty::Enum(id) => self.p.enums[*id]
                .variants
                .iter()
                .any(|v| v.fields.iter().any(|f| self.droppable(&f.ty))),
            Ty::Optional(inner) => self.droppable(inner),
            _ => false,
        }
    }

    /// A name for a type that is usable inside an LLVM identifier.
    fn mangle(&self, ty: &Ty) -> String {
        match ty {
            Ty::Int => "int".to_string(),
            Ty::Float => "float".to_string(),
            Ty::Bool => "bool".to_string(),
            Ty::Byte => "byte".to_string(),
            Ty::Str => "str".to_string(),
            Ty::Unit | Ty::Never | Ty::Unknown => "unit".to_string(),
            Ty::List(element) => format!("L[{}]", self.mangle(element)),
            Ty::Optional(inner) => format!("O[{}]", self.mangle(inner)),
            Ty::Result(ok, _) => format!("R[{}]", self.mangle(ok)),
            Ty::Struct(id) => format!("S.{}", self.p.structs[*id].name),
            Ty::Enum(id) => format!("E.{}", self.p.enums[*id].name),
            Ty::Error(id) => format!("X.{}", self.p.errors[*id].name),
            Ty::Failure => "failure".to_string(),
            Ty::Tuple(types) => format!(
                "T[{}]",
                types.iter().map(|t| self.mangle(t)).collect::<Vec<_>>().join(",")
            ),
            Ty::Fn(_) => "fn".to_string(),
        }
    }

    fn glue(&mut self, glue: Glue) -> String {
        // Every result with one success type has one representation, whatever its error
        // set, and so one set of glue.
        fn erase(ty: &Ty) -> Ty {
            match ty {
                Ty::Result(ok, _) => Ty::Result(Box::new(erase(ok)), ErrSet::Any),
                Ty::List(inner) => Ty::list(erase(inner)),
                Ty::Optional(inner) => Ty::optional(erase(inner)),
                Ty::Tuple(types) => Ty::Tuple(types.iter().map(erase).collect()),
                // Every function value is dropped, retained and printed the same way.
                Ty::Fn(_) => Ty::Fn(Box::new(FnTy {
                    params: Vec::new(),
                    ret: Ty::Unit,
                })),
                other => other.clone(),
            }
        }
        let glue = match glue {
            Glue::Drop(ty) => Glue::Drop(erase(&ty)),
            Glue::Retain(ty) => Glue::Retain(erase(&ty)),
            Glue::Freeze(ty) => Glue::Freeze(erase(&ty)),
            Glue::Fmt(ty) => Glue::Fmt(erase(&ty)),
            Glue::Eq(ty) => Glue::Eq(erase(&ty)),
            Glue::Hash(ty) => Glue::Hash(erase(&ty)),
            Glue::Unique(ty) => Glue::Unique(erase(&ty)),
            Glue::Push(ty) => Glue::Push(erase(&ty)),
            Glue::PushUnique(ty) => Glue::PushUnique(erase(&ty)),
            Glue::Pop(ty) => Glue::Pop(erase(&ty)),
            Glue::FreeList(ty) => Glue::FreeList(erase(&ty)),
            Glue::FreeError(id) => Glue::FreeError(id),
            Glue::FreeClosure(id) => Glue::FreeClosure(id),
        };
        let name = match &glue {
            Glue::Drop(ty) => format!("@\"drop.{}\"", self.mangle(ty)),
            Glue::Retain(ty) => format!("@\"retain.{}\"", self.mangle(ty)),
            Glue::Freeze(ty) => format!("@\"freeze.{}\"", self.mangle(ty)),
            Glue::Fmt(ty) => format!("@\"fmt.{}\"", self.mangle(ty)),
            Glue::Eq(ty) => format!("@\"eq.{}\"", self.mangle(ty)),
            Glue::Hash(ty) => format!("@\"hash.{}\"", self.mangle(ty)),
            Glue::FreeError(id) => format!("@\"free.X.{}\"", self.p.errors[*id].name),
            Glue::Unique(ty) => format!("@\"unique.{}\"", self.mangle(ty)),
            Glue::Push(ty) => format!("@\"push.{}\"", self.mangle(ty)),
            Glue::PushUnique(ty) => format!("@\"push.unique.{}\"", self.mangle(ty)),
            Glue::Pop(ty) => format!("@\"pop.{}\"", self.mangle(ty)),
            Glue::FreeList(ty) => format!("@\"free.{}\"", self.mangle(ty)),
            Glue::FreeClosure(id) => format!("@\"free.closure.{}\"", self.p.functions[*id].name),
        };
        if self.glue_seen.insert(glue.clone()) {
            self.glue_queue.push(glue);
        }
        name
    }

    fn error_box_ty(&self, id: ErrorId) -> String {
        format!("%\"X.{}\"", self.p.errors[id].name)
    }

    fn variant_ty(&self, id: EnumId, variant: usize) -> String {
        let def = &self.p.enums[id];
        format!("%\"E.{}.{}\"", def.name, def.variants[variant].name)
    }

    // ---- constants ----------------------------------------------------------------------

    /// A static string value.
    fn string(&mut self, text: &str) -> String {
        let id = match self.string_ids.get(text) {
            Some(&id) => id,
            None => {
                let id = self.strings.len();
                self.strings.push(text.to_string());
                self.string_ids.insert(text.to_string(), id);
                id
            }
        };
        format!("@str.{id}")
    }

    fn line(&self, span: Span) -> usize {
        if self.f.prelude {
            self.prelude_index.line(span.start) as usize
        } else {
            self.sources.location(span.start).1 as usize
        }
    }

    /// The name of the source of the function being generated, as a string value.
    /// For a panic that a function of the library raises itself: the name of the
    /// function, with -1 for a line, or nothing to say where, with 0.
    fn library_place(&mut self) -> (String, i64) {
        let name = self.p.functions[self.f.id].name.clone();
        // An assertion says the line of the test it is in, which is the place.
        let named = if name.starts_with("test.") {
            None
        } else {
            library_name(&name)
        };
        match named {
            Some(name) => (self.string(&name), -1),
            None => (self.string("the prelude"), 0),
        }
    }

    fn file_string(&mut self) -> String {
        let name = if self.f.prelude {
            "the prelude".to_string()
        } else {
            self.f.file.clone()
        };
        self.string(&name)
    }

    // ---- emission -----------------------------------------------------------------------

    fn tmp(&mut self) -> String {
        self.f.next_tmp += 1;
        format!("%t{}", self.f.next_tmp)
    }

    fn new_label(&mut self, hint: &str) -> String {
        self.f.next_label += 1;
        format!("{hint}.{}", self.f.next_label)
    }

    /// Emits one instruction. Code after a terminator goes into a block nothing reaches.
    fn emit(&mut self, instruction: impl AsRef<str>) {
        if self.f.terminated {
            let label = self.new_label("dead");
            let _ = writeln!(self.f.body, "{label}:");
            self.f.terminated = false;
        }
        let _ = writeln!(self.f.body, "  {}", instruction.as_ref());
    }

    /// Emits an instruction that defines a value, and returns the value's name.
    fn value(&mut self, instruction: impl AsRef<str>) -> String {
        let name = self.tmp();
        self.emit(format!("{name} = {}", instruction.as_ref()));
        name
    }

    fn terminate(&mut self, instruction: impl AsRef<str>) {
        if !self.f.terminated {
            let _ = writeln!(self.f.body, "  {}", instruction.as_ref());
            self.f.terminated = true;
        }
    }

    fn br(&mut self, label: &str) {
        self.terminate(format!("br label %{label}"));
    }

    fn cond_br(&mut self, cond: &str, then: &str, otherwise: &str) {
        self.terminate(format!("br i1 {cond}, label %{then}, label %{otherwise}"));
    }

    /// Starts a block, falling into it from the current one.
    fn label(&mut self, label: &str) {
        self.br(label);
        let _ = writeln!(self.f.body, "{label}:");
        self.f.terminated = false;
    }

    /// Allocates a slot in the entry block. A slot that owns something droppable starts zero.
    fn alloca(&mut self, ty: &Ty, zero: bool) -> String {
        let name = self.tmp();
        let llty = self.llty(ty);
        let _ = writeln!(self.f.entry, "  {name} = alloca {llty}");
        if zero {
            let _ = writeln!(self.f.entry, "  store {llty} zeroinitializer, ptr {name}");
        }
        name
    }

    fn zero(&mut self, ty: &Ty, slot: &str) {
        let llty = self.llty(ty);
        self.emit(format!("store {llty} zeroinitializer, ptr {slot}"));
    }

    fn load(&mut self, ty: &Ty, ptr: &str) -> String {
        if matches!(ty, Ty::Unit | Ty::Never | Ty::Unknown) {
            return "zeroinitializer".to_string();
        }
        let llty = self.llty(ty);
        self.value(format!("load {llty}, ptr {ptr}{}{}", tbaa(ty), load_facts(ty)))
    }

    fn store(&mut self, ty: &Ty, value: &str, ptr: &str) {
        if matches!(ty, Ty::Unit | Ty::Never | Ty::Unknown) {
            return;
        }
        let llty = self.llty(ty);
        self.emit(format!("store {llty} {value}, ptr {ptr}{}", tbaa(ty)));
    }

    fn memcpy(&mut self, ty: &Ty, dest: &str, src: &str) {
        let size = self.size(ty);
        if size > 0 {
            self.emit(format!(
                "call void @llvm.memcpy.p0.p0.i64(ptr {dest}, ptr {src}, i64 {size}, i1 false)"
            ));
        }
    }

    /// A pointer to field `index` of the struct type `struct_ty` at `ptr`.
    fn field_ptr(&mut self, struct_ty: &str, ptr: &str, index: usize) -> String {
        self.value(format!(
            "getelementptr inbounds {struct_ty}, ptr {ptr}, i32 0, i32 {index}"
        ))
    }

    // ---- scopes and ownership -----------------------------------------------------------

    fn push_scope(&mut self, kind: ScopeKind) {
        self.f.scopes.push(Scope {
            kind,
            drops: Vec::new(),
        });
    }

    /// Emits the drops of the scopes from `from` up, innermost first, without leaving them.
    fn cleanup_from(&mut self, from: usize) {
        if self.f.terminated {
            return;
        }
        for index in (from..self.f.scopes.len()).rev() {
            let drops = self.f.scopes[index].drops.clone();
            for (slot, ty) in drops.into_iter().rev() {
                let drop = self.glue(Glue::Drop(ty.clone()));
                self.emit(format!("call void {drop}(ptr {slot})"));
                self.zero(&ty, &slot);
            }
        }
    }

    fn pop_scope(&mut self) {
        self.cleanup_from(self.f.scopes.len() - 1);
        self.f.scopes.pop();
    }

    /// Makes the innermost scope own the slot.
    fn own(&mut self, slot: &str, ty: &Ty) {
        if self.droppable(ty) {
            self.f
                .scopes
                .last_mut()
                .unwrap()
                .drops
                .push((slot.to_string(), ty.clone()));
        }
    }

    /// Makes the innermost block (not statement) own the slot: a local variable lives until
    /// the end of its block.
    fn own_in_block(&mut self, slot: &str, ty: &Ty) {
        if self.droppable(ty) {
            let scope = self
                .f
                .scopes
                .iter_mut()
                .rev()
                .find(|s| s.kind != ScopeKind::Statement)
                .unwrap();
            scope.drops.push((slot.to_string(), ty.clone()));
        }
    }

    /// A slot for a temporary, owned by the innermost scope.
    fn temp(&mut self, ty: &Ty) -> String {
        let droppable = self.droppable(ty);
        let slot = self.alloca(ty, droppable);
        self.own(&slot, ty);
        slot
    }

    /// Adds one reference to a string, list or error value.
    fn retain_value(&mut self, ty: &Ty, value: &str) {
        if self.droppable(ty) {
            self.emit(format!("call void @koo.retain(ptr {value})"));
        }
    }

    /// Adds references for everything the value at `ptr` holds, after it was copied there.
    fn retain_at(&mut self, ty: &Ty, ptr: &str) {
        if self.droppable(ty) {
            let retain = self.glue(Glue::Retain(ty.clone()));
            self.emit(format!("call void {retain}(ptr {ptr})"));
        }
    }

    /// Freezes the value at `ptr`, and everything it holds: none of it is changed again,
    /// and all of it may be reached from other threads.
    fn freeze_at(&mut self, ty: &Ty, ptr: &str) {
        if self.droppable(ty) {
            let freeze = self.glue(Glue::Freeze(ty.clone()));
            self.emit(format!("call void {freeze}(ptr {ptr})"));
        }
    }

    fn drop_at(&mut self, ty: &Ty, ptr: &str) {
        if self.droppable(ty) {
            let drop = self.glue(Glue::Drop(ty.clone()));
            self.emit(format!("call void {drop}(ptr {ptr})"));
        }
    }

    /// A block that reports a panic and does not return.
    fn panic_label(&mut self, message: &str, span: Span) -> String {
        let line = self.line(span);
        let id = match self.messages.iter().position(|m| m == message) {
            Some(id) => id,
            None => {
                self.messages.push(message.to_string());
                self.messages.len() - 1
            }
        };
        if !self.f.panics.iter().any(|(_, m, l)| *m == id && *l == line) {
            let label = format!("panic.{id}.{line}");
            self.f.panics.push((label, id, line));
        }
        format!("panic.{id}.{line}")
    }

    /// Continues only if `ok` holds; otherwise panics with `message`.
    fn check(&mut self, ok: &str, message: &str, span: Span) {
        let panic = self.panic_label(message, span);
        let next = self.new_label("ok");
        self.cond_br(ok, &next, &panic);
        self.label(&next);
    }

    // ---- the module ---------------------------------------------------------------------

    fn module(&mut self) -> String {
        let sources = self.sources;
        for file in sources.files() {
            self.string(&file.name);
        }
        self.string("the prelude");
        for id in 0..self.p.functions.len() {
            self.function(id);
        }
        let entry = self.entry_point();
        // Glue may need more glue; run until none is left.
        let mut glue = String::new();
        while let Some(next) = self.glue_queue.pop() {
            glue.push_str(&self.emit_glue(&next));
        }

        let mut out = String::new();
        let _ = writeln!(
            out,
            "; Generated by the Koo compiler from {}.",
            self.sources.files()[0].name
        );
        out.push_str(PRELUDE);
        out.push_str(&crate::intrinsics::declarations());
        out.push('\n');
        for def in &self.p.structs {
            let fields: Vec<String> = def.fields.iter().map(|f| self.llty(&f.ty)).collect();
            let _ = writeln!(out, "%\"S.{}\" = type {{ {} }}", def.name, fields.join(", "));
        }
        for (id, def) in self.p.enums.iter().enumerate() {
            let words = self.payload_words(id);
            let _ = writeln!(out, "%\"E.{}\" = type {{ i64, [{words} x i64] }}", def.name);
            for variant in &def.variants {
                let fields: Vec<String> = variant.fields.iter().map(|f| self.llty(&f.ty)).collect();
                let _ = writeln!(
                    out,
                    "%\"E.{}.{}\" = type {{ {} }}",
                    def.name,
                    variant.name,
                    fields.join(", ")
                );
            }
        }
        for def in &self.p.errors {
            let mut fields = vec!["i64".to_string(), "i64".to_string()];
            fields.extend(def.fields.iter().map(|f| self.llty(&f.ty)));
            let _ = writeln!(out, "%\"X.{}\" = type {{ {} }}", def.name, fields.join(", "));
        }
        // An error with no fields is one static value.
        for (id, def) in self.p.errors.iter().enumerate() {
            if def.fields.is_empty() {
                let _ = writeln!(
                    out,
                    "@\"error.{}\" = private unnamed_addr constant %\"X.{}\" {{ i64 {IMMORTAL}, i64 {id} }}",
                    def.name, def.name
                );
            }
        }
        out.push('\n');
        for (id, text) in self.strings.iter().enumerate() {
            let len = text.len();
            let data = if len == 0 {
                "zeroinitializer".to_string()
            } else {
                format!("c\"{}\"", escape_bytes(text.as_bytes()))
            };
            let _ = writeln!(
                out,
                "@str.{id} = private unnamed_addr constant {{ i64, i64, i64, [{len} x i8] }} {{ i64 {IMMORTAL}, i64 {len}, i64 {len}, [{len} x i8] {data} }}"
            );
        }
        for (id, message) in self.messages.iter().enumerate() {
            let _ = writeln!(
                out,
                "@msg.{id} = private constant [{} x i8] c\"{}\"",
                message.len(),
                escape_bytes(message.as_bytes())
            );
        }
        out.push('\n');
        for id in 0..self.p.functions.len() {
            if self.p.functions[id].closure {
                out.push_str(&self.closure_type_def(id));
            }
        }
        out.push_str(&self.static_function_values());
        out.push('\n');
        out.push_str(&self.functions);
        out.push_str(&entry);
        out.push_str(&glue);
        out.push_str(TBAA);
        out
    }

    fn fn_name(&self, id: FnId) -> String {
        let name = &self.p.functions[id].name;
        // Two functions may be shown by one name, as a program's own `json<T>` and a
        // library's; what the linker sees is never the same for two.
        if self.namesakes.contains(name) {
            format!("@\"koo.{name}#{id}\"")
        } else {
            format!("@\"koo.{name}\"")
        }
    }

    /// How a parameter is passed: its LLVM type, and whether that is a pointer to storage.
    fn param_abi(&self, local: &Local) -> (String, bool) {
        let by_pointer = matches!(local.kind, LocalKind::Param(ParamKind::Var)) || is_aggregate(&local.ty);
        if by_pointer {
            ("ptr".to_string(), true)
        } else {
            (self.llty(&local.ty), false)
        }
    }

    fn function(&mut self, id: FnId) {
        let function = &self.p.functions[id];
        self.f = FnState {
            id,
            prelude: function.prelude,
            file: self.sources.location(function.span.start).0.to_string(),
            ret: function.ret.clone(),
            locals: vec![None; function.locals.len()],
            ..FnState::default()
        };
        self.push_scope(ScopeKind::Function);

        let mut params: Vec<String> = Vec::new();
        if is_aggregate(&function.ret) {
            params.push("ptr noalias %ret".to_string());
        }
        if function.closure {
            // A lambda's function takes the function value it belongs to.
            params.push("ptr %env".to_string());
            self.bind_captures(id);
        }
        for (index, local) in function.locals[..function.params].iter().enumerate() {
            let (llty, by_pointer) = self.param_abi(local);
            // Two arguments never name storage that one of them changes (Section 5.4), so
            // every pointer parameter is `noalias`.
            let noalias = if by_pointer { " noalias" } else { "" };
            params.push(format!("{llty}{noalias} %p{index}"));
            let owned = local.kind == LocalKind::Param(ParamKind::Take);
            let slot = if by_pointer {
                format!("%p{index}")
            } else {
                let slot = self.alloca(&local.ty, false);
                let _ = writeln!(self.f.entry, "  store {llty} %p{index}, ptr {slot}");
                slot
            };
            if owned {
                self.own(&slot, &local.ty);
            }
            self.f.locals[index] = Some(slot);
        }
        if function.located && function.params > 0 && !function.name.starts_with("test.") {
            self.f.called_from = self.f.locals[function.params - 1].clone();
        }

        self.block_body(&function.body, None);
        // Reaching the end returns `()`.
        if !self.f.terminated {
            match &function.ret {
                Ty::Unit => {
                    self.cleanup_from(0);
                    self.terminate("ret void");
                }
                Ty::Result(ok, _) if **ok == Ty::Unit => {
                    self.emit("store ptr null, ptr %ret");
                    self.cleanup_from(0);
                    self.terminate("ret void");
                }
                // The checker has shown that a function with a value to return does not
                // reach its end.
                _ => self.terminate("unreachable"),
            }
        }

        let ret = if is_aggregate(&function.ret) || matches!(function.ret, Ty::Unit | Ty::Never | Ty::Unknown) {
            "void".to_string()
        } else {
            self.llty(&function.ret)
        };
        let name = self.fn_name(id);
        let mut text = format!("define internal {ret} {name}({}) {{\nentry:\n", params.join(", "));
        text.push_str(&self.f.entry);
        text.push_str("  br label %start\nstart:\n");
        text.push_str(&self.f.body);
        // A panic in a function of the library is not at a line of the program: it says
        // which function it was in, which the program called.
        let within = match (self.f.prelude, library_name(&function.name)) {
            (false, _) => None,
            (true, Some(name)) => {
                self.string(&name);
                Some((self.string_ids[&name], -1))
            }
            // A function of the library's own, which the program does not know of.
            (true, None) => {
                self.string("the prelude");
                Some((self.string_ids["the prelude"], 0))
            }
        };
        let called_from = self.f.called_from.clone().map(|slot| {
            let name = library_name(&function.name).unwrap_or_default();
            self.string(&name);
            (slot, self.string_ids[&name])
        });
        for (label, message, line) in &self.f.panics {
            let len = self.messages[*message].len();
            if let Some((slot, name)) = &called_from {
                let _ = writeln!(
                    text,
                    "{label}:\n  %from.{label} = load ptr, ptr {slot}\n  \
                     call void @koo_panic_from(ptr @msg.{message}, i64 {len}, ptr @str.{name}, ptr %from.{label})\n  unreachable"
                );
                continue;
            }
            let (file, line) = match within {
                Some(place) => place,
                None => (self.string_ids[&self.f.file], *line as i64),
            };
            let _ = writeln!(
                text,
                "{label}:\n  call void @koo_panic(ptr @msg.{message}, i64 {len}, ptr @str.{file}, i64 {line})\n  unreachable"
            );
        }
        text.push_str("}\n\n");
        self.functions.push_str(&text);
    }

    /// The C `main`: runs the program's `main`, and reports an error it returns.
    /// Calls a function that takes nothing and returns `()` or `() ! E`, as `main` and the
    /// tests are. An error it returns is reported, and the program exits with status 1.
    fn run_entry(&mut self, id: FnId, tag: &str) -> String {
        let name = self.fn_name(id);
        if let Ty::Result(..) = &self.p.functions[id].ret {
            let fmt = self.glue(Glue::Fmt(Ty::Failure));
            format!(
                "  %result{tag} = alloca {{ ptr, {{}} }}\n  store ptr null, ptr %result{tag}\n  call void {name}(ptr %result{tag})\n  %error{tag} = load ptr, ptr %result{tag}\n  %failed{tag} = icmp ne ptr %error{tag}, null\n  br i1 %failed{tag}, label %fail{tag}, label %done{tag}\nfail{tag}:\n  %text{tag} = alloca ptr\n  %new{tag} = call ptr @koo_str_new(i64 64)\n  store ptr %new{tag}, ptr %text{tag}\n  call void {fmt}(ptr %text{tag}, ptr %result{tag})\n  %message{tag} = load ptr, ptr %text{tag}\n  call void @koo_main_failed(ptr %message{tag})\n  unreachable\ndone{tag}:\n"
            )
        } else {
            format!("  call void {name}()\n")
        }
    }

    /// The C `main`: runs the program's `main`, and reports an error it returns. For
    /// `koo test` it runs the test whose number `KOO_TEST` gives, or lists the tests.
    fn entry_point(&mut self) -> String {
        let mut text =
            String::from("define i32 @main(i32 %argc, ptr %argv) {\n  call void @koo_init(i32 %argc, ptr %argv)\n");
        // What runs a message that was told to an actor (`koo-rt/src/actor.rs`).
        if self.p.functions.iter().any(|function| function.name == "a__job") {
            text.push_str("  call void @koo_actor_runner(ptr @\"koo.a__job\")\n");
        }
        // What answers a message that another process has for an actor of this one.
        if self
            .p
            .functions
            .iter()
            .any(|function| function.name == "a__serve_remote")
        {
            text.push_str("  call void @koo_actor_answerer(ptr @\"koo.a__serve_remote\")\n");
        }
        if self.p.test_mode {
            let tests = self.p.tests.clone();
            let cases: Vec<String> = (0..tests.len()).map(|i| format!("i64 {i}, label %test{i}")).collect();
            let _ = writeln!(
                text,
                "  %index = call i64 @koo_test_index()\n  switch i64 %index, label %list [ {} ]\nlist:",
                cases.join(" ")
            );
            for (name, _) in &tests {
                let value = self.string(name);
                let _ = writeln!(text, "  call void @koo_print(ptr {value})");
            }
            text.push_str("  br label %end\n");
            for (index, (_, id)) in tests.iter().enumerate() {
                let _ = writeln!(text, "test{index}:");
                text.push_str(&self.run_entry(*id, &format!(".{index}")));
                text.push_str("  br label %end\n");
            }
            text.push_str("end:\n");
        } else if let Some(main) = self.p.main {
            text.push_str(&self.run_entry(main, ""));
        } else {
            return String::new();
        }
        // The actors that are still running stop when `main` returns (Section 7.3).
        if self.p.functions.iter().any(|function| function.name == "a__stop_all") {
            text.push_str("  call void @\"koo.a__stop_all\"()\n");
        }
        text.push_str("  call void @koo_exit_main()\n  ret i32 0\n}\n\n");
        text
    }
}

/// Declarations every module starts with: the runtime, the intrinsics, and the two helpers
/// that every reference count goes through.
const PRELUDE: &str = r#"
declare noalias nonnull ptr @koo_alloc(i64)
declare noalias nonnull ptr @koo_alloc_zeroed(i64)
declare void @koo_free(ptr)
declare void @koo_panic(ptr, i64, ptr, i64) noreturn cold
declare void @koo_panic_str(ptr, ptr, i64) noreturn cold
declare void @koo_panic_from(ptr, i64, ptr, ptr) noreturn cold
declare void @koo_panic_str_from(ptr, ptr, ptr) noreturn cold
declare void @koo_main_failed(ptr) noreturn cold
declare void @koo_flush()
declare void @koo_exit_main()
declare i32 @_setjmp(ptr) returns_twice
declare ptr @koo_guard_enter()
declare void @koo_actor_runner(ptr)
declare void @koo_actor_answerer(ptr)
declare void @koo_init(i32, ptr)
declare i64 @koo_test_index()
declare void @koo_print(ptr)
declare noalias nonnull dereferenceable(24) ptr @koo_str_new(i64)
declare noalias nonnull dereferenceable(24) ptr @koo_str_concat(ptr, ptr)
declare void @koo_str_append(ptr, ptr)
declare void @koo_str_append_quoted(ptr, ptr)
declare void @koo_str_append_bytes(ptr, ptr, i64)
declare void @koo_str_append_int(ptr, i64)
declare void @koo_str_append_float(ptr, double, i32)
declare void @koo_str_append_bool(ptr, i1 zeroext)
declare void @koo_str_append_padded(ptr, ptr, i64, i32, i1 zeroext)
declare zeroext i1 @koo_str_eq(ptr, ptr)
declare i32 @koo_str_cmp(ptr, ptr)
declare i64 @koo_str_hash(ptr)
declare i64 @koo_float_compare(double, double)
declare zeroext i1 @koo_int_parse(ptr, ptr)
declare zeroext i1 @koo_float_parse(ptr, ptr)
declare noalias nonnull dereferenceable(24) ptr @koo_list_grow(ptr, i64, i64)
declare void @koo_list_sort_ints(ptr)
declare noalias nonnull dereferenceable(24) ptr @koo_list_zeroed(i64, i64)
declare noalias nonnull dereferenceable(24) ptr @koo_list_new(i64, i64)

declare { i64, i1 } @llvm.sadd.with.overflow.i64(i64, i64)
declare { i64, i1 } @llvm.ssub.with.overflow.i64(i64, i64)
declare { i64, i1 } @llvm.smul.with.overflow.i64(i64, i64)
declare i64 @llvm.smin.i64(i64, i64)
declare i64 @llvm.smax.i64(i64, i64)
declare i64 @llvm.abs.i64(i64, i1)
declare double @llvm.sqrt.f64(double)
declare double @llvm.fabs.f64(double)
declare double @llvm.floor.f64(double)
declare double @llvm.ceil.f64(double)
declare double @llvm.round.f64(double)
declare double @llvm.trunc.f64(double)
declare double @llvm.minimum.f64(double, double)
declare double @llvm.maximum.f64(double, double)
declare void @llvm.memcpy.p0.p0.i64(ptr noalias, ptr noalias, i64, i1)
declare void @llvm.memmove.p0.p0.i64(ptr, ptr, i64, i1)

@koo.empty = private unnamed_addr constant { i64, i64, i64 } { i64 -1, i64 0, i64 0 }

; A count says who may reach the value (`koo-rt/src/lib.rs`):
;   above zero: one thread, and that many references, counted with plain arithmetic;
;   -1: a constant, never counted and never written;
;   below -2^62: frozen, which is never changed again and may be reached from any thread;
;       its references, counted atomically, are the count less the smallest integer.
define internal void @koo.retain(ptr %value) alwaysinline {
  %rc = load atomic i64, ptr %value unordered, align 8, !tbaa !2
  %local = icmp sgt i64 %rc, 0
  br i1 %local, label %count, label %shared
count:
  %more = add i64 %rc, 1
  store i64 %more, ptr %value, !tbaa !2
  ret void
shared:
  %frozen = icmp slt i64 %rc, -4611686018427387904
  br i1 %frozen, label %atomic, label %done
atomic:
  %old = atomicrmw add ptr %value, i64 1 monotonic
  br label %done
done:
  ret void
}

; Takes one reference from the value. Returns whether that was the last one.
define internal i1 @koo.release(ptr %value) alwaysinline {
  %rc = load atomic i64, ptr %value unordered, align 8, !tbaa !2
  %local = icmp sgt i64 %rc, 0
  br i1 %local, label %count, label %shared
count:
  %less = sub i64 %rc, 1
  store i64 %less, ptr %value, !tbaa !2
  %last = icmp eq i64 %less, 0
  ret i1 %last
shared:
  %frozen = icmp slt i64 %rc, -4611686018427387904
  br i1 %frozen, label %atomic, label %static
atomic:
  %old = atomicrmw sub ptr %value, i64 1 acq_rel
  %gone = icmp eq i64 %old, -9223372036854775807
  ret i1 %gone
static:
  ret i1 false
}
"#;

/// The name of a function of the library as a program knows it, for a panic that
/// happens inside it: `random__int` is `random.int`. Nothing for a function that the
/// library has for its own use, whose name would mean nothing to the program.
pub(super) fn library_name(name: &str) -> Option<String> {
    // A lambda, a method that stands behind an operator, or the one loop that a chain
    // of calls was made into, has no name to give.
    if name.starts_with("lambda ") || name.contains("._") {
        return None;
    }
    match name.split_once("__") {
        None => Some(name.to_string()),
        // A type of a standard package has the package in its name: `Http__Reply.json`
        // is `http.Reply.json`.
        Some((package, function)) if package.len() > 1 && !function.contains("__") => Some(format!(
            "{}.{function}",
            crate::check::package_shown(&package.to_ascii_lowercase())
        )),
        Some(_) => None,
    }
}

#[cfg(test)]
mod tests {
    /// A function of the library that a program calls, and that panics by its own
    /// choice, is told where the program called it, so that the panic says the line.
    #[test]
    fn a_library_function_that_panics_says_where() {
        let prelude = crate::PRELUDE;
        let mut starts: Vec<usize> = prelude.match_indices("\nfn ").map(|(at, _)| at + 1).collect();
        starts.push(prelude.len());
        let mut without = Vec::new();
        for pair in starts.windows(2) {
            let function = &prelude[pair[0]..pair[1]];
            let header = function.lines().next().unwrap();
            let name = header.rsplit_once('(').map_or(header, |(before, _)| before);
            let name = name.rsplit([' ', ')']).next().unwrap();
            // What the library has for its own use, behind one letter: `e__require`.
            let own = name.split_once("__").is_some_and(|(package, _)| package.len() < 2) || name.starts_with('_');
            if function.contains("panic(") && !own && !header.contains("location: string") {
                without.push(header.to_string());
            }
        }
        assert!(
            without.is_empty(),
            "these panic without `location: string` as a last parameter: {without:#?}"
        );
    }
}
