//! The parser: hand-written recursive descent over the token list, with no type information
//! (Section 3 of the specification; `spec/grammar.ebnf` is the grammar it implements).
//!
//! Two things are decided here and not in the lexer:
//!
//! * **Which newlines end statements** (Section 3.7). The parser keeps a stack that says, for
//!   each open bracket, whether it is a block brace. `peek` reports `Tok::Newline` in place
//!   of a token that begins a line when the innermost bracket is a block brace, or none is
//!   open, and the token does not continue the line before it.
//! * **Which braces are literal braces** (Section 3.9). `struct_lit_ok` is false while a
//!   condition, a `match` scrutinee or a `for` subject is being parsed.

mod exprs;
mod habits;
mod items;
mod patterns;
mod stmts;
mod types;

use std::collections::HashMap;

use crate::ast::*;
use crate::diag::{Diagnostic, LineIndex, Span};
use crate::lexer::lex_at;
use crate::token::{Kw, Tok, Token, is_primitive};

pub(crate) type PResult<T> = Result<T, ()>;

static NEWLINE: Tok = Tok::Newline;

/// How deep a syntax tree may be. One level is one bracket, block or operand: `((x))` is
/// two levels deep and `a + b + c` is two. The parser and everything that walks the tree
/// are recursive, so the limit is what keeps hostile input from exhausting the stack.
const MAX_NESTING: u32 = 512;

/// The result of parsing: a tree, and every error found. The tree is complete when
/// `diagnostics` is empty; otherwise it holds what could be parsed.
#[derive(Debug)]
pub struct Parsed {
    pub file: File,
    pub diagnostics: Vec<Diagnostic>,
}

/// Parses a complete source file: imports, then declarations.
pub fn parse_file(src: &str) -> Parsed {
    parse(src, false, 0)
}

/// Parses a source file that begins at offset `base` of its package (see `SourceMap`):
/// every span in the result counts from the start of the package.
pub fn parse_file_at(src: &str, base: u32) -> Parsed {
    parse(src, false, base)
}

/// Parses a fragment, as found in documentation: declarations and statements in any order,
/// where a function may be a signature without a body.
pub fn parse_fragment(src: &str) -> Parsed {
    parse(src, true, 0)
}

/// The stack the parser runs on. `MAX_NESTING` levels of the deepest recursion fit in it
/// with room to spare, in unoptimized builds too.
const PARSER_STACK: usize = 64 * 1024 * 1024;

fn parse(src: &str, fragment: bool, base: u32) -> Parsed {
    // The parser is recursive, and the thread it is called on may have a small stack, so
    // it runs on a thread of its own. Where a thread cannot be started it runs in place.
    std::thread::scope(|scope| {
        let spawned = std::thread::Builder::new()
            .name("koo-parser".to_string())
            .stack_size(PARSER_STACK)
            .spawn_scoped(scope, || parse_in_place(src, fragment, base));
        match spawned {
            Ok(handle) => handle.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
            Err(_) => parse_in_place(src, fragment, base),
        }
    })
}

fn parse_in_place(src: &str, fragment: bool, base: u32) -> Parsed {
    let lexed = lex_at(src, base);
    let index = LineIndex::at(src, base);
    let docs = lexed
        .docs
        .iter()
        .map(|d| (index.line(d.offset), d.text.clone()))
        .collect();
    let mut parser = Parser::new(lexed.tokens, fragment);
    parser.docs = docs;
    parser.index = index;
    let file = parser.file();
    // What the parser said about a stretch of text as a whole (a `for` loop written as
    // in C) stands for what the lexer said about its parts (`i++`).
    let claimed = std::mem::take(&mut parser.claimed);
    let mut diagnostics = lexed.diagnostics;
    diagnostics.retain(|d| {
        !claimed
            .iter()
            .any(|region| region.start <= d.span.start && d.span.end <= region.end)
    });
    // And what the lexer said stands for what the parser then made of the same text:
    // of a token the lexer had to guess at, and of a line whose string never ends.
    let line_of = |offset: u32| parser.index.line(offset);
    let unclosed: Vec<u32> = diagnostics
        .iter()
        .filter(|d| d.message.contains("is not closed"))
        .map(|d| line_of(d.span.start))
        .collect();
    let guessed: Vec<Span> = diagnostics.iter().map(|d| d.span).collect();
    parser.diags.retain(|d| {
        let on_unclosed_line = std::iter::once(d.span)
            .chain(d.related.iter().map(|(span, _)| *span))
            .any(|span| unclosed.contains(&line_of(span.start)));
        let at_guess = guessed
            .iter()
            .any(|span| span.start <= d.span.start && d.span.start < span.end);
        !on_unclosed_line && !at_guess
    });
    diagnostics.append(&mut parser.diags);
    diagnostics.sort_by_key(|d| d.span.start);
    Parsed { file, diagnostics }
}

/// What `enter` saves and `leave` restores.
#[derive(Clone, Copy)]
struct Saved {
    struct_lit_ok: bool,
    else_bind_ok: bool,
}

/// `myValue` as `my_value`.
fn snake_case(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 && !out.ends_with('_') {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// `order_line` as `OrderLine`.
fn camel_case(name: &str) -> String {
    name.split('_')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

/// `what` with its indefinite article.
fn article(what: &str) -> String {
    let an = what.starts_with(['a', 'e', 'i', 'o', 'u']);
    format!("{} {what}", if an { "an" } else { "a" })
}

pub(crate) struct Parser {
    toks: Vec<Token>,
    pos: usize,
    /// One entry per open bracket: whether newlines are significant inside it.
    nl_sig: Vec<bool>,
    /// One entry per open bracket: the bracket.
    openers: Vec<Tok>,
    /// Set when a block is left because a declaration begins at the left margin inside
    /// it: where that declaration is. The block was never closed.
    unclosed_before: Option<Span>,
    /// Whether the line break before the current token has been consumed.
    nl_skipped: bool,
    /// Whether `Name {` may begin a struct literal here.
    struct_lit_ok: bool,
    /// Whether `else name =>` binds the error here. It does not in a `match` guard or a
    /// `select` operand, where `=>` ends the expression (Section 3.9).
    else_bind_ok: bool,
    /// Whether signatures without bodies and top-level statements are accepted.
    fragment: bool,
    /// The names bound by `import c` declarations in this file (Section 11.1).
    c_prefixes: Vec<String>,
    /// Documentation comments by line.
    docs: HashMap<u32, String>,
    index: LineIndex,
    /// The current depth of `nested` calls.
    nesting: u32,
    diags: Vec<Diagnostic>,
    /// Stretches of text reported as a whole: what the lexer reported inside is dropped.
    claimed: Vec<Span>,
    /// Blocks whose `}` is not indented as the line of their `{` is: the `{` and the `}`.
    /// When a `{` is never closed, the last of these is the likely place of the missing `}`.
    misaligned: Vec<(Span, Span)>,
}

impl Parser {
    fn new(toks: Vec<Token>, fragment: bool) -> Parser {
        Parser {
            toks,
            pos: 0,
            nl_sig: Vec::new(),
            openers: Vec::new(),
            unclosed_before: None,
            nl_skipped: false,
            struct_lit_ok: true,
            else_bind_ok: true,
            fragment,
            // A fragment has no imports of its own; in one, `c` is the prefix of C names.
            c_prefixes: if fragment { vec!["c".to_string()] } else { Vec::new() },
            docs: HashMap::new(),
            index: LineIndex::new(""),
            nesting: 0,
            diags: Vec::new(),
            claimed: Vec::new(),
            misaligned: Vec::new(),
        }
    }

    // ---- tokens -------------------------------------------------------------------------

    fn significant(&self) -> bool {
        self.nl_sig.last().copied().unwrap_or(true)
    }

    /// Whether a significant line break precedes the token at `i`.
    fn breaks_at(&self, i: usize) -> bool {
        i > 0 && self.toks[i].nl_before && self.significant() && !self.toks[i].kind.continues_line()
    }

    fn at_newline(&self) -> bool {
        !self.nl_skipped && self.breaks_at(self.pos)
    }

    /// The current token, or `Tok::Newline` if a significant line break comes first.
    fn peek(&self) -> &Tok {
        if self.at_newline() {
            &NEWLINE
        } else {
            &self.toks[self.pos].kind
        }
    }

    /// The token after the current one, or `Tok::Newline` if a significant line break
    /// separates the two.
    fn peek2(&self) -> &Tok {
        let i = (self.pos + 1).min(self.toks.len() - 1);
        if self.breaks_at(i) {
            &NEWLINE
        } else {
            &self.toks[i].kind
        }
    }

    /// The token `n` places ahead, ignoring line breaks.
    fn raw(&self, n: usize) -> &Tok {
        &self.toks[(self.pos + n).min(self.toks.len() - 1)].kind
    }

    fn span(&self) -> Span {
        self.toks[self.pos].span
    }

    /// The span of the token consumed last.
    fn prev_span(&self) -> Span {
        self.toks[self.pos.saturating_sub(1)].span
    }

    fn bump(&mut self) -> Token {
        let token = self.toks[self.pos].clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        self.nl_skipped = false;
        token
    }

    fn at(&self, tok: &Tok) -> bool {
        self.peek() == tok
    }

    fn at_kw(&self, kw: Kw) -> bool {
        *self.peek() == Tok::Kw(kw)
    }

    fn eat(&mut self, tok: &Tok) -> bool {
        if self.at(tok) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat_kw(&mut self, kw: Kw) -> bool {
        self.eat(&Tok::Kw(kw))
    }

    fn skip_newlines(&mut self) {
        if self.at_newline() {
            self.nl_skipped = true;
        }
    }

    /// Whether the statement or declaration being parsed may end here.
    fn at_stmt_end(&self) -> bool {
        matches!(self.peek(), Tok::Newline | Tok::RBrace | Tok::Eof)
    }

    // ---- errors -------------------------------------------------------------------------

    fn report(&mut self, span: Span, message: impl Into<String>) {
        self.diags.push(Diagnostic::new(span, message));
    }

    fn report_help(&mut self, span: Span, message: impl Into<String>, help: impl Into<String>) {
        self.diags.push(Diagnostic::new(span, message).with_help(help));
    }

    fn diag(&mut self, diagnostic: Diagnostic) {
        self.diags.push(diagnostic);
    }

    /// The column at which `offset` is: the number of bytes from the start of its line.
    fn column(&self, offset: u32) -> u32 {
        offset - self.index.line_start(self.index.line(offset))
    }

    /// The first token on the line of the token at `i`.
    fn line_first(&self, i: usize) -> usize {
        let mut first = i;
        while first > 0 && !self.toks[first].nl_before {
            first -= 1;
        }
        first
    }

    /// The last token on the line of the token at `i`.
    fn line_last(&self, i: usize) -> usize {
        let mut last = i;
        while last + 1 < self.toks.len() && !self.toks[last + 1].nl_before && self.toks[last + 1].kind != Tok::Eof {
            last += 1;
        }
        last
    }

    fn fail<T>(&mut self, span: Span, message: impl Into<String>) -> PResult<T> {
        self.report(span, message);
        Err(())
    }

    /// The span at which a missing token is reported.
    fn here(&self) -> Span {
        if self.at_newline() || self.at(&Tok::Eof) {
            let end = self.prev_span().end;
            Span { start: end, end }
        } else {
            self.span()
        }
    }

    /// Reports "expected X, found Y" at the current token.
    fn expected<T>(&mut self, what: &str) -> PResult<T> {
        let found = self.peek().describe();
        let span = self.here();
        let message = format!("expected {what}, found {found}");
        let mut diagnostic = Diagnostic::new(span, message);
        let block = what.starts_with("`{`");
        diagnostic = match self.peek() {
            Tok::Semi => {
                let note = "Koo has no semicolons: a statement ends at the end of its line";
                if self.line_last(self.pos) == self.pos {
                    diagnostic.with_note(note).with_fix("remove it", span, "")
                } else {
                    diagnostic.with_note(note).with_help("write each statement on a line of its own")
                }
            }
            Tok::Eq if block => diagnostic
                .with_note("`=` assigns; a condition compares")
                .with_help("to bind an optional's value, write `if x := value { ... }`")
                .with_fix("compare with `==`", span, "=="),
            Tok::Colon if block => diagnostic
                .with_help("a block is written in braces, not after a colon: `if x > 0 { ... }`"),
            Tok::Ident(word) if block && matches!(word.as_str(), "then" | "do" | "begin") => {
                let help = format!("a block is written in braces: write `{{` in place of `{word}`, and `}}` where it ends");
                diagnostic.with_help(help)
            }
            Tok::Newline | Tok::Eof if block => diagnostic.with_help("the `{` of a block goes on the same line as what it belongs to"),
            _ if what == "`{`" && !self.at_newline() => {
                diagnostic.with_help("a block is written in braces, even when it holds one statement")
            }
            Tok::FatArrow if what.contains("`)`") => diagnostic
                .with_note("`=>` belongs to the arms of a `match`")
                .with_fix("a lambda is written with `->`", span, "->"),
            Tok::Pipe | Tok::Kw(Kw::Fn) if what == "an expression" => {
                diagnostic.with_help("a lambda is written with an arrow: `x -> x + 1`, or `(a, b) -> { ... }` for a body of several statements")
            }
            Tok::Newline | Tok::Eof if what == "an expression" && self.pos > 0 => {
                let before = self.toks[self.pos - 1].kind.describe();
                diagnostic.with_label(format!("the line ends after {before}, which needs a value after it"))
            }
            Tok::Kw(word) if what == "an expression" || what.contains("name") => {
                let word = word.as_str();
                diagnostic.with_note(format!("`{word}` is a word of the language, so it cannot be used as a name"))
            }
            Tok::RParen | Tok::RBracket if what == "the end of the line" => {
                diagnostic.message = format!("this {found} closes nothing");
                diagnostic.with_fix("remove it", span, "")
            }
            Tok::Int(_) | Tok::Float(_) if what == "a member name" => diagnostic.with_help(
                "the elements of a tuple have no names or numbers: take it apart with a pattern, as in `(first, second) := pair` (Section 4.5)",
            ),
            Tok::NoneLit if what.contains("name") => diagnostic
                .with_help("`none` is the value of an optional that holds nothing, so it cannot name something else"),
            _ => diagnostic,
        };
        self.diag(diagnostic);
        Err(())
    }

    fn expect(&mut self, tok: &Tok, what: &str) -> PResult<Token> {
        if self.at(tok) {
            Ok(self.bump())
        } else {
            self.expected(what)
        }
    }

    fn expect_kw(&mut self, kw: Kw) -> PResult<Token> {
        if self.at_kw(kw) {
            Ok(self.bump())
        } else {
            self.expected(&format!("`{}`", kw.as_str()))
        }
    }

    // ---- brackets -----------------------------------------------------------------------

    /// Records an opened bracket. `significant` says whether it is a block brace.
    /// Returns the value to hand back to `leave`.
    fn enter(&mut self, significant: bool) -> Saved {
        self.nl_sig.push(significant);
        self.openers.push(self.toks[self.pos.saturating_sub(1)].kind.clone());
        Saved {
            struct_lit_ok: std::mem::replace(&mut self.struct_lit_ok, true),
            else_bind_ok: std::mem::replace(&mut self.else_bind_ok, true),
        }
    }

    /// Call after the closing bracket has been consumed.
    fn leave(&mut self, saved: Saved) {
        self.nl_sig.pop();
        self.openers.pop();
        self.struct_lit_ok = saved.struct_lit_ok;
        self.else_bind_ok = saved.else_bind_ok;
    }

    /// Parses `open item, item, ... close`, where newlines are white space and a trailing
    /// comma is allowed.
    fn delimited<T>(
        &mut self,
        open: &Tok,
        close: &Tok,
        mut item: impl FnMut(&mut Parser) -> PResult<T>,
    ) -> PResult<Vec<T>> {
        let opened = self.pos;
        self.expect(open, &open.describe())?;
        let saved = self.enter(false);
        let mut items = Vec::new();
        while !self.at(close) {
            items.push(item(self)?);
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        if !self.at(close) {
            let reported = self.diags.len();
            self.expected_separator(close);
            // If no bracket closes this one, that is the mistake, and where it is.
            if self.closer(opened).is_none()
                && let Some(diagnostic) = self.diags.get_mut(reported)
            {
                let text = format!("this {} is never closed", open.describe());
                diagnostic.related.push((self.toks[opened].span, text));
            }
            return Err(());
        }
        self.bump();
        self.leave(saved);
        Ok(items)
    }

    /// Reports that an item of a list is followed by neither a comma nor the bracket that
    /// ends the list.
    fn expected_separator(&mut self, close: &Tok) {
        let _ = self.expected::<()>(&format!("`,` or {}", close.describe()));
        let value = matches!(
            self.peek(),
            Tok::Ident(_) | Tok::UpperIdent(_) | Tok::Int(_) | Tok::Float(_) | Tok::Str(_) | Tok::True | Tok::False
        );
        let end = self.prev_span().end;
        if let Some(diagnostic) = self.diags.last_mut() {
            if value && !self.toks[self.pos].nl_before {
                // `[1 2 3]`: two values with nothing between them.
                diagnostic.fix = Some(crate::diag::Fix {
                    message: "a comma is missing".to_string(),
                    edits: vec![(Span { start: end, end }, ",".to_string())],
                });
            } else if self.toks[self.pos].kind == Tok::Eq && *close == Tok::RBrace {
                // `Point { x = 1 }`.
                let span = self.toks[self.pos].span;
                diagnostic
                    .notes
                    .push("a field is given its value with `:`, as it is given its type".to_string());
                diagnostic.fix = Some(crate::diag::Fix {
                    message: "write `:`".to_string(),
                    edits: vec![(
                        Span {
                            start: end,
                            end: span.end,
                        },
                        ":".to_string(),
                    )],
                });
            }
        }
    }

    /// The index of the bracket that closes the one at `open`, if one does.
    fn closer(&self, open: usize) -> Option<usize> {
        let mut depth = 0usize;
        for (i, token) in self.toks.iter().enumerate().skip(open) {
            match token.kind {
                Tok::LParen | Tok::LBracket | Tok::LBrace => depth += 1,
                Tok::RParen | Tok::RBracket | Tok::RBrace => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        let pair = matches!(
                            (&self.toks[open].kind, &token.kind),
                            (Tok::LParen, Tok::RParen) | (Tok::LBracket, Tok::RBracket) | (Tok::LBrace, Tok::RBrace)
                        );
                        return pair.then_some(i);
                    }
                }
                Tok::Eof => return None,
                _ => {}
            }
        }
        None
    }

    /// The index of the `)` that matches the `(` at the current position.
    fn matching_paren(&self) -> Option<usize> {
        self.closer(self.pos)
    }

    /// Whether the current token is a `(` whose matching `)` is followed by `tok`.
    fn paren_then(&self, tok: &Tok) -> bool {
        self.toks[self.pos].kind == Tok::LParen
            && self
                .matching_paren()
                .is_some_and(|i| i + 1 < self.toks.len() && self.toks[i + 1].kind == *tok && !self.breaks_at(i + 1))
    }

    /// Counts one more level of tree depth, refusing input deeper than `MAX_NESTING`.
    fn link(&mut self) -> PResult<()> {
        if self.nesting >= MAX_NESTING {
            let span = self.span();
            // Report once, however deep the input goes on.
            let reported = self
                .diags
                .iter()
                .any(|d| d.message.starts_with("this is nested too deeply"));
            if !reported {
                self.report_help(
                    span,
                    format!("this is nested too deeply: the limit is {MAX_NESTING} levels"),
                    "give part of it a name",
                );
            }
            return Err(());
        }
        self.nesting += 1;
        Ok(())
    }

    /// Runs a parsing function that recurses, one level deeper.
    fn nested<T>(&mut self, parse: impl FnOnce(&mut Parser) -> PResult<T>) -> PResult<T> {
        let base = self.nesting;
        let result = self.link().and_then(|()| parse(self));
        self.nesting = base;
        result
    }

    /// Runs a parsing function that builds a left-deep chain (`a + b + c`, `x.f.g`), calling
    /// `link` for each operand it adds. The depth is restored when the chain is complete.
    fn chain<T>(&mut self, parse: impl FnOnce(&mut Parser) -> PResult<T>) -> PResult<T> {
        let base = self.nesting;
        let result = parse(self);
        self.nesting = base;
        result
    }

    // ---- recovery -----------------------------------------------------------------------

    /// After an error, restores the bracket state of the enclosing block and skips to the
    /// end of the statement.
    fn recover(&mut self, depth: usize, struct_lit_ok: bool) {
        // The brackets above `depth` were open when the error was found.
        self.nl_sig.truncate(depth);
        let mut open: Vec<Tok> = self.openers.split_off(depth.min(self.openers.len()));
        self.struct_lit_ok = struct_lit_ok;
        self.else_bind_ok = true;
        loop {
            if open.is_empty() && self.at_newline() {
                return;
            }
            let closes = match self.toks[self.pos].kind {
                Tok::Eof => return,
                Tok::LParen | Tok::LBracket | Tok::LBrace => {
                    open.push(self.toks[self.pos].kind.clone());
                    None
                }
                Tok::RParen => Some(Tok::LParen),
                Tok::RBracket => Some(Tok::LBracket),
                Tok::RBrace => Some(Tok::LBrace),
                _ => None,
            };
            if let Some(opener) = closes {
                // A closing bracket closes the nearest bracket of its kind, and with it
                // whatever was left open inside. One that closes nothing here belongs
                // to the enclosing block: an unclosed `(` must not swallow its `}`.
                match open.iter().rposition(|open| *open == opener) {
                    Some(at) => open.truncate(at),
                    None if open.is_empty() || opener == Tok::LBrace => return,
                    None => {}
                }
            }
            self.bump();
        }
    }

    /// After an error in a declaration, skips to the next line that begins one: a word
    /// at the left margin.
    fn recover_item(&mut self, depth: usize) {
        self.nl_sig.truncate(depth);
        self.openers.truncate(depth);
        self.struct_lit_ok = true;
        self.else_bind_ok = true;
        loop {
            let token = &self.toks[self.pos];
            let word = matches!(token.kind, Tok::Ident(_) | Tok::UpperIdent(_) | Tok::Kw(_) | Tok::At);
            if token.kind == Tok::Eof || (token.nl_before && word && self.column(token.span.start) == 0) {
                return;
            }
            self.bump();
        }
    }

    // ---- names --------------------------------------------------------------------------

    fn ident_from(token: Token) -> Ident {
        let name = match token.kind {
            Tok::Ident(s) | Tok::UpperIdent(s) => s,
            Tok::Kw(k) => k.as_str().to_string(),
            Tok::True => "true".to_string(),
            Tok::False => "false".to_string(),
            Tok::NoneLit => "none".to_string(),
            other => unreachable!("not a word: {other:?}"),
        };
        Ident { name, span: token.span }
    }

    /// A snake_case name: a function, variable, field, package, constant or label.
    fn lower_name(&mut self, what: &str) -> PResult<Ident> {
        let a = article(what);
        match self.peek() {
            Tok::Ident(_) => {
                let id = Self::ident_from(self.bump());
                if id.name.bytes().any(|b| b.is_ascii_uppercase()) {
                    let snake = snake_case(&id.name);
                    self.diag(
                        Diagnostic::new(id.span, format!("`{}` is not snake_case", id.name))
                            .with_note(format!("{a} is written in snake_case (Section 3.1)"))
                            .with_fix(format!("write `{snake}`, here and where it is used"), id.span, snake),
                    );
                }
                Ok(id)
            }
            Tok::UpperIdent(_) => {
                let id = Self::ident_from(self.bump());
                self.report_help(
                    id.span,
                    format!("expected {a}, found the capitalized name `{}`", id.name),
                    format!("{a} is written in snake_case; capitalized names are types and variants (Section 3.1)"),
                );
                Ok(id)
            }
            _ => self.expected(&a),
        }
    }

    /// A name that is in scope by itself: a binding, a parameter, an import alias.
    /// It may not be a primitive type name (Section 3.1).
    fn binding_name(&mut self, what: &str) -> PResult<Ident> {
        let id = self.lower_name(what)?;
        if is_primitive(&id.name) {
            self.report_help(
                id.span,
                format!(
                    "`{}` is a primitive type and cannot be used as {}",
                    id.name,
                    article(what)
                ),
                "the primitive type names are predeclared (Section 3.1)",
            );
        }
        Ok(id)
    }

    /// An UpperCamelCase name: a type, variant, actor, interface, error or type parameter.
    fn upper_name(&mut self, what: &str) -> PResult<Ident> {
        let a = article(what);
        match self.peek() {
            Tok::UpperIdent(_) => {
                let id = Self::ident_from(self.bump());
                if id.name.contains('_') {
                    self.report_help(
                        id.span,
                        format!("`{}` is not UpperCamelCase", id.name),
                        format!("{a} is written in UpperCamelCase, without underscores (Section 3.1)"),
                    );
                }
                Ok(id)
            }
            Tok::Ident(_) => {
                let id = Self::ident_from(self.bump());
                let camel = camel_case(&id.name);
                self.diag(
                    Diagnostic::new(id.span, format!("expected {a}, found the lowercase name `{}`", id.name))
                        .with_note(format!("{a} is written in UpperCamelCase (Section 3.1)"))
                        .with_fix(format!("write `{camel}`, here and where it is used"), id.span, camel),
                );
                Ok(id)
            }
            _ => self.expected(&a),
        }
    }

    fn at_word(&self) -> bool {
        matches!(
            self.peek(),
            Tok::Ident(_) | Tok::UpperIdent(_) | Tok::Kw(_) | Tok::True | Tok::False | Tok::NoneLit
        )
    }

    /// A member name: any word, reserved or not (Section 3.2).
    fn member_name(&mut self, what: &str) -> PResult<Ident> {
        if self.at_word() {
            Ok(Self::ident_from(self.bump()))
        } else {
            self.expected(what)
        }
    }

    /// The `///` comments on the lines directly above `span`.
    fn docs_before(&self, span: Span) -> Vec<String> {
        if self.docs.is_empty() {
            return Vec::new();
        }
        let mut line = self.index.line(span.start);
        let mut docs = Vec::new();
        while line > 1 {
            line -= 1;
            match self.docs.get(&line) {
                Some(text) => docs.push(text.clone()),
                None => break,
            }
        }
        docs.reverse();
        docs
    }
}
