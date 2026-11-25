//! What to say when the text is the syntax of another language: the mistake, named for
//! what it is, and the Koo that was meant.
//!
//! Each of these is reported once, and the rest of the statement or declaration is
//! skipped, so that one habit is one error.

use super::Parser;
use crate::ast::*;
use crate::diag::{Diagnostic, Span};
use crate::token::{Kw, Tok};

/// The Koo type that a type name of another language stands for.
pub(super) fn koo_type(name: &str) -> Option<&'static str> {
    Some(match name {
        "i8" | "i16" | "i32" | "i64" | "u16" | "u32" | "u64" | "isize" | "usize" | "long" | "short" | "integer"
        | "uint" | "int8" | "int16" | "int32" | "int64" | "uint32" | "uint64" | "Int" | "Integer" | "Long" => "int",
        "u8" | "uint8" | "Byte" => "byte",
        "f32" | "f64" | "double" | "real" | "number" | "float32" | "float64" | "Float" | "Double" | "Number" => "float",
        "str" | "text" | "String" | "Str" => "string",
        "boolean" | "Bool" | "Boolean" => "bool",
        "int" => "int",
        "float" => "float",
        "string" => "string",
        "bool" => "bool",
        "byte" => "byte",
        _ => return None,
    })
}

fn word_of(tok: &Tok) -> Option<&str> {
    match tok {
        Tok::Ident(word) | Tok::UpperIdent(word) => Some(word),
        _ => None,
    }
}

fn starts_value(tok: &Tok) -> bool {
    matches!(
        tok,
        Tok::Ident(_)
            | Tok::UpperIdent(_)
            | Tok::Int(_)
            | Tok::Float(_)
            | Tok::Str(_)
            | Tok::RawStr(_)
            | Tok::Char(_)
            | Tok::True
            | Tok::False
            | Tok::NoneLit
    )
}

impl Parser {
    /// A statement was read and its line goes on. Reports that, with the most exact
    /// advice there is, and returns whether the statement is worth keeping: a lone word
    /// that turned out to be the start of something else is not.
    pub(super) fn line_goes_on(&mut self, stmt: &Stmt) -> bool {
        let lone = matches!(
            &stmt.kind,
            StmtKind::Expr(Expr {
                kind: ExprKind::Name(_),
                ..
            })
        );
        if !self.foreign_statement(lone) {
            let _ = self.expected::<()>("the end of the line");
        }
        !lone
    }

    /// Reports the statement of another language that ends at the current token, if it
    /// is one. `lone` says whether what was read so far is a word alone.
    fn foreign_statement(&mut self, lone: bool) -> bool {
        let before = self.toks[self.pos - 1].clone();
        let next = self.toks[self.pos].clone();
        let after = self.toks[(self.pos + 1).min(self.toks.len() - 1)].clone();
        let gap = Span {
            start: before.span.end,
            end: next.span.start,
        };
        let through_next = Span {
            start: before.span.start,
            end: next.span.start,
        };

        // `} elif x {`, `} catch e {`.
        if before.kind == Tok::RBrace
            && let Some(word) = word_of(&next.kind)
        {
            match word {
                "elif" | "elsif" | "elseif" | "elsif_" => {
                    self.diag(
                        Diagnostic::new(next.span, format!("`{word}` is written `else if`")).with_fix(
                            "write `else if`",
                            next.span,
                            "else if",
                        ),
                    );
                    return true;
                }
                "catch" | "except" | "rescue" | "finally" | "ensure" => {
                    self.no_try(next.span, word);
                    return true;
                }
                _ => {}
            }
        }

        // `cond ? a : b`.
        if before.kind == Tok::Question && starts_value(&next.kind) {
            let last = self.line_last(self.pos);
            if (self.pos..=last).any(|i| self.toks[i].kind == Tok::Colon) {
                self.diag(
                    Diagnostic::new(before.span, "Koo has no `? :`")
                        .with_note("a `?` after a value passes its error on to the caller (Section 6.4)")
                        .with_help("an `if` has a value: write `if condition { a } else { b }`"),
                );
                return true;
            }
        }

        let Some(word) = word_of(&before.kind).map(str::to_string) else {
            return false;
        };
        let word = word.as_str();
        match word {
            // `let x = 5`, `let mut x = 5`, `const x = 5`, `let x: int = 5`.
            "let" | "const" | "val" | "auto" | "local" | "final" | "my" | "dim" | "mut" if lone => {
                let mut at = self.pos;
                let mutable = word == "mut" || self.toks[at].kind == Tok::Ident("mut".to_string());
                if word != "mut" && mutable {
                    at += 1;
                }
                let name = self.toks[at].clone();
                let mut diagnostic = Diagnostic::new(
                    before.span,
                    format!("`{word}` is not Koo: a variable is declared with `:=`"),
                )
                .with_note("`x := 5` declares a variable that does not change, and `var x := 5` one that does");
                if let Some(name_text) = word_of(&name.kind) {
                    let lead = Span {
                        start: before.span.start,
                        end: name.span.start,
                    };
                    let mut edits = vec![(lead, if mutable { "var ".to_string() } else { String::new() })];
                    let follows = &self.toks[(at + 1).min(self.toks.len() - 1)];
                    if follows.kind == Tok::Eq && !follows.nl_before {
                        edits.push((follows.span, ":=".to_string()));
                        diagnostic = diagnostic.with_edits(format!("declare `{name_text}` the Koo way"), edits);
                    } else if follows.kind == Tok::Colon && !follows.nl_before {
                        // `let x: int = 5` is `x: int = 5`.
                        diagnostic = diagnostic.with_edits(format!("declare `{name_text}` the Koo way"), edits);
                    }
                }
                self.diag(diagnostic);
                true
            }
            // `int x = 5`, `String name = "a"`.
            _ if lone
                && matches!(next.kind, Tok::Ident(_))
                && (koo_type(word).is_some() || before_is_upper(&before.kind)) =>
            {
                let name = word_of(&next.kind).unwrap_or("x").to_string();
                let ty = koo_type(word).unwrap_or(word).to_string();
                let both = Span {
                    start: before.span.start,
                    end: next.span.end,
                };
                let mut diagnostic = Diagnostic::new(both, "the type of a variable comes after its name").with_note(
                    format!("a binding is `{name} := value`, or `{name}: {ty} = value` when the type is to be stated"),
                );
                if after.kind == Tok::Eq && !after.nl_before {
                    diagnostic = diagnostic.with_fix("write the name first", both, format!("{name}: {ty}"));
                } else {
                    diagnostic =
                        diagnostic.with_help(format!("a binding is always given a value: `{name}: {ty} = ...`"));
                }
                self.diag(diagnostic);
                true
            }
            "throw" | "raise" if lone => {
                self.diag(
                    Diagnostic::new(
                        before.span,
                        format!("`{word}` is not Koo: an error is reported with `fail`"),
                    )
                    .with_note("and the function says what it may fail with: `fn f() -> T ! E` (Section 6.2)")
                    .with_fix("write `fail`", before.span, "fail"),
                );
                true
            }
            "try" if lone => {
                self.no_try(before.span, word);
                true
            }
            "new" if matches!(next.kind, Tok::UpperIdent(_)) => {
                self.diag(
                    Diagnostic::new(before.span, "Koo has no `new`")
                        .with_note("a value is made by naming its type and its fields: `Point { x: 1, y: 2 }`")
                        .with_fix("remove it", through_next, ""),
                );
                true
            }
            "await" if starts_value(&next.kind) => {
                self.diag(
                    Diagnostic::new(before.span, "Koo has no `await`")
                        .with_note("a call returns when its result is ready, whatever it waits for (Section 7.1)")
                        .with_fix("remove it", through_next, ""),
                );
                true
            }
            "f" | "F" if matches!(next.kind, Tok::Str(_)) && gap.start == gap.end => {
                self.diag(
                    Diagnostic::new(before.span, "a string needs no `f` to hold values")
                        .with_note("every string takes values in braces: `\"total: {n}\"`")
                        .with_fix("remove it", before.span, ""),
                );
                true
            }
            "switch" | "case" | "when" if lone => {
                self.diag(
                    Diagnostic::new(before.span, format!("`{word}` is not Koo: choosing among cases is `match`")).with_help(
                        "write `match value { 1 => ..., 2 | 3 => ..., _ => ... }`: one arm a line, and `_` for every other case (Section 6.6)",
                    ),
                );
                true
            }
            "loop" | "forever" if lone && next.kind == Tok::LBrace => {
                self.diag(
                    Diagnostic::new(
                        before.span,
                        format!("`{word}` is not Koo: a loop without an end is `while true`"),
                    )
                    .with_fix("write `while true`", before.span, "while true"),
                );
                true
            }
            "do" | "repeat" if lone && next.kind == Tok::LBrace => {
                self.diag(
                    Diagnostic::new(
                        before.span,
                        format!("`{word}` is not Koo: a loop tests its condition first"),
                    )
                    .with_help("write `while true { ... if done { break } }` to run the body before the first test"),
                );
                true
            }
            "unless" | "until" if lone => {
                let koo = if word == "unless" { "if not" } else { "while not" };
                self.diag(Diagnostic::new(before.span, format!("`{word}` is not Koo")).with_fix(
                    format!("write `{koo}`"),
                    before.span,
                    koo,
                ));
                true
            }
            "foreach" | "forEach" if lone => {
                self.diag(
                    Diagnostic::new(before.span, format!("`{word}` is not Koo: a loop over a list is `for`"))
                        .with_help(
                            "write `for item in items { ... }`, or `for (i, item) in items.enumerate() { ... }`",
                        ),
                );
                true
            }
            "del" | "delete" if lone => {
                self.diag(
                    Diagnostic::new(before.span, format!("`{word}` is not Koo")).with_help(
                        "a value is freed when nothing names it any more; to take something out of a collection, call its `remove`",
                    ),
                );
                true
            }
            "go" | "async" if lone => {
                self.diag(
                    Diagnostic::new(
                        before.span,
                        format!("`{word}` is not Koo: a task is started with `spawn`"),
                    )
                    .with_help("write `spawn { ... }` inside a `scope { ... }` (Section 7.6)"),
                );
                true
            }
            // `print "hello"`: a call without its parentheses.
            _ if lone
                && before_is_lower(&before.kind)
                && matches!(next.kind, Tok::Str(_) | Tok::Int(_) | Tok::Float(_)) =>
            {
                let end = self.toks[self.line_last(self.pos)].span.end;
                let close = Span { start: end, end };
                self.diag(
                    Diagnostic::new(
                        next.span,
                        format!("a call of `{word}` puts its arguments in parentheses"),
                    )
                    .with_edits(
                        "add the parentheses",
                        vec![(gap, "(".to_string()), (close, ")".to_string())],
                    ),
                );
                true
            }
            _ => false,
        }
    }

    fn no_try(&mut self, span: Span, word: &str) {
        self.diag(
            Diagnostic::new(
                span,
                format!("Koo has no `{word}`: an error is a value, handled where the call is made"),
            )
            .with_help("`value := risky()?` passes the error on to the caller")
            .with_help("`value := risky() else fallback` puts a value in its place")
            .with_help("`match risky() { value => ..., ! e => ... }` looks at it (Section 6.4)"),
        );
    }

    /// A word alone on its line that is no statement: what it stands for elsewhere.
    pub(super) fn lone_word(&mut self, name: &Ident) -> bool {
        let diagnostic = match name.name.as_str() {
            "end" | "endif" | "fi" | "done" | "endfor" | "endwhile" | "endfunction" | "esac" => Diagnostic::new(
                name.span,
                format!("`{}` is not Koo: a block ends with `}}`", name.name),
            )
            .with_fix("write `}`", name.span, "}"),
            "pass" | "nop" | "noop" => Diagnostic::new(name.span, format!("`{}` is not Koo", name.name))
                .with_note("a block may be empty: `{}`")
                .with_fix("remove it", name.span, ""),
            "begin" | "then" => Diagnostic::new(
                name.span,
                format!("`{}` is not Koo: a block begins with `{{`", name.name),
            ),
            "exit" | "quit" | "abort" => Diagnostic::new(name.span, format!("`{}` is not a statement", name.name))
                .with_help(
                    "to end the program, `import std/os` and call `os.exit(1)`; to end it for a bug, `panic(\"why\")`",
                ),
            _ => return false,
        };
        self.diag(diagnostic);
        true
    }

    /// Reports a declaration written in the syntax of another language, if the current
    /// token begins one.
    pub(super) fn foreign_item(&mut self) -> bool {
        let token = self.toks[self.pos].clone();
        let next = self.toks[(self.pos + 1).min(self.toks.len() - 1)].clone();
        let same_line = !next.nl_before && next.kind != Tok::Eof;
        let line = Span {
            start: token.span.start,
            end: self.toks[self.line_last(self.pos)].span.end,
        };
        let diagnostic = match &token.kind {
            Tok::RBrace => Diagnostic::new(token.span, "this `}` closes nothing")
                .with_help("remove it, or look above for the `{` it was meant to close"),
            Tok::RParen | Tok::RBracket => {
                Diagnostic::new(token.span, format!("this {} closes nothing", token.kind.describe()))
            }
            Tok::Kw(Kw::If | Kw::For | Kw::While | Kw::Match | Kw::Return) => self.outside_a_function(token.span),
            Tok::Ident(word) | Tok::UpperIdent(word) => match word.as_str() {
                "function" | "func" | "def" | "fun" | "sub" | "proc" | "method" | "defn" => {
                    Diagnostic::new(token.span, format!("`{word}` is not Koo: a function is declared with `fn`"))
                        .with_note("`fn add(a: int, b: int) -> int { return a + b }`: each parameter has a type, the result follows `->`, and the body is in braces")
                        .with_fix("write `fn`", token.span, "fn")
                }
                "struct" | "class" | "record" | "data" | "object" | "typedef" => {
                    let name = match &next.kind {
                        Tok::UpperIdent(name) if same_line => name.clone(),
                        _ => "Point".to_string(),
                    };
                    Diagnostic::new(token.span, format!("`{word}` is not Koo: a type with fields is declared with `type`"))
                        .with_help(format!("write `type {name} = {{ x: int, y: int }}`, with a comma or a new line between fields"))
                        .with_help(format!("a method is a function declared on it: `fn (self: {name}) area() -> int {{ ... }}` (Section 4.6)"))
                }
                "const" | "let" | "val" | "static" | "final" | "define" | "global" => {
                    let name = match &next.kind {
                        Tok::Ident(name) | Tok::UpperIdent(name) if same_line => name.to_lowercase(),
                        _ => "limit".to_string(),
                    };
                    let diagnostic =
                        Diagnostic::new(token.span, format!("`{word}` is not Koo: a constant is declared by its name alone"))
                            .with_note("a name at the top level is a constant, written in snake_case (Section 3.5)");
                    // `const LIMIT = 5` is `limit := 5`.
                    let eq = self.toks[(self.pos + 2).min(self.toks.len() - 1)].clone();
                    if same_line && eq.kind == Tok::Eq && !eq.nl_before && matches!(next.kind, Tok::Ident(_) | Tok::UpperIdent(_)) {
                        let lead = Span {
                            start: token.span.start,
                            end: eq.span.end,
                        };
                        diagnostic.with_fix("declare it the Koo way", lead, format!("{name} :="))
                    } else {
                        diagnostic.with_help(format!("write `{name} := ...`"))
                    }
                }
                "package" | "module" | "namespace" | "mod" => Diagnostic::new(
                    token.span,
                    format!("`{word}` is not Koo: a file does not name its package"),
                )
                .with_note("the directory a file is in is its package (Section 8.1)")
                .with_fix("remove the line", line, ""),
                "use" | "using" | "require" | "include" | "from" | "extern" => {
                    Diagnostic::new(token.span, format!("`{word}` is not Koo: a package is brought in with `import`"))
                        .with_help("write `import std/io`, and then name what it holds through it: `io.read_file(path)` (Section 8.2)")
                }
                "public" | "export" => Diagnostic::new(token.span, format!("`{word}` is not Koo: what a package offers is marked `pub`"))
                    .with_fix("write `pub`", token.span, "pub"),
                "private" | "protected" | "internal" | "local" => Diagnostic::new(token.span, format!("`{word}` is not Koo"))
                    .with_note("a declaration is private to its package unless it is marked `pub`")
                    .with_fix("remove it", Span { start: token.span.start, end: next.span.start }, ""),
                "impl" | "extend" | "extension" => {
                    Diagnostic::new(token.span, format!("`{word}` is not Koo: a method is declared on its own"))
                        .with_help("write `fn (self: Point) length() -> float { ... }`: the receiver comes before the name (Section 4.6)")
                }
                "trait" | "protocol" | "abstract" => Diagnostic::new(
                    token.span,
                    format!("`{word}` is not Koo: a set of methods a type must have is an `interface`"),
                )
                .with_help("write `interface Shape { fn area() -> float }`; a type satisfies it by having the methods (Section 4.9)"),
                _ if next.kind == Tok::Eq && same_line && matches!(token.kind, Tok::Ident(_)) => {
                    Diagnostic::new(next.span, "a constant is declared with `:=`")
                        .with_note("a name at the top level is a constant; state that changes belongs in a function (Section 3.5)")
                        .with_fix("write `:=`", next.span, ":=")
                }
                _ if matches!(next.kind, Tok::LParen | Tok::Dot) && same_line => self.outside_a_function(token.span),
                _ => return false,
            },
            _ => return false,
        };
        // What the lexer made of the rest of the line is beside the point.
        self.claimed.push(line);
        self.diag(diagnostic);
        true
    }

    fn outside_a_function(&self, span: Span) -> Diagnostic {
        Diagnostic::new(span, "a statement cannot stand outside a function")
            .with_note("a file holds declarations: types, constants and functions")
            .with_help("the program starts in `main`: put the statements in `fn main() { ... }`")
    }

    /// `for (i = 0; i < n; i++) {`: reports the loop of C, with the range it counts over
    /// if that can be read off. Returns whether it was one.
    pub(super) fn foreign_for(&mut self) -> bool {
        if self.toks[self.pos].kind != Tok::LParen {
            return false;
        }
        let Some(close) = self.matching_paren() else {
            return false;
        };
        let inside = &self.toks[self.pos + 1..close];
        if !inside.iter().any(|token| token.kind == Tok::Semi) {
            return false;
        }
        let whole = Span {
            start: self.toks[self.pos].span.start,
            end: self.toks[close].span.end,
        };
        // `[let|var|int] i = A ; i < B ; ...`
        let kinds: Vec<&Tok> = inside.iter().map(|token| &token.kind).collect();
        let skip = match kinds.first() {
            Some(Tok::Kw(Kw::Var)) => 1,
            Some(Tok::Ident(word)) if matches!(kinds.get(1), Some(Tok::Ident(_))) && word != "i" => 1,
            _ => 0,
        };
        let simple = |tok: &Tok| match tok {
            Tok::Int(text) => Some(text.clone()),
            Tok::Ident(text) => Some(text.clone()),
            _ => None,
        };
        let range = match &kinds[skip.min(kinds.len())..] {
            [
                Tok::Ident(i),
                Tok::Eq | Tok::ColonEq,
                from,
                Tok::Semi,
                Tok::Ident(j),
                op @ (Tok::Lt | Tok::Le),
                to,
                Tok::Semi,
                ..,
            ] if i == j => match (simple(from), simple(to)) {
                (Some(from), Some(to)) => {
                    let dots = if **op == Tok::Lt { ".." } else { "..=" };
                    Some(format!("{i} in {from}{dots}{to}"))
                }
                _ => None,
            },
            _ => None,
        };
        let mut diagnostic = Diagnostic::new(whole, "this is the `for` of another language").with_note(
            "a `for` loop goes over a range or a list: `for i in 0..n { ... }`, `for item in items { ... }`",
        );
        diagnostic = match range {
            Some(range) => diagnostic.with_fix("count with a range", whole, range),
            None => diagnostic.with_help("for a loop that is not a count, write `while condition { ... }`"),
        };
        self.claimed.push(whole);
        self.diag(diagnostic);
        true
    }
}

fn before_is_upper(tok: &Tok) -> bool {
    matches!(tok, Tok::UpperIdent(_))
}

fn before_is_lower(tok: &Tok) -> bool {
    matches!(tok, Tok::Ident(_))
}
