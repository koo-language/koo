//! Help for a name the compiler does not know: the nearest name it does know, or the Koo
//! spelling of a habit from another language.

use koo_syntax::Span;

use super::Checker;
use super::generics::OwnerKey;
use crate::hir::*;

impl<'a> Checker<'a> {
    /// Reports an unknown name with the best help there is: `habit` if the name is a
    /// habit from elsewhere, or else the candidate nearest to what was written.
    pub(super) fn error_near(
        &mut self,
        span: Span,
        message: String,
        wanted: &str,
        candidates: Vec<String>,
        habit: Option<String>,
    ) {
        let whole = (span.end - span.start) as usize == wanted.len();
        if let Some(habit) = habit {
            // "in Koo that is `string`": a name for a name, shown in place.
            let koo = habit
                .strip_prefix("in Koo that is `")
                .and_then(|rest| rest.strip_suffix('`'))
                .filter(|name| name.chars().all(|c| c.is_alphanumeric() || c == '_'));
            match koo {
                Some(koo) if whole => {
                    self.error(span, message);
                    self.fix(habit.clone(), span, koo);
                }
                _ => self.error_help(span, message, habit),
            }
            return;
        }
        let unknown_name = message.starts_with("unknown name");
        // What a type or a call has is worth listing when it is not much.
        let lists = message.contains(" has no ");
        self.error(span, message);
        if unknown_name {
            self.label("nothing of this name is in scope");
        }
        let mut listed: Vec<&String> = candidates
            .iter()
            .filter(|name| !name.starts_with('_') && !name.contains("__"))
            .collect();
        listed.sort();
        listed.dedup();
        if closest(wanted, &candidates).is_none() && lists && (1..=12).contains(&listed.len()) {
            let names: Vec<String> = listed.iter().map(|name| format!("`{name}`")).collect();
            self.help(format!("it has {}", names.join(", ")));
        }
        if let Some(name) = closest(wanted, &candidates) {
            // The correction is shown in place when the span is the name alone.
            if (span.end - span.start) as usize == wanted.len() {
                self.fix(format!("did you mean `{name}`?"), span, name);
            } else {
                self.help(format!("did you mean `{name}`?"));
            }
        }
    }

    /// The variables in scope and the functions of the program.
    pub(super) fn names_in_scope(&self) -> Vec<String> {
        let mut names: Vec<String> = self.scopes.iter().flat_map(|scope| scope.keys().cloned()).collect();
        names.extend(self.function_names());
        names
    }

    pub(super) fn function_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.free_fns.keys().chain(self.generic_fns.keys()).cloned().collect();
        names.push("print".to_string());
        names.push("panic".to_string());
        names
    }

    /// The functions of an imported standard package.
    pub(super) fn package_names(&self, package: &str) -> Vec<String> {
        let prefix = format!("{package}__");
        self.free_fns
            .keys()
            .chain(self.generic_fns.keys())
            .filter_map(|name| name.strip_prefix(&prefix))
            .map(str::to_string)
            .collect()
    }

    pub(super) fn field_names(&self, ty: &Ty) -> Vec<String> {
        match ty {
            Ty::Struct(id) if !self.is_prelude_type(ty) => self.program.structs[*id]
                .fields
                .iter()
                .map(|f| f.name.clone())
                .collect(),
            Ty::Error(id) => self.program.errors[*id].fields.iter().map(|f| f.name.clone()).collect(),
            Ty::Struct(_) | Ty::List(_) | Ty::Str => vec!["len".to_string()],
            _ => Vec::new(),
        }
    }

    pub(super) fn method_names(&self, ty: &Ty) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        if let Some(owner) = self.owner_of(ty) {
            names.extend(
                self.methods
                    .keys()
                    .filter(|(o, _)| *o == owner)
                    .map(|(_, name)| name.clone()),
            );
        }
        let key = match ty {
            Ty::List(_) => Some(OwnerKey::List),
            Ty::Optional(_) => Some(OwnerKey::Optional),
            other => self
                .instance_of
                .get(other)
                .map(|(generic, _)| OwnerKey::Generic(generic.clone())),
        };
        if let Some(key) = key {
            names.extend(
                self.generic_methods
                    .keys()
                    .filter(|(k, _)| *k == key)
                    .map(|(_, name)| name.clone()),
            );
        }
        let built_in: &[&str] = match ty {
            Ty::List(_) => &["push", "pop", "get", "swap", "clear"],
            Ty::Int => &["abs", "compare", "min", "max"],
            Ty::Float => &[
                "abs", "compare", "min", "max", "sqrt", "floor", "ceil", "round", "trunc",
            ],
            Ty::Str | Ty::Byte | Ty::Bool => &["compare"],
            _ => &[],
        };
        names.extend(built_in.iter().map(|name| name.to_string()));
        names.push("to_string".to_string());
        names
    }

    /// Help for `x.name` where `x` has no such field.
    pub(super) fn field_habit(&self, ty: &Ty, wanted: &str) -> Option<String> {
        if self.method_names(ty).iter().any(|name| name == wanted) {
            return Some(format!("`{wanted}` is a method: call it, `.{wanted}()`"));
        }
        let has_len = matches!(ty, Ty::List(_) | Ty::Str) || self.is_prelude_type(ty);
        let lowered = wanted.to_lowercase();
        if has_len && (matches!(lowered.as_str(), "size" | "count") || lowered.starts_with("len")) {
            return Some("the number of elements is `.len`".to_string());
        }
        None
    }

    /// The Koo name of a method that `wanted` names in another language.
    pub(super) fn method_rename(&self, ty: &Ty, wanted: &str) -> Option<&'static str> {
        let koo = match wanted {
            "append" | "add" | "push_back" | "addLast" => "push",
            "toUpperCase" | "upper" | "to_uppercase" | "toUpper" | "ToUpper" => "to_upper",
            "toLowerCase" | "lower" | "to_lowercase" | "toLower" | "ToLower" => "to_lower",
            "strip" | "trimSpace" | "Trim" => "trim",
            "includes" | "has" | "include" | "Contains" | "contains_key" | "containsKey" => "contains",
            "forEach" | "for_each" | "foreach" => "each",
            "reduce" | "inject" | "aggregate" => "fold",
            "indexOf" | "index" => "index_of",
            "substring" | "substr" | "sub" => "slice",
            "startsWith" | "startswith" | "has_prefix" | "hasPrefix" => "starts_with",
            "endsWith" | "endswith" | "has_suffix" | "hasSuffix" => "ends_with",
            "select" | "where" => "filter",
            "delete" | "erase" | "discard" => "remove",
            "put" | "set" => "insert",
            "str" | "toString" | "String" | "to_str" => "to_string",
            _ => return None,
        };
        self.method_names(ty).iter().any(|name| name == koo).then_some(koo)
    }

    /// Help for `x.name()` where `x` has no such method.
    pub(super) fn method_habit(&self, ty: &Ty, wanted: &str) -> Option<String> {
        if self.field_names(ty).iter().any(|name| name == wanted) {
            return Some(format!(
                "`{wanted}` is a field: write it without the parentheses, `.{wanted}`"
            ));
        }
        let has = |name: &str| self.method_names(ty).iter().any(|n| n == name);
        let koo = match wanted {
            "length" | "size" | "count" if matches!(ty, Ty::List(_) | Ty::Str) => {
                return Some("the number of elements is the field `.len`".to_string());
            }
            "append" | "add" | "push_back" | "addLast" => "push",
            "toUpperCase" | "upper" | "to_uppercase" | "toUpper" | "ToUpper" => "to_upper",
            "toLowerCase" | "lower" | "to_lowercase" | "toLower" | "ToLower" => "to_lower",
            "strip" | "trimSpace" | "Trim" => "trim",
            "includes" | "has" | "include" | "Contains" | "contains_key" | "containsKey" => "contains",
            "forEach" | "for_each" | "foreach" => "each",
            "reduce" | "inject" | "aggregate" => "fold",
            "indexOf" | "index" => "index_of",
            "substring" | "substr" | "sub" => "slice",
            "startsWith" | "startswith" | "has_prefix" | "hasPrefix" => "starts_with",
            "endsWith" | "endswith" | "has_suffix" | "hasSuffix" => "ends_with",
            "collect" | "to_list" | "toList" | "iter" | "into_iter" => {
                return matches!(ty, Ty::List(_)).then(|| {
                    "a list is used as it is: `map`, `filter` and the rest take a list and return a list".to_string()
                });
            }
            "unwrap" | "expect" | "unwrap_or" | "get_or_insert" | "orElse" | "getOrElse" => {
                return Some("give the value to use when there is none with `else`: `x else fallback`".to_string());
            }
            "select" | "where" => "filter",
            "delete" | "erase" | "discard" => "remove",
            "put" | "set" => "insert",
            "str" | "toString" | "String" | "to_str" => "to_string",
            _ => return None,
        };
        has(koo).then(|| format!("in Koo that is `.{koo}`"))
    }
}

/// Help for an unknown variable.
pub(super) fn name_habit(wanted: &str) -> Option<String> {
    match wanted {
        "nil" | "null" | "None" | "NULL" | "nullptr" | "undefined" | "Nil" => {
            Some("Koo has no null: a value that may be absent has the type `T?`, and its absence is `none`".to_string())
        }
        "True" | "TRUE" => Some("write `true`".to_string()),
        "False" | "FALSE" => Some("write `false`".to_string()),
        "this" => Some("a method names its receiver `self`".to_string()),
        "io" | "os" | "test" | "http" => Some(format!(
            "this file does not import the package: add `import std/{wanted}` at the top of the file"
        )),
        _ => None,
    }
}

/// Help for an unknown function.
pub(super) fn function_habit(wanted: &str) -> Option<String> {
    match wanted {
        "println" | "printf" | "puts" | "echo" | "log" | "printLine" | "writeln" | "Println" => {
            Some("printing a line is `print(...)`; values go inside the string: `print(\"total: {n}\")`".to_string())
        }
        "len" | "length" | "size" => Some("the number of elements is a field: `x.len`".to_string()),
        "str" | "string" | "String" | "to_string" | "toString" => {
            Some("a value becomes text in a string, `\"{x}\"`, or with `x.to_string()`".to_string())
        }
        "int" | "float" | "Int" | "Float" => {
            Some("conversions are written `int.parse(text)`, `float.from(n)` and `int.from(x)`".to_string())
        }
        "range" => Some("a range is written `0..n`, or `0..=n` to include `n`".to_string()),
        "assert" | "assert_eq" => {
            Some("assertions are in `std/test`: `import std/test`, then `test.assert(...)`".to_string())
        }
        "exit" => Some("ending the program is in `std/os`: `import std/os`, then `os.exit(1)`".to_string()),
        _ => None,
    }
}

/// Help for an unknown type.
pub(super) fn type_habit(wanted: &str) -> Option<String> {
    let koo = match wanted {
        "String" | "str" | "Str" | "text" => "`string`",
        "Int" | "integer" | "Integer" | "long" | "isize" | "usize" | "number" | "Number" => "`int`",
        "Float" | "double" | "Double" | "real" => "`float`",
        "Bool" | "boolean" | "Boolean" => "`bool`",
        "Vec" | "Array" | "array" | "list" | "Slice" => "a list, written `[T]`",
        "HashMap" | "dict" | "Dict" | "map" | "BTreeMap" | "Dictionary" => "a map, written `[K: V]`",
        "HashSet" | "set" => "`Set<T>`",
        "Option" | "Optional" | "Maybe" => "an optional, written `T?`",
        "Result" => "a result, written `T ! E`",
        "void" | "Void" | "unit" | "Unit" => "no return type at all: leave it out",
        "any" | "object" | "Object" | "interface" => {
            return Some("Koo has no type that holds anything: name the type, or use a type parameter".to_string());
        }
        _ => return None,
    };
    Some(format!("in Koo that is {koo}"))
}

/// Whether some candidate is near enough to `wanted` to be what was meant.
pub(super) fn has_near(wanted: &str, candidates: &[String]) -> bool {
    closest(wanted, candidates).is_some()
}

/// The candidate nearest to `wanted`, if one is near enough to be what was meant.
fn closest<'c>(wanted: &str, candidates: &'c [String]) -> Option<&'c str> {
    let limit = match wanted.chars().count() {
        0..=2 => 0,
        3..=5 => 1,
        6..=9 => 2,
        _ => 3,
    };
    let lowered = wanted.to_lowercase();
    candidates
        .iter()
        .filter(|name| !name.starts_with('_') && !name.contains("__") && !name.contains(' ') && name.as_str() != wanted)
        .map(|name| {
            let distance = if name.to_lowercase() == lowered {
                0
            } else {
                edit_distance(wanted, name)
            };
            (distance, name)
        })
        .filter(|(distance, _)| *distance <= limit)
        .min_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)))
        .map(|(_, name)| name.as_str())
}

/// The number of single-character insertions, deletions, replacements and swaps of
/// neighbours that turn `a` into `b`.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut before: Vec<usize> = Vec::new();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut current = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let replace = previous[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            current[j] = replace.min(previous[j] + 1).min(current[j - 1] + 1);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                current[j] = current[j].min(before[j - 2] + 1);
            }
        }
        before = std::mem::replace(&mut previous, current);
    }
    previous[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances() {
        assert_eq!(edit_distance("lenght", "length"), 1);
        assert_eq!(edit_distance("frist", "first"), 1);
        assert_eq!(edit_distance("usre", "user"), 1);
        assert_eq!(edit_distance("abc", "abc"), 0);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
    }

    #[test]
    fn nearest_name_within_reach() {
        let names: Vec<String> = ["user", "users", "count", "_hidden", "io__read"]
            .map(String::from)
            .to_vec();
        assert_eq!(closest("usre", &names), Some("user"));
        assert_eq!(closest("Count", &names), Some("count"));
        assert_eq!(closest("zebra", &names), None);
        assert_eq!(closest("x", &names), None);
        assert_eq!(closest("hidden", &names), None);
    }
}
