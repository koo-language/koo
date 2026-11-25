//! JSON for the program's own types (Section 10.4).
//!
//! A struct or an enum whose fields are all things JSON can hold gets `value.to_json()`
//! and `Type.from_json(text)` without the program asking. This module writes them, as
//! Koo source built on the prelude's readers and writers, from the declarations alone.
//! The source is then parsed and checked like any other, so what it does to a value is
//! exactly what the same code written by hand would do.
//!
//! The mapping is fixed. A struct is an object keyed by its field names. A `T?` field
//! that is `none` is left out, and a missing key or `null` reads as `none`; a field with
//! a default takes it when its key is missing; any other missing key is an error. An enum
//! whose variants have no fields is a string, the variant's name in snake_case; one with
//! fields is an object with the variant's name under `"type"`. Unknown keys are passed
//! over. A mistake in the text is reported with the path to it: `/items/3/qty`.

use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use koo_syntax::Span;
use koo_syntax::ast::*;

/// The types whose JSON the prelude has.
const BUILT_IN: [&str; 4] = ["int", "float", "bool", "string"];

/// What the compiler writes for a program, and what it found that it cannot write.
pub struct Derived {
    pub source: String,
    /// Each a place, what is wrong there, and what to do about it.
    pub problems: Vec<(Span, String, String)>,
    /// What was not written because of a problem, as the checker will say it is missing:
    /// the place that uses it need not also be told.
    pub unwritten: Vec<String>,
}

/// The type of what `handle.send` is, for an actor of a name.
pub fn send_type(actor: &str) -> String {
    format!("Actor{actor}Send")
}

/// The type of what `handle.try_send` is, for an actor of a name.
pub fn try_type(actor: &str) -> String {
    format!("Actor{actor}Try")
}

/// The type that holds the fields of an actor of a name.
pub fn state_type(actor: &str) -> String {
    format!("Actor{actor}State")
}

/// The function that holds the body of an actor's method.
pub fn method_function(actor: &str, method: &str) -> String {
    format!("{}__{method}", own(actor))
}

const UNREADABLE: &str = "else e => panic(\"a message between actors could not be read: {e.message}\")";

struct Deriver<'a> {
    structs: HashMap<&'a str, &'a [Field]>,
    /// The error types with fields, and the names that stand for several errors.
    error_sets: HashMap<&'a str, &'a [Type]>,
    actors: Vec<&'a ActorDecl>,
    /// How many messages may wait for each actor that says, with `@mailbox(n)`.
    mailboxes: HashMap<&'a str, i64>,
    problems: Vec<(Span, String, String)>,
    unwritten: Vec<String>,
    enums: HashMap<&'a str, &'a EnumDecl>,
    aliases: HashMap<&'a str, &'a Type>,
    /// The types JSON is written for.
    ok: HashSet<&'a str>,
    /// The readers written for lists, optionals and maps, by the type each reads.
    readers: HashMap<String, usize>,
    out: String,
    count: usize,
}

/// What the names of the functions written for an actor begin with. No name of the
/// prelude's begins so, whatever the actor is called: an actor `A`, `Io` or `J` would
/// otherwise have functions with the names of the prelude's own. The function behind a
/// method is this, `__` and the method's name; the others have names that begin with
/// one more `_`, which no method's does.
fn own(actor: &str) -> String {
    format!("actor__{}", snake(actor))
}

/// `OrderLine` as `order_line`.
fn snake(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

impl<'a> Deriver<'a> {
    /// The type an alias stands for, or the type itself.
    fn resolved(&self, ty: &'a Type) -> &'a Type {
        let mut ty = ty;
        // An alias of an alias ends somewhere, or is refused by the checker.
        for _ in 0..16 {
            match &ty.kind {
                TypeKind::Named { path, args } if path.len() == 1 && args.is_empty() => {
                    match self.aliases.get(path[0].name.as_str()) {
                        Some(target) => ty = target,
                        None => break,
                    }
                }
                _ => break,
            }
        }
        ty
    }

    /// Whether JSON can hold a value of a type, given the types in `ok`.
    fn holds(&self, ty: &'a Type) -> bool {
        match &self.resolved(ty).kind {
            TypeKind::Named { path, args } if path.len() == 1 && args.is_empty() => {
                let name = path[0].name.as_str();
                BUILT_IN.contains(&name) || self.ok.contains(name)
            }
            TypeKind::List(element) => self.holds(element),
            TypeKind::Optional(inner) => {
                !matches!(self.resolved(inner).kind, TypeKind::Optional(_)) && self.holds(inner)
            }
            TypeKind::Map(key, value) => {
                matches!(&self.resolved(key).kind, TypeKind::Named { path, args }
                    if path.len() == 1 && args.is_empty() && path[0].name == "string")
                    && self.holds(value)
            }
            _ => false,
        }
    }

    fn fields_hold(&self, fields: &'a [Field]) -> bool {
        fields.iter().all(|field| {
            // A field that is not written must have something to be when a value is read.
            if field.name.name.starts_with('_') {
                return field.default.is_some();
            }
            field.name.name != "type" && self.holds(&field.ty)
        })
    }

    /// A type as it is written.
    fn written(&self, ty: &'a Type) -> String {
        match &self.resolved(ty).kind {
            TypeKind::Named { path, .. } => path[0].name.clone(),
            TypeKind::List(element) => format!("[{}]", self.written(element)),
            TypeKind::Optional(inner) => format!("{}?", self.written(inner)),
            TypeKind::Map(_, value) => format!("[string: {}]", self.written(value)),
            _ => unreachable!("only a type JSON can hold is written"),
        }
    }

    /// Whether a value of the type may hold a function. What a function captured cannot
    /// be frozen, so a function cannot go from one actor to another.
    fn has_function(&self, ty: &'a Type, seen: &mut Vec<&'a str>) -> bool {
        let fields = |fields: &'a [Field], deriver: &Self, seen: &mut Vec<&'a str>| {
            fields.iter().any(|field| deriver.has_function(&field.ty, seen))
        };
        match &self.resolved(ty).kind {
            TypeKind::Fn { .. } => true,
            TypeKind::List(inner) | TypeKind::Optional(inner) | TypeKind::Array(inner, _) => {
                self.has_function(inner, seen)
            }
            TypeKind::Map(key, value) => self.has_function(key, seen) || self.has_function(value, seen),
            TypeKind::Tuple(types) => types.iter().any(|ty| self.has_function(ty, seen)),
            TypeKind::Struct(inside) => fields(inside, self, seen),
            TypeKind::Result { ok, .. } => self.has_function(ok, seen),
            TypeKind::Named { path, args } => {
                if args.iter().any(|arg| self.has_function(arg, seen)) {
                    return true;
                }
                let name = path[0].name.as_str();
                // A server holds the functions that answer its routes.
                if path.len() == 2 && name == "http" && path[1].name == "App" {
                    return true;
                }
                if path.len() != 1 || seen.contains(&name) {
                    return false;
                }
                seen.push(name);
                if let Some(inside) = self.structs.get(name) {
                    return fields(inside, self, seen);
                }
                self.enums.get(name).is_some_and(|decl| {
                    decl.variants.iter().any(|variant| {
                        variant
                            .fields
                            .as_deref()
                            .is_some_and(|inside| fields(inside, self, seen))
                    })
                })
            }
        }
    }

    /// Whether a value of the type holds nothing that is counted, so that there is
    /// nothing of it to freeze.
    fn scalar(&self, ty: &'a Type) -> bool {
        match &self.resolved(ty).kind {
            TypeKind::Named { path, .. } => matches!(path[0].name.as_str(), "int" | "float" | "bool"),
            _ => false,
        }
    }

    fn is_optional(&self, ty: &'a Type) -> bool {
        matches!(self.resolved(ty).kind, TypeKind::Optional(_))
    }

    /// The call that reads a value of a type from `text` at `at`.
    fn reader(&mut self, ty: &'a Type) -> String {
        let ty = self.resolved(ty);
        match &ty.kind {
            TypeKind::Named { path, .. } => match path[0].name.as_str() {
                "int" => "j__int(text, var at)".to_string(),
                "float" => "j__float(text, var at)".to_string(),
                "bool" => "j__bool(text, var at)".to_string(),
                "string" => "j__str(text, var at)".to_string(),
                name => format!("{name}._json_read(text, var at)"),
            },
            _ => {
                let key = self.written(ty);
                let number = match self.readers.get(&key) {
                    Some(number) => *number,
                    None => {
                        self.count += 1;
                        let number = self.count;
                        self.readers.insert(key.clone(), number);
                        self.composite_reader(ty, &key, number);
                        number
                    }
                };
                format!("j__r{number}(text, var at)")
            }
        }
    }

    /// The reader of a list, an optional or a map.
    fn composite_reader(&mut self, ty: &'a Type, written: &str, number: usize) {
        let mut body = String::new();
        match &ty.kind {
            TypeKind::List(element) => {
                let read = self.reader(element);
                let _ = write!(
                    body,
                    "    var items: {written} = []\n    var first := true\n    j__open(text, var at, 91)?\n    \
                     while j__more(text, var at, 93, var first)? {{\n        \
                     item := {read} else e => fail j__in(e, \"/{{items.len}}\")\n        \
                     items.push(item)\n    }}\n    return items\n"
                );
            }
            TypeKind::Optional(inner) => {
                let read = self.reader(inner);
                let _ = write!(
                    body,
                    "    if j__null(text, var at) {{ return none }}\n    value := {read}?\n    return value\n"
                );
            }
            TypeKind::Map(_, value) => {
                let read = self.reader(value);
                let _ = write!(
                    body,
                    "    var entries: {written} = [:]\n    var first := true\n    j__open(text, var at, 123)?\n    \
                     while j__more(text, var at, 125, var first)? {{\n        \
                     key := j__key(text, var at)?\n        \
                     value := {read} else e => fail j__in(e, \"/{{key}}\")\n        \
                     entries[key] = value\n    }}\n    return entries\n"
                );
            }
            _ => unreachable!("a named type has a reader of its own"),
        }
        let _ = write!(
            self.out,
            "fn j__r{number}(text: string, at: var int) -> {written} ! JsonError {{\n{body}}}\n\n"
        );
    }

    /// The statements that write the fields of an object, each read from `value_of(field)`.
    fn write_fields(&self, fields: &'a [Field], value_of: &dyn Fn(usize, &Field) -> String, started: bool) -> String {
        let shown: Vec<(usize, &Field)> = fields
            .iter()
            .enumerate()
            .filter(|(_, field)| !field.name.name.starts_with('_'))
            .collect();
        // With a field that may be left out, where the commas go is known only as the
        // value is written.
        let counted = shown.iter().any(|(_, field)| self.is_optional(&field.ty));
        let mut out = String::new();
        if counted {
            let _ = writeln!(out, "    var count := {}", usize::from(started));
        }
        for (position, (index, field)) in shown.iter().enumerate() {
            let (name, value) = (&field.name.name, value_of(*index, field));
            if counted {
                let comma = "if count > 0 { out += \",\" }\n        count += 1";
                if self.is_optional(&field.ty) {
                    let _ = write!(
                        out,
                        "    if value := {value} {{\n        {comma}\n        out += \"\\\"{name}\\\":\"\n        \
                         value._json_write(var out)\n    }}\n"
                    );
                } else {
                    let _ = write!(
                        out,
                        "    {comma}\n    out += \"\\\"{name}\\\":\"\n    {value}._json_write(var out)\n"
                    );
                }
            } else {
                let comma = if position > 0 || started { "," } else { "" };
                let _ = write!(
                    out,
                    "    out += \"{comma}\\\"{name}\\\":\"\n    {value}._json_write(var out)\n"
                );
            }
        }
        out
    }

    /// The body of a function that reads an object into `made`, a struct or a variant
    /// literal without its braces.
    fn read_fields(&mut self, fields: &'a [Field], made: &str) -> String {
        let mut out = String::new();
        let shown: Vec<(usize, &'a Field)> = fields
            .iter()
            .enumerate()
            .filter(|(_, field)| !field.name.name.starts_with('_'))
            .collect();
        for (index, field) in &shown {
            let written = self.written(&field.ty);
            let holder = if self.is_optional(&field.ty) {
                written
            } else {
                format!("{written}?")
            };
            let _ = writeln!(out, "    var f{index}: {holder} = none");
        }
        out.push_str("    var first := true\n    j__open(text, var at, 123)?\n");
        out.push_str("    while j__more(text, var at, 125, var first)? {\n        key := j__key(text, var at)?\n");
        out.push_str("        match key {\n");
        for (index, field) in &shown {
            let (name, read) = (&field.name.name, self.reader(&field.ty));
            let _ = write!(
                out,
                "            \"{name}\" => {{\n                \
                 value := {read} else e => fail j__in(e, \"/{name}\")\n                \
                 f{index} = value\n            }}\n"
            );
        }
        out.push_str("            _ => j__skip(text, var at)?\n        }\n    }\n");
        // A field with a default keeps it unless the text gives another.
        let defaulted: Vec<&(usize, &Field)> = shown.iter().filter(|(_, f)| f.default.is_some()).collect();
        let binding = if defaulted.is_empty() { "result" } else { "var result" };
        let _ = writeln!(out, "    {binding} := {made} {{");
        for (index, field) in &shown {
            let name = &field.name.name;
            if field.default.is_some() {
                continue;
            }
            if self.is_optional(&field.ty) {
                let _ = writeln!(out, "        {name}: f{index},");
            } else {
                let _ = writeln!(out, "        {name}: f{index} else fail j__missing(\"{name}\"),");
            }
        }
        out.push_str("    }\n");
        for (index, field) in defaulted {
            let name = &field.name.name;
            let _ = writeln!(out, "    if value := f{index} {{ result.{name} = value }}");
        }
        out.push_str("    return result\n");
        out
    }

    /// The functions every type gets on top of its reader and its writer.
    fn entry_points(&mut self, name: &str) {
        let _ = write!(
            self.out,
            "fn (self: {name}) to_json() -> string {{\n    var out := \"\"\n    self._json_write(var out)\n    \
             return out\n}}\n\n\
             fn {name}.from_json(text: string) -> {name} ! JsonError {{\n    __json_begin()\n    var at := 0\n    \
             value := {name}._json_read(text, var at)?\n    j__end(text, at)?\n    return value\n}}\n\n"
        );
    }

    /// A type as it is written, whatever it is, or nothing for one that is written in
    /// no way a signature can use.
    fn spelled(&self, ty: &'a Type) -> Option<String> {
        Some(match &ty.kind {
            TypeKind::Named { path, args } => {
                let name: Vec<&str> = path.iter().map(|part| part.name.as_str()).collect();
                let mut out = name.join(".");
                if !args.is_empty() {
                    let args: Option<Vec<String>> = args.iter().map(|arg| self.spelled(arg)).collect();
                    out = format!("{out}<{}>", args?.join(", "));
                }
                out
            }
            TypeKind::List(element) => format!("[{}]", self.spelled(element)?),
            TypeKind::Map(key, value) => format!("[{}: {}]", self.spelled(key)?, self.spelled(value)?),
            TypeKind::Optional(inner) => format!("{}?", self.spelled(inner)?),
            TypeKind::Tuple(parts) => {
                let parts: Option<Vec<String>> = parts.iter().map(|part| self.spelled(part)).collect();
                format!("({})", parts?.join(", "))
            }
            _ => return None,
        })
    }

    /// The error types a method may fail with, by name, or nothing if one of them
    /// cannot be sent.
    fn errors_of(&mut self, sig: &'a FnSig) -> Option<Vec<String>> {
        let ErrorSpec::Set(types) = &sig.errors else {
            if matches!(sig.errors, ErrorSpec::Inferred) {
                self.cannot_send(sig.name.span, "an error set that is left to be worked out");
                return None;
            }
            return Some(Vec::new());
        };
        let mut names = Vec::new();
        let mut waiting: Vec<&'a Type> = types.iter().collect();
        let mut rounds = 0;
        while let Some(ty) = waiting.pop() {
            rounds += 1;
            let name = match &ty.kind {
                TypeKind::Named { path, args } if path.len() == 1 && args.is_empty() => path[0].name.as_str(),
                _ => "",
            };
            if let Some(set) = self.error_sets.get(name).filter(|_| rounds < 256) {
                waiting.extend(set.iter());
            } else if self.structs.contains_key(name) && self.ok.contains(name) {
                if !names.iter().any(|known| known == name) {
                    names.push(name.to_string());
                }
            } else {
                self.cannot_send(ty.span, "this error");
                return None;
            }
        }
        names.sort();
        Some(names)
    }

    fn cannot_send(&mut self, span: Span, what: &str) {
        self.problems.push((
            span,
            format!("{what} cannot be sent to an actor, or back from one"),
            "a message is made of numbers, strings, booleans, lists, optionals, maps with string keys, the \
             program's structs, enums and errors made of those, and the handles of actors; a function, a tuple \
             or a generic type cannot cross from one actor to another yet"
                .to_string(),
        ));
    }

    /// What carries messages to an actor and back: `start` and `stop`, a method of the
    /// handle for each method of the actor, which asks, and one of `handle.send` for
    /// each that returns nothing, which tells.
    ///
    /// For an actor of the same process, to ask is to take the actor's state, call the
    /// method on it and put it back, and to tell is to keep a closure that will. For an
    /// actor of another process the arguments go as text to the process that has it
    /// (`remote`, below), which asks or tells in the same way and sends the answer back.
    /// Returns whether the actor was written.
    fn an_actor(&mut self, actor: &'a ActorDecl) -> bool {
        let name = actor.name.name.as_str();
        let (send, state, lower) = (send_type(name), state_type(name), own(name));
        let attempt = try_type(name);
        // A thousand and twenty-four messages may wait, unless the actor says how many.
        let mailbox = self.mailboxes.get(name).copied().unwrap_or(0);
        let mut stubs = String::new();
        let mut arms = String::new();
        // The fields, as the function behind a method is given them.
        let fields: Vec<String> = actor
            .fields
            .iter()
            .map(|field| {
                let var = if field.mutable { "var " } else { "" };
                format!("{var}cell__[0].{}", field.name.name)
            })
            .collect();
        for method in &actor.methods {
            // A method that cannot be carried is left out, and the rest are written, so
            // that the one mistake is the one reported.
            let before = self.problems.len();
            let sig = &method.decl.sig;
            let called = sig.name.name.as_str();
            if matches!(called, "send" | "try_send" | "stop" | "start") || called.starts_with('_') {
                self.problems.push((
                    sig.name.span,
                    format!("`{called}` cannot be the name of an actor's method"),
                    "`start`, `stop`, `send` and `try_send` are what every actor's handle has (Section 7.3)"
                        .to_string(),
                ));
                continue;
            }
            for param in &sig.params {
                if !self.holds(&param.ty) {
                    self.cannot_send(param.ty.span, "this type");
                }
            }
            if let Some(ret) = &sig.ret
                && !self.holds(ret)
            {
                self.cannot_send(ret.span, "this type");
            }
            let errors = self.errors_of(sig);
            if self.problems.len() > before || errors.is_none() {
                self.unwritten.push(format!("has no method `{called}`"));
                continue;
            }
            let Some(errors) = errors else { continue };
            let params: Vec<String> = sig
                .params
                .iter()
                .map(|param| format!("{}: {}", param.name.name, self.written(&param.ty)))
                .collect();
            let names: Vec<&str> = sig.params.iter().map(|param| param.name.name.as_str()).collect();
            let returns = sig.ret.as_ref().map(|ret| self.written(ret));
            let valued = returns.is_some() || !errors.is_empty();
            let mut signature = format!("{called}({})", params.join(", "));
            if let Some(returns) = &returns {
                let _ = write!(signature, " -> {returns}");
            }
            if !errors.is_empty() {
                let _ = write!(signature, " ! {}", errors.join(" | "));
            }
            // The call of the function behind the method, on a state that was taken.
            let call = |handle: &str| {
                let mut args = vec![handle.to_string()];
                args.extend(fields.iter().cloned());
                args.extend(names.iter().map(|name| name.to_string()));
                format!("{}({})", method_function(name, called), args.join(", "))
            };
            // The arguments as one JSON array, for another process.
            let mut pack = String::from("    var out__ := \"[\"\n");
            for (i, param) in names.iter().enumerate() {
                if i > 0 {
                    pack.push_str("    out__ += \",\"\n");
                }
                let _ = writeln!(pack, "    {param}._json_write(var out__)");
            }
            pack.push_str("    out__ += \"]\"\n");

            // What crosses to the actor is frozen first, and so is what comes back:
            // another thread may be the next to have the actor, or the value.
            let mut frozen = String::from("__freeze(self)\n");
            for (param, name) in sig.params.iter().zip(&names) {
                if !self.scalar(&param.ty) {
                    let _ = writeln!(frozen, "        __freeze({name})");
                }
            }
            // To ask: the actor is this thread's from `take` to `put`. What was told to
            // it earlier has run by then.
            let _ = write!(
                stubs,
                "fn (self: {name}) {signature} {{\n    if __actor_local(self.home) {{\n        {frozen}        \
                 var cell__: [{state}] = __actor_take(self.id, \"{called}\")\n        \
                 {bind}{local}\n{freeze}        __actor_put(self.id, cell__)\n        \
                 return{value}\n    }}\n{pack}    \
                 text__ := __actor_ask(self.home, \"{name} {{self.id}} {called}\", out__)\n",
                bind = if valued { "value__ := " } else { "" },
                local = call("self"),
                freeze = if valued { "        __freeze(value__)\n" } else { "" },
                value = if valued { " value__" } else { "" },
            );
            if valued {
                stubs.push_str("    __json_begin()\n    var at__ := 0\n");
            }
            if !errors.is_empty() {
                stubs.push_str("    kind__ := __actor_kind()\n");
                for error in &errors {
                    let _ = write!(
                        stubs,
                        "    if kind__ == \"{error}\" {{\n        failed__ := {error}._json_read(text__, var at__) \
                         {UNREADABLE}\n        fail failed__\n    }}\n"
                    );
                }
            }
            if let Some(ret) = &sig.ret {
                let read = self.reader(ret).replace("(text, var at)", "(text__, var at__)");
                let _ = write!(stubs, "    value__ := {read} {UNREADABLE}\n    return value__\n");
            }
            stubs.push_str("}\n\n");
            // To tell: for a method with nothing to send back. What is told is a function
            // that asks, kept by the support library as a letter. The first two things
            // the function holds are the support library's to use, so they are the first
            // two it names: a word to chain the letters of an actor through, and the
            // number of the actor (`koo-rt/src/actor.rs`).
            if !valued {
                let _ = write!(
                    stubs,
                    "fn (self: {send}) {signature} {{\n    if __actor_local(self.home) {{\n        {frozen}        \
                     next__ := 0\n        id__ := self.id\n        \
                     __actor_tell(id__, () -> {{\n            __actor_letter(next__, id__)\n            \
                     var cell__: [{state}] = __actor_take(id__, \"{called}\")\n            \
                     {local}\n            __actor_put(id__, cell__)\n        }}, \"{called}\")\n        return\n    }}\n\
                     {pack}    __actor_send(self.home, \"{name} {{self.id}} send.{called}\", out__)\n}}\n\n\
                     fn (self: {attempt}) {called}({typed}) -> bool {{\n    if __actor_local(self.home) {{\n        {frozen}        \
                     next__ := 0\n        id__ := self.id\n        \
                     return __actor_try_tell(id__, () -> {{\n            __actor_letter(next__, id__)\n            \
                     var cell__: [{state}] = __actor_take(id__, \"{called}\")\n            \
                     {local}\n            __actor_put(id__, cell__)\n        }})\n    }}\n\
                     {pack}    __actor_send(self.home, \"{name} {{self.id}} send.{called}\", out__)\n    return true\n}}\n\n",
                    typed = params.join(", "),
                    local = call(&format!("{lower}___handle(id__, __actor_home())")),
                );
            }

            // For a message from another process: the arguments are read, and the actor
            // is asked or told as this process would itself.
            let mut read = format!("                j__open(text, var at, 91) {UNREADABLE}\n");
            for (i, param) in sig.params.iter().enumerate() {
                let reader = self.reader(&param.ty);
                let _ = write!(
                    read,
                    "                j__more(text, var at, 93, var first) {UNREADABLE}\n                \
                     a{i} := {reader} {UNREADABLE}\n"
                );
            }
            let given: Vec<String> = (0..sig.params.len()).map(|i| format!("a{i}")).collect();
            let given = given.join(", ");
            let answer = |value: &str, kind: &str| {
                format!("var out := \"\"\n{value}._json_write(var out)\n__actor_reply(\"{kind}\", out)")
            };
            let asked = format!("self.{called}({given})");
            let body = match (returns.is_some(), errors.is_empty()) {
                (false, true) => format!("{asked}\n__actor_reply(\"\", \"null\")"),
                (true, true) => format!("value := {asked}\n{}", answer("value", "")),
                (returned, false) => {
                    let mut out = format!("match {asked} {{\n");
                    if returned {
                        let _ = writeln!(out, "    value => {{\n{}\n    }}", answer("value", ""));
                    } else {
                        out.push_str("    _ => __actor_reply(\"\", \"null\")\n");
                    }
                    for error in &errors {
                        let _ = writeln!(out, "    ! e: {error} => {{\n{}\n    }}", answer("e", error));
                    }
                    out.push('}');
                    out
                }
            };
            let _ = write!(arms, "            \"{called}\" => {{\n{read}");
            for line in body.lines() {
                let _ = writeln!(arms, "                {line}");
            }
            arms.push_str("            }\n");
            if !valued {
                let _ = write!(
                    arms,
                    "            \"send.{called}\" => {{\n{read}                self.send.{called}({given})\n            }}\n"
                );
            }
        }
        // What `start` is given: every field that has no default.
        let before = self.problems.len();
        let mut given = Vec::new();
        let mut set = Vec::new();
        // What the actor is started with came from another actor, or from none.
        let mut kept = String::new();
        for field in actor.fields.iter().filter(|field| field.default.is_none()) {
            match self.spelled(&field.ty) {
                Some(_) if self.has_function(&field.ty, &mut Vec::new()) => self.problems.push((
                    field.ty.span,
                    "a function cannot be given to an actor when it is started".to_string(),
                    "what a function has captured would be shared between the actor and whoever started it; \
                     give the field a default, and the actor makes the function itself"
                        .to_string(),
                )),
                Some(ty) => {
                    given.push(format!("{}: {ty}", field.name.name));
                    set.push(format!("{0}: {0}", field.name.name));
                    if !self.scalar(&field.ty) {
                        let _ = writeln!(kept, "    __freeze({})", field.name.name);
                    }
                }
                None => self.problems.push((
                    field.ty.span,
                    "a field of this type must have a default: it cannot be given to `start` yet".to_string(),
                    "give the field a default, or a type that has a name".to_string(),
                )),
            }
        }
        if self.problems.len() > before {
            self.unwritten.push("has no function `start`".to_string());
            return false;
        }
        let _ = write!(
            self.out,
            "fn {lower}___handle(id: int, home: string) -> {name} {{\n    \
             return {name} {{ id, home, send: {send} {{ id, home }}, try_send: {attempt} {{ id, home }} }}\n}}\n\n\
             fn (self: {name}) _json_write(out: var string) {{\n    out += \"\\{{\\\"id\\\":{{self.id}},\\\"home\\\":\"\n    \
             self.home._json_write(var out)\n    out += \"}}\"\n}}\n\n\
             fn {name}._json_read(text: string, at: var int) -> {name} ! JsonError {{\n    \
             var id: int? = none\n    var home: string? = none\n    var first := true\n    \
             j__open(text, var at, 123)?\n    while j__more(text, var at, 125, var first)? {{\n        \
             key := j__key(text, var at)?\n        match key {{\n            \
             \"id\" => {{\n                value := j__int(text, var at)?\n                id = value\n            }}\n            \
             \"home\" => {{\n                value := j__str(text, var at)?\n                home = value\n            }}\n            \
             _ => j__skip(text, var at)?\n        }}\n    }}\n    \
             the_id := id else fail j__missing(\"id\")\n    the_home := home else fail j__missing(\"home\")\n    \
             return {lower}___handle(the_id, the_home)\n}}\n\n\
             fn {name}.start({given}) -> {name} {{\n{kept}    cell__: [{state}] = [{state} {{ {set} }}]\n    \
             return {lower}___handle(__actor_new(\"{name}\", cell__, {mailbox}), __actor_home())\n}}\n\n\
             fn (self: {name}) stop() {{\n    if not __actor_local(self.home) {{\n        \
             __actor_ask(self.home, \"{name} {{self.id}} stop\", \"[]\")\n        return\n    }}\n    \
             if not __actor_alive(self.id) {{ return }}\n    \
             state__: [{state}] = __actor_end(self.id, 0)\n    initial__: [{state}] = __actor_end(self.id, 1)\n}}\n\n\
             fn {lower}___stop_all() {{\n    __actor_scan()\n    while true {{\n        \
             id := __actor_any(\"{name}\")\n        if id < 0 {{ break }}\n        \
             {lower}___handle(id, __actor_home()).stop()\n    }}\n}}\n\n\
             fn {lower}___remote(self: {name}, method: string, text: string) {{\n    __json_begin()\n    \
             var at := 0\n    var first := true\n    match method {{\n{arms}            \
             \"stop\" => {{\n                self.stop()\n                __actor_reply(\"\", \"null\")\n            }}\n            \
             _ => panic(\"the actor `{name}` has no method `{{method}}`\")\n    }}\n}}\n\n{stubs}",
            given = given.join(", "),
            set = set.join(", "),
        );
        true
    }

    fn a_struct(&mut self, name: &'a str, fields: &'a [Field]) {
        let writes = self.write_fields(fields, &|_, field| format!("self.{}", field.name.name), false);
        let _ = write!(
            self.out,
            "fn (self: {name}) _json_write(out: var string) {{\n    out += \"\\{{\"\n{writes}    out += \"}}\"\n}}\n\n"
        );
        let reads = self.read_fields(fields, name);
        let _ = write!(
            self.out,
            "fn {name}._json_read(text: string, at: var int) -> {name} ! JsonError {{\n{reads}}}\n\n"
        );
        self.entry_points(name);
    }

    fn an_enum(&mut self, decl: &'a EnumDecl) {
        let name = decl.name.name.as_str();
        let plain = decl.variants.iter().all(|variant| variant.fields.is_none());
        let mut writes = String::new();
        let mut reads = String::new();
        for variant in &decl.variants {
            let (variant_name, tag) = (&variant.name.name, snake(&variant.name.name));
            let fields: &'a [Field] = variant.fields.as_deref().unwrap_or(&[]);
            if plain {
                let _ = writeln!(writes, "        {variant_name} => out += \"\\\"{tag}\\\"\"");
                let _ = writeln!(reads, "        \"{tag}\" => {{ return {name}.{variant_name} }}");
                continue;
            }
            // A variant's fields are bound to names of the writer's own choosing.
            let bound: Vec<String> = fields
                .iter()
                .enumerate()
                .map(|(i, field)| format!("{}: v{i}", field.name.name))
                .collect();
            let pattern = match &variant.fields {
                Some(_) => format!("{variant_name} {{ {} }}", bound.join(", ")),
                None => variant_name.clone(),
            };
            let body = self.write_fields(fields, &|i, _| format!("v{i}"), true);
            let body = body.replace("\n    ", "\n            ");
            let _ = write!(
                writes,
                "        {pattern} => {{\n            out += \"\\{{\\\"type\\\":\\\"{tag}\\\"\"\n        {body}    \
                 out += \"}}\"\n        }}\n"
            );
            self.count += 1;
            let number = self.count;
            let made = format!("{name}.{variant_name}");
            let read = if variant.fields.is_some() {
                self.read_fields(fields, &made)
            } else {
                // Nothing to read but the object itself.
                format!(
                    "    var first := true\n    j__open(text, var at, 123)?\n    \
                     while j__more(text, var at, 125, var first)? {{\n        j__key(text, var at)?\n        \
                     j__skip(text, var at)?\n    }}\n    return {made}\n"
                )
            };
            // The tag is one of the keys, and is passed over like any the variant lacks.
            let _ = write!(
                self.out,
                "fn j__r{number}(text: string, at: var int) -> {name} ! JsonError {{\n{read}}}\n\n"
            );
            let _ = writeln!(reads, "        \"{tag}\" => {{ return j__r{number}(text, var at)? }}");
        }
        let _ = write!(
            self.out,
            "fn (self: {name}) _json_write(out: var string) {{\n    match self {{\n{writes}    }}\n}}\n\n"
        );
        let names: Vec<String> = decl
            .variants
            .iter()
            .map(|v| format!("\\\"{}\\\"", snake(&v.name.name)))
            .collect();
        let names = names.join(", ");
        if plain {
            let _ = write!(
                self.out,
                "fn {name}._json_read(text: string, at: var int) -> {name} ! JsonError {{\n    \
                 tag := j__str(text, var at)?\n    match tag {{\n{reads}        _ => {{}}\n    }}\n    \
                 fail JsonError {{ message: \"expected one of {names}, found \\\"{{tag}}\\\"\", path: \"\" }}\n}}\n\n"
            );
        } else {
            let _ = write!(
                self.out,
                "fn {name}._json_read(text: string, at: var int) -> {name} ! JsonError {{\n    \
                 at = __json_ws(text, at)\n    if not __json_tag(text, at, \"type\") {{\n        \
                 fail JsonError {{ message: \"expected an object whose \\\"type\\\" is one of {names}\", path: \"\" }}\n    }}\n    \
                 tag := __json_text()\n    match tag {{\n{reads}        _ => {{}}\n    }}\n    \
                 fail JsonError {{ message: \"expected one of {names}, found \\\"{{tag}}\\\"\", path: \"/type\" }}\n}}\n\n"
            );
        }
        self.entry_points(name);
    }
}

/// What the compiler writes for a program: the JSON functions of its types, and what
/// carries messages to its actors and back. Nothing is written for a program that never
/// says `json` and has no actor: nothing is made that nothing uses.
pub fn derive(package: &File, json: bool) -> Derived {
    let has_actors = package.items.iter().any(|item| matches!(item.kind, ItemKind::Actor(_)));
    let types = json || has_actors;
    let mut deriver = Deriver {
        structs: HashMap::new(),
        error_sets: HashMap::new(),
        actors: Vec::new(),
        mailboxes: HashMap::new(),
        unwritten: Vec::new(),
        problems: Vec::new(),
        enums: HashMap::new(),
        aliases: HashMap::new(),
        ok: HashSet::new(),
        readers: HashMap::new(),
        out: String::new(),
        count: 0,
    };
    // A type that gives itself one of these functions keeps its own, and gets none.
    let mut hand_written: HashSet<&str> = HashSet::new();
    for item in &package.items {
        if let ItemKind::Fn(decl) = &item.kind
            && matches!(
                decl.sig.name.name.as_str(),
                "to_json" | "from_json" | "_json_write" | "_json_read"
            )
        {
            let owner = match &decl.sig.target {
                FnTarget::Method { ty: Some(ty), .. } | FnTarget::TypeLevel(ty) => Some(ty),
                _ => None,
            };
            if let Some(TypeKind::Named { path, .. }) = owner.map(|ty| &ty.kind) {
                hand_written.insert(path[0].name.as_str());
            }
        }
    }
    let mut order: Vec<&str> = Vec::new();
    for item in &package.items {
        match &item.kind {
            ItemKind::Type(decl) if decl.type_params.is_empty() && !decl.resource => match &decl.body {
                TypeBody::Struct(fields) if !hand_written.contains(decl.name.name.as_str()) => {
                    deriver.structs.insert(&decl.name.name, fields);
                    order.push(&decl.name.name);
                }
                TypeBody::Alias(target) => {
                    deriver.aliases.insert(&decl.name.name, target);
                }
                _ => {}
            },
            ItemKind::Enum(decl) if decl.type_params.is_empty() && !hand_written.contains(decl.name.name.as_str()) => {
                deriver.enums.insert(&decl.name.name, decl);
                order.push(&decl.name.name);
            }
            // An error is sent as a struct is: an actor's method may fail with one.
            ItemKind::Error(decl) if !hand_written.contains(decl.name.name.as_str()) => match &decl.body {
                ErrorBody::Fields(fields) => {
                    deriver.structs.insert(&decl.name.name, fields);
                    order.push(&decl.name.name);
                }
                ErrorBody::Set(errors) => {
                    deriver.error_sets.insert(&decl.name.name, errors);
                }
            },
            ItemKind::Actor(decl) if decl.type_params.is_empty() && decl.satisfies.is_empty() => {
                deriver.actors.push(decl);
                for attribute in &item.attrs {
                    // `@mailbox(n)`: how many messages may wait for the actor (Section 7.3).
                    let size = match (attribute.name.name.as_str(), attribute.args.as_slice()) {
                        (
                            "mailbox",
                            [
                                AttrArg {
                                    name: None,
                                    value:
                                        AttrValue::Literal(Expr {
                                            kind: ExprKind::Int(size),
                                            ..
                                        }),
                                },
                            ],
                        ) => size.replace('_', "").parse::<i64>().ok().filter(|size| *size > 0),
                        _ => None,
                    };
                    match size {
                        Some(size) => {
                            deriver.mailboxes.insert(&decl.name.name, size);
                        }
                        None if attribute.name.name == "mailbox" => deriver.problems.push((
                            attribute.span,
                            "`@mailbox` takes one number, greater than zero".to_string(),
                            "it is the most messages that may wait for the actor: `@mailbox(4096)`".to_string(),
                        )),
                        None => deriver.problems.push((
                            attribute.span,
                            format!(
                                "the attribute `@{}` is not supported by this compiler yet",
                                attribute.name.name
                            ),
                            "an actor may say `@mailbox(n)`, the most messages that may wait for it".to_string(),
                        )),
                    }
                }
            }
            _ => {}
        }
    }
    // Every type is taken to do, and those with a field JSON cannot hold are struck out
    // until none is left to strike: a type may hold itself, in a list or an optional.
    deriver.ok = order.iter().copied().collect();
    // A handle of an actor is its address, which can always be sent.
    let handles: Vec<&str> = deriver.actors.iter().map(|actor| actor.name.name.as_str()).collect();
    deriver.ok.extend(handles.iter().copied());
    loop {
        let struck: Vec<&str> = order
            .iter()
            .copied()
            .filter(|name| deriver.ok.contains(name))
            .filter(|name| match (deriver.structs.get(name), deriver.enums.get(name)) {
                (Some(fields), _) => !deriver.fields_hold(fields),
                (_, Some(decl)) => !decl
                    .variants
                    .iter()
                    .all(|variant| deriver.fields_hold(variant.fields.as_deref().unwrap_or(&[]))),
                _ => !handles.contains(name),
            })
            .collect();
        if struck.is_empty() {
            break;
        }
        for name in struck {
            deriver.ok.remove(name);
        }
    }
    for name in order {
        if !types || !deriver.ok.contains(name) {
            continue;
        }
        if let Some(fields) = deriver.structs.get(name).copied() {
            deriver.a_struct(name, fields);
        } else if let Some(decl) = deriver.enums.get(name).copied() {
            deriver.an_enum(decl);
        }
    }
    // What every program has, whether or not it has an actor: where a message from
    // another process is handed to the actor it is for, and what stops the actors that
    // are still running when `main` returns, the last declared first.
    let mut handed = String::new();
    let mut stopped = String::new();
    for actor in deriver.actors.clone() {
        if deriver.an_actor(actor) {
            let (name, lower) = (&actor.name.name, own(&actor.name.name));
            let _ = writeln!(
                handed,
                "        \"{name}\" => {lower}___remote({lower}___handle(id, __actor_home()), parts[2], text)"
            );
            stopped.insert_str(0, &format!("    {lower}___stop_all()\n"));
        }
    }
    let _ = write!(
        deriver.out,
        "fn a__dispatch(message: string, text: string) {{\n    parts := message.split(\" \")\n    \
         if parts.len != 3 {{ return }}\n    id := int.parse(parts[1]) else -1\n    match parts[0] {{\n{handed}        \
         _ => {{}}\n    }}\n}}\n\nfn a__stop_all() {{\n    __actor_scan()\n{stopped}}}\n"
    );
    Derived {
        source: deriver.out,
        problems: deriver.problems,
        unwritten: deriver.unwritten,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_written_parses() {
        let source = "type Line = { sku: string, qty: int = 1, note: string?, _seen: bool = false }\n\
                      enum Status { Open, InProgress, Done }\n\
                      enum Payment {\n    Cash\n    Card { last4: string, brand: string? }\n}\n\
                      type Order = { id: int, lines: [Line], status: Status, paid: Payment?, tags: [string: [float]], parent: Order? }\n\
                      type Handler = { run: fn() -> int }\n\
                      type Holder = { handler: Handler }\n";
        let parsed = koo_syntax::parse_file(source);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let derived = derive(&parsed.file, true).source;
        let again = koo_syntax::parse_file(&derived);
        assert!(again.diagnostics.is_empty(), "{:#?}\n{derived}", again.diagnostics);
        for name in ["Line", "Status", "Payment", "Order"] {
            assert!(derived.contains(&format!("fn {name}.from_json")), "{name}\n{derived}");
        }
        // A function cannot be JSON, and neither can what holds one.
        assert!(!derived.contains("Handler") && !derived.contains("Holder"));
        assert!(!derive(&parsed.file, false).source.contains("from_json"));
    }
}
