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
const BUILT_IN: [&str; 5] = ["int", "float", "bool", "string", "byte"];

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
    /// The structures with type parameters, each with the names of its parameters: a
    /// `Page<T>` has JSON for every `T` that has.
    generics: HashMap<&'a str, (Vec<&'a str>, &'a [Field])>,
    /// The type parameters of the generic structure being written, which stand for
    /// types that have JSON; empty when no such structure is.
    params: Vec<&'a str>,
    /// The error types with fields, and the names that stand for several errors.
    error_sets: HashMap<&'a str, &'a [Type]>,
    actors: Vec<&'a ActorDecl>,
    /// How many messages may wait for each actor that says, with `@mailbox(n)`.
    mailboxes: HashMap<&'a str, i64>,
    /// How the fields of a type are named in JSON, for each type that says, with
    /// `@json(rename: camel)`.
    renames: HashMap<&'a str, Rename>,
    /// The one for the type whose fields are being written.
    rename: Rename,
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

/// How the name of a field becomes its key in JSON (Section 10.4).
#[derive(Clone, Copy, PartialEq)]
enum Rename {
    /// As it is written, which is the default: `created_at`.
    Snake,
    /// `createdAt`.
    Camel,
    /// `CreatedAt`.
    Pascal,
    /// `created-at`.
    Kebab,
}

impl Rename {
    fn of(&self, name: &str) -> String {
        match self {
            Rename::Snake => name.to_string(),
            Rename::Kebab => name.replace('_', "-"),
            Rename::Camel | Rename::Pascal => {
                let mut out = String::new();
                let mut upper = *self == Rename::Pascal;
                for c in name.chars() {
                    if c == '_' {
                        upper = true;
                    } else if upper {
                        out.push(c.to_ascii_uppercase());
                        upper = false;
                    } else {
                        out.push(c);
                    }
                }
                out
            }
        }
    }
}

/// The key that a field's own `@json("createdAt")` gives it, if it has one.
fn own_key(field: &Field) -> Option<&str> {
    field.attrs.iter().find_map(
        |attribute| match (attribute.name.name.as_str(), attribute.args.as_slice()) {
            (
                "json",
                [
                    AttrArg {
                        name: None,
                        value:
                            AttrValue::Literal(Expr {
                                kind: ExprKind::Str(parts),
                                ..
                            }),
                    },
                ],
            ) => match parts.as_slice() {
                // A key is written into the text of the functions made for the type, and
                // into JSON, as it is: what would end a string, or is no character to
                // print, cannot be in one.
                [StrPart::Lit(key)]
                    if !key.is_empty()
                        && !key.contains(['"', '\\', '{', '}'])
                        && !key.chars().any(char::is_control) =>
                {
                    Some(key.as_str())
                }
                _ => None,
            },
            _ => None,
        },
    )
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
                BUILT_IN.contains(&name) || self.ok.contains(name) || self.params.contains(&name)
            }
            // A generic structure that has JSON, of types that have.
            TypeKind::Named { path, args } if path.len() == 1 => {
                let name = path[0].name.as_str();
                self.generics.contains_key(name) && self.ok.contains(name) && args.iter().all(|arg| self.holds(arg))
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

    /// `tagged` is for the fields of a variant, which is written with a key `type`
    /// that says which variant it is: no field of it can have that key too.
    fn fields_hold(&self, fields: &'a [Field], tagged: Option<Rename>) -> bool {
        fields.iter().all(|field| {
            // A field that is not written must have something to be when a value is read.
            if field.name.name.starts_with('_') {
                return field.default.is_some();
            }
            let key = |rename: Rename| own_key(field).map_or_else(|| rename.of(&field.name.name), str::to_string);
            tagged.is_none_or(|rename| key(rename) != "type") && self.holds(&field.ty)
        })
    }

    /// A type as it is written.
    fn written(&self, ty: &'a Type) -> String {
        match &self.resolved(ty).kind {
            TypeKind::Named { path, args } if args.is_empty() => path[0].name.clone(),
            TypeKind::Named { path, args } => {
                let args: Vec<String> = args.iter().map(|arg| self.written(arg)).collect();
                format!("{}<{}>", path[0].name, args.join(", "))
            }
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
            TypeKind::Named { path, .. } => matches!(path[0].name.as_str(), "int" | "float" | "bool" | "byte"),
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
                "byte" => "j__byte(text, var at)".to_string(),
                _ => format!("{}._json_read(text, var at)", self.written(ty)),
            },
            // In a generic structure a list or a map may hold what a type parameter
            // stands for, and is read by the functions the prelude has for every list
            // and map; an optional is read by its field itself (`read_fields`).
            TypeKind::List(element) if !self.params.is_empty() => {
                format!("List<{}>._json_read(text, var at)", self.written(element))
            }
            TypeKind::Map(_, value) if !self.params.is_empty() => {
                format!("Map<string, {}>._json_read(text, var at)", self.written(value))
            }
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

    /// The key of a field in JSON: the one it is given by `@json("...")`, or its name
    /// as the type's policy has names.
    fn key(&self, field: &Field) -> String {
        match own_key(field) {
            Some(key) => key.to_string(),
            None => self.rename.of(&field.name.name),
        }
    }

    /// What is wrong with the attributes of fields, and with keys that two fields of
    /// one object would share.
    fn check_keys(&mut self, fields: &'a [Field], rename: Rename) {
        let mut seen: HashMap<String, &'a Field> = HashMap::new();
        for field in fields {
            for attribute in &field.attrs {
                if attribute.name.name != "json" {
                    self.problems.push((
                        attribute.span,
                        format!(
                            "the attribute `@{}` is not supported by this compiler yet",
                            attribute.name.name
                        ),
                        "a field may have `@json(\"key\")`, which is its key in JSON".to_string(),
                    ));
                } else if own_key(field).is_none() {
                    self.problems.push((
                        attribute.span,
                        "`@json` on a field takes the key, as a string".to_string(),
                        "write `@json(\"createdAt\")`: a key that is not empty, with no quote, backslash, brace, tab or end of line in it".to_string(),
                    ));
                }
            }
            if field.name.name.starts_with('_') {
                continue;
            }
            let key = match own_key(field) {
                Some(key) => key.to_string(),
                None => rename.of(&field.name.name),
            };
            if let Some(first) = seen.get(&key) {
                self.problems.push((
                    field.name.span,
                    format!(
                        "the fields `{}` and `{}` have one key in JSON, `{key}`",
                        first.name.name, field.name.name
                    ),
                    "give one of them another key with `@json(\"...\")`".to_string(),
                ));
            } else {
                seen.insert(key, field);
            }
        }
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
            let (name, value) = (&self.key(field), value_of(*index, field));
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
            let _ = writeln!(out, "    var got{index}: {holder} = none");
        }
        out.push_str("    var first := true\n    j__open(text, var at, 123)?\n");
        out.push_str("    while j__more(text, var at, 125, var first)? {\n        key := j__key(text, var at)?\n");
        out.push_str("        match key {\n");
        for (index, field) in &shown {
            let read = if !self.params.is_empty() && self.is_optional(&field.ty) {
                format!("got{index}._json_read_optional(text, var at)")
            } else {
                self.reader(&field.ty)
            };
            let name = &self.key(field);
            let _ = write!(
                out,
                "            \"{name}\" => {{\n                \
                 value := {read} else e => fail j__in(e, \"/{name}\")\n                \
                 got{index} = value\n            }}\n"
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
                let _ = writeln!(out, "        {name}: got{index},");
            } else {
                let key = self.key(field);
                let _ = writeln!(out, "        {name}: got{index} else fail j__missing(\"{key}\"),");
            }
        }
        out.push_str("    }\n");
        for (index, field) in defaulted {
            let name = &field.name.name;
            let _ = writeln!(out, "    if value := got{index} {{ result.{name} = value }}");
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
            // An error of the library's, which the prelude writes and reads.
            let library = match &ty.kind {
                TypeKind::Named { path, args } if args.is_empty() => {
                    let path: Vec<&str> = path.iter().map(|part| part.name.as_str()).collect();
                    match path.as_slice() {
                        ["IoError"] => Some("IoError"),
                        ["ParseError"] => Some("ParseError"),
                        ["JsonError"] => Some("JsonError"),
                        ["sql", "Error"] => Some("SqlError"),
                        ["http", "Error"] => Some("HttpError"),
                        _ => None,
                    }
                }
                _ => None,
            };
            if let Some(library) = library.filter(|library| !self.structs.contains_key(library)) {
                if !names.iter().any(|known| known == library) {
                    names.push(library.to_string());
                }
            } else if let Some(set) = self.error_sets.get(name).filter(|_| rounds < 256) {
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
            "a message is made of numbers, bytes, strings, booleans, lists, optionals, maps with string keys, the \
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
            // What goes to another process is written so that it comes back the same:
            // a float that is not a number as itself, where JSON for others has `null`.
            let mut pack = String::from("    __json_wire(true)\n    var out__ := \"[\"\n");
            for (i, param) in names.iter().enumerate() {
                if i > 0 {
                    pack.push_str("    out__ += \",\"\n");
                }
                let _ = writeln!(pack, "    {param}._json_write(var out__)");
            }
            pack.push_str("    out__ += \"]\"\n    __json_wire(false)\n");

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
                stubs.push_str("    __json_begin()\n    __json_wire(true)\n    var at__ := 0\n");
            }
            if !errors.is_empty() {
                stubs.push_str("    kind__ := __actor_kind()\n");
                for error in &errors {
                    let _ = write!(
                        stubs,
                        "    if kind__ == \"{error}\" {{\n        failed__ := {error}._json_read(text__, var at__) \
                         {UNREADABLE}\n        __json_wire(false)\n        fail failed__\n    }}\n"
                    );
                }
            }
            if let Some(ret) = &sig.ret {
                let read = self.reader(ret).replace("(text, var at)", "(text__, var at__)");
                let _ = write!(
                    stubs,
                    "    value__ := {read} {UNREADABLE}\n    __json_wire(false)\n    return value__\n"
                );
            } else if valued {
                // A method that may fail and returns nothing has read nothing, and
                // what the thread writes next is for anyone again.
                stubs.push_str("    __json_wire(false)\n");
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
                     {pack}    \
                     while __actor_ask(self.home, \"{name} {{self.id}} try.{called}\", out__) == \"false\" {{\n        \
                     __sleep(1000000)\n    }}\n}}\n\n\
                     fn (self: {attempt}) {called}({typed}) -> bool {{\n    if __actor_local(self.home) {{\n        {frozen}        \
                     next__ := 0\n        id__ := self.id\n        \
                     return __actor_try_tell(id__, () -> {{\n            __actor_letter(next__, id__)\n            \
                     var cell__: [{state}] = __actor_take(id__, \"{called}\")\n            \
                     {local}\n            __actor_put(id__, cell__)\n        }})\n    }}\n\
                     {pack}    return __actor_ask(self.home, \"{name} {{self.id}} try.{called}\", out__) == \"true\"\n}}\n\n",
                    typed = params.join(", "),
                    local = call(&format!("{lower}___handle(id__, __actor_home())")),
                );
            }

            // For a message from another process: the arguments are read, and the actor
            // is asked or told as this process would itself.
            let mut read =
                format!("                __json_wire(true)\n                j__open(text, var at, 91) {UNREADABLE}\n");
            for (i, param) in sig.params.iter().enumerate() {
                let reader = self.reader(&param.ty);
                let _ = write!(
                    read,
                    "                j__more(text, var at, 93, var first) {UNREADABLE}\n                \
                     a{i} := {reader} {UNREADABLE}\n"
                );
            }
            // What the actor does with them may write JSON for anyone.
            read.push_str("                __json_wire(false)\n");
            let given: Vec<String> = (0..sig.params.len()).map(|i| format!("a{i}")).collect();
            let given = given.join(", ");
            let answer = |value: &str, kind: &str| {
                format!(
                    "var out := \"\"\n__json_wire(true)\n{value}._json_write(var out)\n__json_wire(false)\n\
                     __actor_reply(\"{kind}\", out)"
                )
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
                // What another process tells: it is left for the actor if the actor has
                // room, and the process is told whether it was. A process that means
                // to wait for room asks again, so that this one, which answers for
                // every actor, never waits on one of them. There is no room to wait
                // for in an actor that is stopped: what is told to it is dropped, as
                // it is in the actor's own process.
                let _ = write!(
                    arms,
                    "            \"try.{called}\" => {{\n{read}                \
                     if self.try_send.{called}({given}) {{\n                    __actor_reply(\"\", \"true\")\n                \
                     }} else if __actor_alive(self.id) {{\n                    __actor_reply(\"\", \"false\")\n                \
                     }} else {{\n                    __actor_reply(\"\", \"gone\")\n                }}\n            }}\n"
                );
            }
        }
        // What `start` is given: every field that has no default.
        let before = self.problems.len();
        let mut given = Vec::new();
        let mut set = Vec::new();
        // What the actor is started with came from another actor, or from none.
        let mut kept = String::new();
        // An actor of a server is kept by the process that watches, whichever process
        // starts it: a worker sends what the actor is started with there (`elsewhere`),
        // and has a handle back. What cannot be sent cannot be started with from a
        // worker, and the first such field is named when one tries.
        let mut pack = String::from("        __json_wire(true)\n        var out__ := \"[\"\n");
        let mut unpack =
            format!("                __json_wire(true)\n                j__open(text, var at, 91) {UNREADABLE}\n");
        let mut passed = Vec::new();
        let mut unsendable: Option<&str> = None;
        for field in actor.fields.iter().filter(|field| field.default.is_none()) {
            let started = field.name.name.as_str();
            if !self.holds(&field.ty) {
                unsendable = unsendable.or(Some(started));
            } else if unsendable.is_none() {
                if !passed.is_empty() {
                    pack.push_str("        out__ += \",\"\n");
                }
                let _ = writeln!(pack, "        {started}._json_write(var out__)");
                let reader = self.reader(&field.ty);
                let _ = write!(
                    unpack,
                    "                j__more(text, var at, 93, var first) {UNREADABLE}\n                \
                     s{} := {reader} {UNREADABLE}\n",
                    passed.len()
                );
                passed.push(format!("{started}: s{}", passed.len()));
            }
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
        // A handle as text says which actor it is of, and no more: what it is made of
        // is the support library's. An actor with a method `to_string` has that.
        let shown = if actor
            .methods
            .iter()
            .any(|method| method.decl.sig.name.name == "to_string")
        {
            String::new()
        } else {
            format!(
                "fn (self: {name}) to_string() -> string {{\n    return \"{name}#{{self.id.bit_and(4294967295)}}\"\n}}\n\n"
            )
        };
        let elsewhere = match unsendable {
            Some(field) => format!(
                "    if __actor_elsewhere() {{\n        panic(\"an actor `{name}` cannot be started by a handler of a server \
                 with several workers: its `{field}` cannot be sent to the process that keeps the server's actors. \
                 Start it in `main`, before `serve`\")\n    }}\n"
            ),
            None => format!(
                "    if __actor_elsewhere() {{\n{pack}        out__ += \"]\"\n        __json_wire(false)\n        \
                 id__ := int.parse(__actor_ask(__actor_parent(), \"{name} 0 .start\", out__)) else -1\n        \
                 return {lower}___handle(id__, __actor_parent())\n    }}\n"
            ),
        };
        let started = match unsendable {
            Some(_) => String::new(),
            None => format!(
                "            \".start\" => {{\n{unpack}                __json_wire(false)\n                \
                 started := {name}.start({})\n                __actor_reply(\"\", \"{{started.id}}\")\n            }}\n",
                passed.join(", ")
            ),
        };
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
             {shown}fn {name}.start({given}) -> {name} {{\n{elsewhere}{kept}    cell__: [{state}] = [{state} {{ {set} }}]\n    \
             return {lower}___handle(__actor_new(\"{name}\", cell__, {mailbox}), __actor_home())\n}}\n\n\
             fn (self: {name}) stop() {{\n    if not __actor_local(self.home) {{\n        \
             __actor_ask(self.home, \"{name} {{self.id}} stop\", \"[]\")\n        return\n    }}\n    \
             if not __actor_alive(self.id) {{ return }}\n    \
             state__: [{state}] = __actor_end(self.id, 0)\n    initial__: [{state}] = __actor_end(self.id, 1)\n}}\n\n\
             fn {lower}___stop_all() {{\n    __actor_scan()\n    while true {{\n        \
             id := __actor_any(\"{name}\")\n        if id < 0 {{ break }}\n        \
             {lower}___handle(id, __actor_home()).stop()\n    }}\n}}\n\n\
             fn {lower}___remote(self: {name}, method: string, text: string) {{\n    __json_begin()\n    \
             var at := 0\n    var first := true\n    match method {{\n{arms}{started}            \
             \"stop\" => {{\n                self.stop()\n                __actor_reply(\"\", \"null\")\n            }}\n            \
             _ => panic(\"the actor `{name}` has no method `{{method}}`\")\n    }}\n}}\n\n{stubs}",
            given = given.join(", "),
            set = set.join(", "),
        );
        true
    }

    fn a_struct(&mut self, bare: &'a str, fields: &'a [Field]) {
        self.rename = self.renames.get(bare).copied().unwrap_or(Rename::Snake);
        // A structure with type parameters is written once, for whatever they are.
        self.params = self
            .generics
            .get(bare)
            .map(|(params, _)| params.clone())
            .unwrap_or_default();
        let full = if self.params.is_empty() {
            bare.to_string()
        } else {
            format!("{bare}<{}>", self.params.join(", "))
        };
        let name = full.as_str();
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
        self.params.clear();
    }

    fn an_enum(&mut self, decl: &'a EnumDecl) {
        let name = decl.name.name.as_str();
        self.rename = self.renames.get(name).copied().unwrap_or(Rename::Snake);
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
pub fn derive(package: &File, json: bool, rows: bool, envs: bool) -> Derived {
    let has_actors = package.items.iter().any(|item| matches!(item.kind, ItemKind::Actor(_)));
    let types = json || has_actors;
    let mut deriver = Deriver {
        structs: HashMap::new(),
        error_sets: HashMap::new(),
        actors: Vec::new(),
        mailboxes: HashMap::new(),
        generics: HashMap::new(),
        params: Vec::new(),
        renames: HashMap::new(),
        rename: Rename::Snake,
        unwritten: Vec::new(),
        problems: Vec::new(),
        enums: HashMap::new(),
        aliases: HashMap::new(),
        ok: HashSet::new(),
        readers: HashMap::new(),
        out: String::new(),
        count: 0,
    };
    // A type that gives itself one of these functions keeps its own: the compiler
    // writes the others, and what joins the type's own to them, so that a structure
    // which holds the type is written with the type's `to_json` and read with its
    // `from_json`. A function of another shape than the compiler's is not one it can
    // join to, and the type then gets nothing, as it did.
    let mut theirs: HashMap<&str, HashSet<&str>> = HashMap::new();
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
            if let Some(TypeKind::Named { path, args }) = owner.map(|ty| &ty.kind) {
                let name = path[0].name.as_str();
                let named = |ty: &Type, wanted: &str| matches!(&ty.kind, TypeKind::Named { path, args } if path.len() == 1 && args.is_empty() && path[0].name == wanted);
                let fits = match (decl.sig.name.name.as_str(), &decl.sig.target) {
                    ("to_json", FnTarget::Method { .. }) => {
                        decl.sig.params.is_empty()
                            && decl.sig.errors == ErrorSpec::None
                            && decl.sig.ret.as_ref().is_some_and(|ret| named(ret, "string"))
                    }
                    ("from_json", FnTarget::TypeLevel(_)) => {
                        decl.sig.params.len() == 1
                            && named(&decl.sig.params[0].ty, "string")
                            && decl.sig.ret.as_ref().is_some_and(|ret| named(ret, name))
                            && matches!(&decl.sig.errors, ErrorSpec::Set(errors)
                                if errors.len() == 1 && named(&errors[0], "JsonError"))
                    }
                    _ => false,
                };
                if fits && path.len() == 1 && args.is_empty() {
                    theirs.entry(name).or_default().insert(decl.sig.name.name.as_str());
                } else {
                    hand_written.insert(name);
                }
            }
        }
    }
    let mut order: Vec<&str> = Vec::new();
    for item in &package.items {
        // `@json(rename: camel)` on a type: how its fields are named in JSON. On an
        // actor an attribute is the actor's, and is looked at below.
        let (named, fields): (Option<&str>, Vec<&[Field]>) = match &item.kind {
            ItemKind::Type(decl) => match &decl.body {
                TypeBody::Struct(fields) => (Some(decl.name.name.as_str()), vec![fields.as_slice()]),
                _ => (Some(decl.name.name.as_str()), Vec::new()),
            },
            ItemKind::Enum(decl) => (
                Some(decl.name.name.as_str()),
                decl.variants
                    .iter()
                    .filter_map(|variant| variant.fields.as_deref())
                    .collect(),
            ),
            ItemKind::Error(decl) => match &decl.body {
                ErrorBody::Fields(fields) => (Some(decl.name.name.as_str()), vec![fields.as_slice()]),
                _ => (Some(decl.name.name.as_str()), Vec::new()),
            },
            _ => (None, Vec::new()),
        };
        if let Some(name) = named {
            let mut rename = Rename::Snake;
            for attribute in &item.attrs {
                let policy = match (attribute.name.name.as_str(), attribute.args.as_slice()) {
                    (
                        "json",
                        [
                            AttrArg {
                                name: Some(key),
                                value: AttrValue::Path { path, args: None },
                            },
                        ],
                    ) if key.name == "rename" && path.len() == 1 => match path[0].name.as_str() {
                        "snake" => Some(Rename::Snake),
                        "camel" => Some(Rename::Camel),
                        "pascal" => Some(Rename::Pascal),
                        "kebab" => Some(Rename::Kebab),
                        _ => None,
                    },
                    _ => None,
                };
                match policy {
                    Some(policy) => rename = policy,
                    None if attribute.name.name == "json" => deriver.problems.push((
                        attribute.span,
                        "`@json` on a type says how its fields are named in JSON".to_string(),
                        "write `@json(rename: camel)`, or `snake`, `pascal` or `kebab`; the rest of what the \
                         specification gives `@json` is not supported by this compiler yet"
                            .to_string(),
                    )),
                    None => deriver.problems.push((
                        attribute.span,
                        format!(
                            "the attribute `@{}` is not supported by this compiler yet",
                            attribute.name.name
                        ),
                        "a type may have `@json(rename: camel)`".to_string(),
                    )),
                }
            }
            if rename != Rename::Snake {
                deriver.renames.insert(name, rename);
            }
            for fields in fields {
                deriver.check_keys(fields, rename);
            }
        }
        match &item.kind {
            ItemKind::Type(decl)
                if !decl.type_params.is_empty()
                    && !decl.resource
                    && !hand_written.contains(decl.name.name.as_str())
                    && !theirs.contains_key(decl.name.name.as_str()) =>
            {
                if let TypeBody::Struct(fields) = &decl.body {
                    let params = decl.type_params.iter().map(|param| param.name.name.as_str()).collect();
                    deriver.generics.insert(&decl.name.name, (params, fields));
                    order.push(&decl.name.name);
                }
            }
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
    let mut handles: Vec<&str> = deriver.actors.iter().map(|actor| actor.name.name.as_str()).collect();
    // And so can a duration and a time, which the prelude writes and reads.
    handles.extend(["Duration", "Time"]);
    deriver.ok.extend(handles.iter().copied());
    loop {
        let mut struck: Vec<&str> = Vec::new();
        for name in order.iter().copied() {
            if !deriver.ok.contains(name) {
                continue;
            }
            let held = if let Some((params, fields)) = deriver.generics.get(name).cloned() {
                // What its type parameters stand for has JSON, or the structure is
                // not asked for JSON with them.
                deriver.params = params;
                let held = deriver.fields_hold(fields, None);
                deriver.params.clear();
                held
            } else if let Some(fields) = deriver.structs.get(name).copied() {
                deriver.fields_hold(fields, None)
            } else if let Some(decl) = deriver.enums.get(name).copied() {
                let rename = deriver.renames.get(name).copied().unwrap_or(Rename::Snake);
                decl.variants
                    .iter()
                    .all(|variant| deriver.fields_hold(variant.fields.as_deref().unwrap_or(&[]), Some(rename)))
            } else {
                handles.contains(&name)
            };
            if !held {
                struck.push(name);
            }
        }
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
        let before = deriver.out.len();
        if let Some((_, fields)) = deriver.generics.get(name).cloned() {
            deriver.a_struct(name, fields);
        } else if let Some(fields) = deriver.structs.get(name).copied() {
            deriver.a_struct(name, fields);
        } else if let Some(decl) = deriver.enums.get(name).copied() {
            deriver.an_enum(decl);
        }
        // What the type has of its own takes the place of what was written for it.
        if let Some(own) = theirs.get(name) {
            let written = deriver.out.split_off(before);
            for function in written.split_inclusive("\n}\n\n") {
                let taken = (own.contains("to_json")
                    && (function.starts_with(&format!("fn (self: {name}) to_json("))
                        || function.starts_with(&format!("fn (self: {name}) _json_write("))))
                    || (own.contains("from_json")
                        && (function.starts_with(&format!("fn {name}.from_json("))
                            || function.starts_with(&format!("fn {name}._json_read("))));
                if !taken {
                    deriver.out.push_str(function);
                }
            }
            if own.contains("to_json") {
                let _ = write!(
                    deriver.out,
                    "fn (self: {name}) _json_write(out: var string) {{\n    out += self.to_json()\n}}\n\n"
                );
            }
            if own.contains("from_json") {
                let _ = write!(
                    deriver.out,
                    "fn {name}._json_read(text: string, at: var int) -> {name} ! JsonError {{\n    \
                     start := __json_ws(text, at)\n    at = start\n    j__skip(text, var at)?\n    \
                     depth := __json_depth()\n    wire := __json_is_wire()\n    \
                     read := {name}.from_json(text.slice(start, at))\n    \
                     __json_resume(depth, wire)\n    value := read?\n    return value\n}}\n\n"
                );
            }
        }
    }
    // `Type.from_row(row)`, for a program that says `from_row`: a struct whose every
    // field is what a column can hold is read from a row of a database, each field from
    // the column of its name (Section 10.3).
    if rows {
        let mut names: Vec<&str> = deriver.structs.keys().copied().collect();
        names.sort_unstable();
        for name in names {
            let fields = deriver.structs[name];
            let read: Option<Vec<String>> = fields
                .iter()
                .map(|field| {
                    let (ty, maybe) = match &deriver.resolved(&field.ty).kind {
                        TypeKind::Optional(inner) => (deriver.resolved(inner), "maybe_"),
                        _ => (deriver.resolved(&field.ty), ""),
                    };
                    let kind = match &ty.kind {
                        TypeKind::Named { path, args } if path.len() == 1 && args.is_empty() => {
                            match path[0].name.as_str() {
                                "int" => "int",
                                "float" => "float",
                                "bool" => "bool",
                                "string" => "string",
                                "Time" => "time",
                                _ => return None,
                            }
                        }
                        // A column that is an array, for a field that is a list.
                        TypeKind::List(element) if maybe.is_empty() => match &deriver.resolved(element).kind {
                            TypeKind::Named { path, args } if path.len() == 1 && args.is_empty() => {
                                match path[0].name.as_str() {
                                    "int" => "ints",
                                    "float" => "floats",
                                    "bool" => "bools",
                                    "string" => "strings",
                                    _ => return None,
                                }
                            }
                            _ => return None,
                        },
                        _ => return None,
                    };
                    Some(format!("{0}: row.{maybe}{kind}(\"{0}\")?", field.name.name))
                })
                .collect();
            if let Some(read) = read.filter(|read| !read.is_empty()) {
                let _ = write!(
                    deriver.out,
                    "fn {name}.from_row(row: SqlRow) -> {name} ! SqlError {{\n    return {name} {{ {} }}\n}}\n\n",
                    read.join(", ")
                );
            }
        }
    }
    // `Type.from_env()`, for a program that says `from_env`: a struct whose every field
    // is what a variable of the environment can say is read from the environment, each
    // field from the variable of its name in capitals. A field with a default keeps it
    // if its variable is not set, an optional one is `none`, and any other must be set.
    if envs {
        let mut names: Vec<&str> = deriver.structs.keys().copied().collect();
        names.sort_unstable();
        for name in names {
            let fields = deriver.structs[name];
            let kinds: Option<Vec<(&Field, &str, bool)>> = fields
                .iter()
                .map(|field| {
                    let (ty, optional) = match &deriver.resolved(&field.ty).kind {
                        TypeKind::Optional(inner) => (deriver.resolved(inner), true),
                        _ => (deriver.resolved(&field.ty), false),
                    };
                    let kind = match &ty.kind {
                        TypeKind::Named { path, args } if path.len() == 1 && args.is_empty() => {
                            match path[0].name.as_str() {
                                "int" => "int",
                                "float" => "float",
                                "bool" => "bool",
                                "string" => "string",
                                "Duration" => "duration",
                                _ => return None,
                            }
                        }
                        _ => return None,
                    };
                    Some((field, kind, optional))
                })
                .collect();
            let Some(kinds) = kinds.filter(|kinds| !kinds.is_empty()) else {
                continue;
            };
            let variable = |field: &Field| field.name.name.to_uppercase();
            let required: Vec<String> = kinds
                .iter()
                .filter(|(field, _, optional)| !optional && field.default.is_none())
                .map(|(field, _, _)| format!("\"{}\"", variable(field)))
                .collect();
            let given: Vec<String> = kinds
                .iter()
                .filter(|(field, _, _)| field.default.is_none())
                .map(|(field, kind, optional)| {
                    let maybe = if *optional { "maybe_" } else { "" };
                    format!("{}: e__{maybe}{kind}(\"{}\")?", field.name.name, variable(field))
                })
                .collect();
            let defaulted: Vec<String> = kinds
                .iter()
                .filter(|(field, _, _)| field.default.is_some())
                .map(|(field, kind, _)| {
                    format!(
                        "    if value := e__maybe_{kind}(\"{}\")? {{ result.{} = value }}\n",
                        variable(field),
                        field.name.name
                    )
                })
                .collect();
            let binding = if defaulted.is_empty() { "result" } else { "var result" };
            let _ = write!(
                deriver.out,
                "fn {name}.from_env() -> {name} ! EnvError {{\n    e__require([{}])?\n    \
                 {binding} := {name} {{ {} }}\n{}    return result\n}}\n\n",
                required.join(", "),
                given.join(", "),
                defaulted.concat()
            );
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
        let derived = derive(&parsed.file, true, false, false).source;
        let again = koo_syntax::parse_file(&derived);
        assert!(again.diagnostics.is_empty(), "{:#?}\n{derived}", again.diagnostics);
        for name in ["Line", "Status", "Payment", "Order"] {
            assert!(derived.contains(&format!("fn {name}.from_json")), "{name}\n{derived}");
        }
        // A function cannot be JSON, and neither can what holds one.
        assert!(!derived.contains("Handler") && !derived.contains("Holder"));
        assert!(!derive(&parsed.file, false, false, false).source.contains("from_json"));
    }
}
