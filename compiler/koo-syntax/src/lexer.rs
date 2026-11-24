//! The lexer. See `spec/grammar.ebnf`, "Lexical grammar", and Section 3 of the specification.
//!
//! The lexer does not decide which newlines end statements: that depends on what kind of
//! bracket is open, which only the parser knows (Section 3.7). It records on every token
//! whether a line break precedes it.

use crate::diag::{Diagnostic, Span};
use crate::token::{Kw, StrPiece, Tok, Token};

/// A `///` comment. `offset` is the position of its first slash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocComment {
    pub offset: u32,
    pub text: String,
}

pub struct Lexed {
    /// The tokens of the file. The last one is always `Tok::Eof`.
    pub tokens: Vec<Token>,
    pub docs: Vec<DocComment>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn lex(src: &str) -> Lexed {
    let mut lexer = Lexer {
        src,
        bytes: src.as_bytes(),
        pos: 0,
        diags: Vec::new(),
        docs: Vec::new(),
        interp_depth: 0,
    };
    let mut tokens: Vec<Token> = Vec::new();
    loop {
        let token = lexer.next_token();
        let done = token.kind == Tok::Eof;
        // An import path is not made of ordinary tokens (`pkg.koo.io/acme/orders-service`),
        // so the word `import` at the start of a line switches the lexer to path mode.
        let import = token.kind == Tok::Kw(Kw::Import) && (token.nl_before || tokens.is_empty());
        tokens.push(token);
        if import && let Some(path) = lexer.import_path() {
            tokens.push(path);
        }
        if done {
            break;
        }
    }
    Lexed {
        tokens,
        docs: lexer.docs,
        diagnostics: lexer.diags,
    }
}

/// Lexes a file that begins at offset `base` of its package: every span counts from the
/// start of the package.
pub fn lex_at(src: &str, base: u32) -> Lexed {
    let mut lexed = lex(src);
    if base != 0 {
        shift(&mut lexed.tokens, base);
        for doc in &mut lexed.docs {
            doc.offset += base;
        }
        for diagnostic in &mut lexed.diagnostics {
            diagnostic.span = diagnostic.span.shifted(base);
        }
    }
    lexed
}

fn shift(tokens: &mut [Token], base: u32) {
    for token in tokens {
        token.span = token.span.shifted(base);
        if let Tok::Str(pieces) = &mut token.kind {
            for piece in pieces {
                if let StrPiece::Interp { tokens, span, .. } = piece {
                    *span = span.shifted(base);
                    shift(tokens, base);
                }
            }
        }
    }
}

struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    diags: Vec<Diagnostic>,
    docs: Vec<DocComment>,
    /// How many interpolations enclose the current position.
    interp_depth: u32,
}

/// How deeply string literals may nest inside interpolations.
const MAX_INTERP_DEPTH: u32 = 8;

const UNITS: [&str; 6] = ["h", "min", "s", "ms", "us", "ns"];

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

impl<'a> Lexer<'a> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_at(&self, n: usize) -> Option<u8> {
        self.bytes.get(self.pos + n).copied()
    }

    fn rest(&self) -> &'a str {
        &self.src[self.pos..]
    }

    fn error(&mut self, start: usize, end: usize, message: impl Into<String>) {
        self.diags.push(Diagnostic::new(Span::new(start, end), message));
    }

    fn error_help(&mut self, start: usize, end: usize, message: impl Into<String>, help: impl Into<String>) {
        self.diags
            .push(Diagnostic::new(Span::new(start, end), message).with_help(help));
    }

    /// Consumes one character and returns it.
    fn next_char(&mut self) -> Option<char> {
        let c = self.rest().chars().next()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    /// Skips white space and comments. Returns whether a line break was crossed.
    fn skip_trivia(&mut self) -> bool {
        let mut newline = false;
        loop {
            match self.peek() {
                Some(b' ') | Some(b'\t') | Some(b'\r') => self.pos += 1,
                Some(b'\n') => {
                    newline = true;
                    self.pos += 1;
                }
                Some(b'/') if self.peek_at(1) == Some(b'/') => {
                    let start = self.pos;
                    let end = self.rest().find('\n').map(|i| self.pos + i).unwrap_or(self.src.len());
                    let line = self.src[start..end].trim_end_matches('\r');
                    if line.starts_with("///") && !line.starts_with("////") {
                        let text = &line[3..];
                        let text = text.strip_prefix(' ').unwrap_or(text);
                        self.docs.push(DocComment {
                            offset: start as u32,
                            text: text.to_string(),
                        });
                    }
                    self.pos = end;
                }
                _ => return newline,
            }
        }
    }

    fn import_path(&mut self) -> Option<Token> {
        let mut p = self.pos;
        while matches!(self.bytes.get(p), Some(b' ') | Some(b'\t')) {
            p += 1;
        }
        let first = *self.bytes.get(p)?;
        if !(is_word_byte(first) || first == b'.') {
            return None;
        }
        let start = p;
        while self.bytes.get(p).is_some_and(|b| !b.is_ascii_whitespace()) {
            p += 1;
        }
        self.pos = p;
        Some(Token {
            kind: Tok::ImportPath(self.src[start..p].to_string()),
            span: Span::new(start, p),
            nl_before: false,
        })
    }

    fn next_token(&mut self) -> Token {
        let mut nl_before = false;
        loop {
            nl_before |= self.skip_trivia();
            let start = self.pos;
            let Some(b) = self.peek() else {
                return Token {
                    kind: Tok::Eof,
                    span: Span::new(start, start),
                    nl_before,
                };
            };
            let kind = match b {
                b'a'..=b'z' | b'A'..=b'Z' | b'_' => Some(self.word()),
                b'0'..=b'9' => Some(self.number()),
                b'"' => Some(self.string()),
                b'`' => Some(self.raw_string()),
                b'\'' => Some(self.char_literal()),
                _ => self.punct(),
            };
            if let Some(kind) = kind {
                return Token {
                    kind,
                    span: Span::new(start, self.pos),
                    nl_before,
                };
            }
        }
    }

    fn word(&mut self) -> Tok {
        let start = self.pos;
        while self.peek().is_some_and(is_word_byte) {
            self.pos += 1;
        }
        let word = &self.src[start..self.pos];
        if word == "_" {
            return Tok::Underscore;
        }
        if let Some(kw) = Kw::from_word(word) {
            return Tok::Kw(kw);
        }
        match word {
            "true" => Tok::True,
            "false" => Tok::False,
            "none" => Tok::NoneLit,
            _ if word.as_bytes()[0].is_ascii_uppercase() => Tok::UpperIdent(word.to_string()),
            _ => Tok::Ident(word.to_string()),
        }
    }

    fn digits(&mut self, ok: impl Fn(u8) -> bool) {
        while self.peek().is_some_and(|b| ok(b) || b == b'_') {
            self.pos += 1;
        }
    }

    fn number(&mut self) -> Tok {
        let start = self.pos;
        if self.peek() == Some(b'0') && matches!(self.peek_at(1), Some(b'x') | Some(b'b')) {
            let hex = self.peek_at(1) == Some(b'x');
            self.pos += 2;
            let digits_start = self.pos;
            if hex {
                self.digits(|b| b.is_ascii_hexdigit());
            } else {
                self.digits(|b| b == b'0' || b == b'1');
            }
            let empty = self.pos == digits_start;
            // Whatever word characters follow are not part of the number.
            let end = self.pos;
            while self.peek().is_some_and(is_word_byte) {
                self.pos += 1;
            }
            if empty || self.pos != end {
                let what = if hex { "hexadecimal" } else { "binary" };
                self.error(start, self.pos, format!("invalid {what} literal"));
            } else if !underscores_ok(&self.src[digits_start - 1..end]) {
                self.error(start, end, UNDERSCORE);
            }
            return Tok::Int(self.src[start..end].to_string());
        }

        self.digits(|b| b.is_ascii_digit());
        let mut fraction = false;
        let mut exponent = false;
        if self.peek() == Some(b'.') && self.peek_at(1).is_some_and(|b| b.is_ascii_digit()) {
            fraction = true;
            self.pos += 1;
            self.digits(|b| b.is_ascii_digit());
        }
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            let sign = matches!(self.peek_at(1), Some(b'+') | Some(b'-')) as usize;
            if self.peek_at(1 + sign).is_some_and(|b| b.is_ascii_digit()) {
                exponent = true;
                self.pos += 1 + sign;
                self.digits(|b| b.is_ascii_digit());
            }
        }
        let number_end = self.pos;
        while self.peek().is_some_and(is_word_byte) {
            self.pos += 1;
        }
        let number = &self.src[start..number_end];
        let suffix = &self.src[number_end..self.pos];
        if !underscores_ok(&self.src[start..self.pos]) {
            self.error(start, self.pos, UNDERSCORE);
        }
        if suffix.is_empty() {
            return if fraction || exponent {
                Tok::Float(number.to_string())
            } else {
                Tok::Int(number.to_string())
            };
        }
        if suffix == "d" && !exponent {
            return Tok::Decimal(number.to_string());
        }
        let text = &self.src[start..self.pos];
        if !fraction && !exponent {
            match check_duration(text) {
                Ok(()) => return Tok::Duration(text.to_string()),
                Err(Some(message)) => {
                    self.error(start, self.pos, message);
                    return Tok::Duration(text.to_string());
                }
                Err(None) => {}
            }
        }
        self.error_help(
            start,
            self.pos,
            format!("invalid number `{text}`"),
            "a number may end in `d` (decimal) or in duration units: `ns`, `us`, `ms`, `s`, `min`, `h`",
        );
        if fraction || exponent {
            Tok::Float(number.to_string())
        } else {
            Tok::Int(number.to_string())
        }
    }

    /// Reads the escape sequence at the backslash.
    fn escape(&mut self) -> Option<char> {
        let start = self.pos;
        self.pos += 1;
        if matches!(self.peek(), None | Some(b'\n') | Some(b'\r')) {
            self.error_help(
                start,
                self.pos,
                "a `\\` at the end of a line is not an escape",
                "a backslash in a string is written `\\\\`",
            );
            return None;
        }
        let c = self.next_char()?;
        Some(match c {
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            '0' => '\0',
            '\\' => '\\',
            '"' => '"',
            '\'' => '\'',
            '{' => '{',
            'u' => {
                if self.peek() != Some(b'{') {
                    self.error(start, self.pos, "a Unicode escape is written `\\u{1F600}`");
                    return None;
                }
                self.pos += 1;
                let hex_start = self.pos;
                while self.peek().is_some_and(|b| b.is_ascii_hexdigit()) {
                    self.pos += 1;
                }
                let hex = &self.src[hex_start..self.pos];
                if self.peek() != Some(b'}') {
                    self.error(start, self.pos, "a Unicode escape is written `\\u{1F600}`");
                    return None;
                }
                self.pos += 1;
                match u32::from_str_radix(hex, 16).ok().and_then(char::from_u32) {
                    Some(c) => c,
                    None => {
                        self.error(start, self.pos, "not a Unicode scalar value");
                        return None;
                    }
                }
            }
            _ => {
                self.error_help(
                    start,
                    self.pos,
                    format!("unknown escape `\\{c}`"),
                    "the escapes are \\n \\r \\t \\0 \\\\ \\\" \\' \\{ and \\u{...}",
                );
                return None;
            }
        })
    }

    /// Handles one element of an interpolated string: an escape, an interpolation or a character.
    fn string_element(&mut self, pieces: &mut Vec<StrPiece>, text: &mut String) {
        match self.peek() {
            Some(b'\\') => {
                if let Some(c) = self.escape() {
                    text.push(c);
                }
            }
            Some(b'{') => {
                if !text.is_empty() {
                    pieces.push(StrPiece::Lit(std::mem::take(text)));
                }
                pieces.push(self.interpolation());
            }
            _ => {
                if let Some(c) = self.next_char() {
                    text.push(c);
                }
            }
        }
    }

    fn string(&mut self) -> Tok {
        if self.rest().starts_with("\"\"\"") {
            return self.text_block();
        }
        let start = self.pos;
        self.pos += 1;
        let mut pieces = Vec::new();
        let mut text = String::new();
        loop {
            match self.peek() {
                None | Some(b'\n') => {
                    self.error_help(
                        start,
                        self.pos,
                        "this string literal is not closed",
                        "a string ends on the line it begins; use a text block (`\"\"\"`) for several lines",
                    );
                    break;
                }
                Some(b'"') => {
                    self.pos += 1;
                    break;
                }
                _ => self.string_element(&mut pieces, &mut text),
            }
        }
        if !text.is_empty() {
            pieces.push(StrPiece::Lit(text));
        }
        Tok::Str(pieces)
    }

    /// Reads `{expr}` or `{expr:format}` at the opening brace.
    fn interpolation(&mut self) -> StrPiece {
        self.interp_depth += 1;
        let piece = self.interpolation_unguarded();
        self.interp_depth -= 1;
        piece
    }

    fn interpolation_unguarded(&mut self) -> StrPiece {
        let start = self.pos;
        if self.interp_depth > MAX_INTERP_DEPTH {
            let end = self.rest().find('\n').map(|i| self.pos + i).unwrap_or(self.src.len());
            self.error(start, start + 1, "interpolations are nested too deeply");
            self.pos = end;
            return StrPiece::Interp {
                tokens: Vec::new(),
                format: None,
                span: Span::new(start, end),
            };
        }
        self.pos += 1;
        let mut tokens = Vec::new();
        let mut format = None;
        let mut depth = 0usize;
        loop {
            while matches!(self.peek(), Some(b' ') | Some(b'\t')) {
                self.pos += 1;
            }
            match self.peek() {
                None | Some(b'\n') | Some(b'\r') => {
                    self.error(
                        start,
                        self.pos,
                        "this interpolation is not closed; an interpolation is written on one line",
                    );
                    break;
                }
                Some(b'/') if self.peek_at(1) == Some(b'/') => {
                    self.error(start, self.pos, "a comment cannot appear inside an interpolation");
                    break;
                }
                Some(b'}') if depth == 0 => {
                    self.pos += 1;
                    break;
                }
                Some(b':') if depth == 0 && self.peek_at(1) != Some(b'=') => {
                    self.pos += 1;
                    let format_start = self.pos;
                    while self.peek().is_some_and(|b| b != b'}' && b != b'\n') {
                        self.next_char();
                    }
                    format = Some(self.src[format_start..self.pos].to_string());
                    if self.peek() == Some(b'}') {
                        self.pos += 1;
                    } else {
                        self.error(start, self.pos, "this interpolation is not closed");
                    }
                    break;
                }
                _ => {
                    let token = self.next_token();
                    match token.kind {
                        Tok::LParen | Tok::LBracket | Tok::LBrace => depth += 1,
                        Tok::RParen | Tok::RBracket | Tok::RBrace => depth = depth.saturating_sub(1),
                        _ => {}
                    }
                    tokens.push(Token {
                        nl_before: false,
                        ..token
                    });
                }
            }
        }
        if tokens.is_empty() {
            self.error_help(
                start,
                self.pos,
                "empty interpolation",
                "a literal brace in a string is written `\\{`",
            );
        }
        StrPiece::Interp {
            tokens,
            format,
            span: Span::new(start, self.pos),
        }
    }

    fn text_block(&mut self) -> Tok {
        let start = self.pos;
        self.pos += 3;
        let line_end = self.rest().find('\n').map(|i| self.pos + i);
        let after = &self.src[self.pos..line_end.unwrap_or(self.src.len())];
        if !after.trim().is_empty() {
            self.error(
                self.pos,
                self.pos + after.len(),
                "the text of a text block begins on the line after `\"\"\"`",
            );
        }
        let Some(mut line_start) = line_end.map(|e| e + 1) else {
            self.error(start, self.src.len(), "this text block is not closed");
            self.pos = self.src.len();
            return Tok::Str(Vec::new());
        };

        // Find the closing delimiter: a line whose first non-blank characters are `"""`.
        let mut lines: Vec<(usize, usize)> = Vec::new();
        let close;
        loop {
            let end = self.src[line_start..]
                .find('\n')
                .map(|i| line_start + i)
                .unwrap_or(self.src.len());
            let line = &self.src[line_start..end];
            let trimmed = line.trim_start();
            if trimmed.starts_with("\"\"\"") {
                close = Some((line_start, line.len() - trimmed.len()));
                break;
            }
            lines.push((line_start, end));
            if end == self.src.len() {
                close = None;
                break;
            }
            line_start = end + 1;
        }
        let Some((close_line, indent_len)) = close else {
            self.error(start, start + 3, "this text block is not closed");
            self.pos = self.src.len();
            return Tok::Str(Vec::new());
        };
        let indent = &self.src[close_line..close_line + indent_len];

        let mut pieces = Vec::new();
        let mut text = String::new();
        for (i, &(line_start, line_end)) in lines.iter().enumerate() {
            if i > 0 {
                text.push('\n');
            }
            let line = self.src[line_start..line_end].trim_end_matches('\r');
            if line.trim().is_empty() {
                continue;
            }
            let body_start = if line.starts_with(indent) {
                line_start + indent.len()
            } else {
                self.error(
                    line_start,
                    line_start + line.len(),
                    "this line is indented less than the closing `\"\"\"` of its text block",
                );
                line_start + (line.len() - line.trim_start().len())
            };
            let body_end = line_start + line.len();
            self.pos = body_start;
            while self.pos < body_end {
                self.string_element(&mut pieces, &mut text);
            }
        }
        if !text.is_empty() {
            pieces.push(StrPiece::Lit(text));
        }
        self.pos = close_line + indent_len + 3;
        Tok::Str(pieces)
    }

    fn raw_string(&mut self) -> Tok {
        let start = self.pos;
        self.pos += 1;
        match self.rest().find('`') {
            Some(i) => {
                let text = self.rest()[..i].to_string();
                self.pos += i + 1;
                Tok::RawStr(text)
            }
            None => {
                self.error(start, start + 1, "this raw string is not closed");
                let text = self.rest().to_string();
                self.pos = self.src.len();
                Tok::RawStr(text)
            }
        }
    }

    fn char_literal(&mut self) -> Tok {
        let start = self.pos;
        self.pos += 1;
        let c = match self.peek() {
            Some(b'\\') => self.escape().unwrap_or('\0'),
            Some(b'\'') | Some(b'\n') | None => {
                self.error(start, self.pos, "empty character literal");
                if self.peek() == Some(b'\'') {
                    self.pos += 1;
                }
                return Tok::Char('\0');
            }
            _ => self.next_char().unwrap_or('\0'),
        };
        if self.peek() == Some(b'\'') {
            self.pos += 1;
        } else {
            self.error_help(
                start,
                self.pos,
                "this character literal is not closed",
                "a character literal holds one character; strings are written with double quotes",
            );
        }
        Tok::Char(c)
    }

    /// Reads punctuation. Returns `None` after reporting a character that is not part of Koo,
    /// so that the caller moves on to the next token.
    fn punct(&mut self) -> Option<Tok> {
        let start = self.pos;
        let b = self.bytes[self.pos];
        let next = self.peek_at(1);
        let third = self.peek_at(2);
        let (kind, len) = match b {
            b'(' => (Tok::LParen, 1),
            b')' => (Tok::RParen, 1),
            b'[' => (Tok::LBracket, 1),
            b']' => (Tok::RBracket, 1),
            b'{' => (Tok::LBrace, 1),
            b'}' => (Tok::RBrace, 1),
            b',' => (Tok::Comma, 1),
            b';' => (Tok::Semi, 1),
            b'?' => (Tok::Question, 1),
            b'@' => (Tok::At, 1),
            b'%' => (Tok::Percent, 1),
            b'.' if next == Some(b'.') && third == Some(b'.') => (Tok::Ellipsis, 3),
            b'.' if next == Some(b'.') && third == Some(b'=') => (Tok::DotDotEq, 3),
            b'.' if next == Some(b'.') => (Tok::DotDot, 2),
            b'.' => (Tok::Dot, 1),
            b':' if next == Some(b'=') => (Tok::ColonEq, 2),
            b':' => (Tok::Colon, 1),
            b'=' if next == Some(b'=') => (Tok::EqEq, 2),
            b'=' if next == Some(b'>') => (Tok::FatArrow, 2),
            b'=' => (Tok::Eq, 1),
            b'+' if next == Some(b'=') => (Tok::PlusEq, 2),
            b'+' if next == Some(b'+') => {
                self.error_help(start, start + 2, "`++` does not exist", "write `x += 1`");
                (Tok::Plus, 2)
            }
            b'+' => (Tok::Plus, 1),
            b'-' if next == Some(b'>') => (Tok::Arrow, 2),
            b'-' if next == Some(b'=') => (Tok::MinusEq, 2),
            b'-' if next == Some(b'-') => {
                self.error_help(
                    start,
                    start + 2,
                    "`--` does not exist",
                    "write `x -= 1`, or `-(-x)` to negate twice",
                );
                (Tok::Minus, 2)
            }
            b'-' => (Tok::Minus, 1),
            b'*' if next == Some(b'=') => (Tok::StarEq, 2),
            b'*' => (Tok::Star, 1),
            b'/' if next == Some(b'=') => (Tok::SlashEq, 2),
            b'/' => (Tok::Slash, 1),
            b'<' if next == Some(b'=') => (Tok::Le, 2),
            b'<' if next == Some(b'<') => {
                self.error_help(
                    start,
                    start + 2,
                    "`<<` does not exist",
                    "bit operations are methods: `a.shl(n)`",
                );
                (Tok::Lt, 2)
            }
            b'<' => (Tok::Lt, 1),
            b'>' if next == Some(b'=') => (Tok::Ge, 2),
            b'>' => (Tok::Gt, 1),
            b'!' if next == Some(b'=') => (Tok::NotEq, 2),
            b'!' => (Tok::Bang, 1),
            b'|' if next == Some(b'|') => {
                self.error_help(start, start + 2, "`||` does not exist", "write `or`");
                (Tok::Kw(Kw::Or), 2)
            }
            b'|' => (Tok::Pipe, 1),
            b'&' if next == Some(b'&') => {
                self.error_help(start, start + 2, "`&&` does not exist", "write `and`");
                (Tok::Kw(Kw::And), 2)
            }
            b'&' | b'^' | b'~' => {
                let c = b as char;
                self.error_help(
                    start,
                    start + 1,
                    format!("`{c}` is not an operator"),
                    "bit operations are methods: `a.bit_and(b)`, `a.bit_or(b)`, `a.bit_xor(b)`, `a.bit_not()`",
                );
                self.pos += 1;
                return None;
            }
            _ => {
                let c = self.next_char().unwrap();
                self.error(start, self.pos, format!("unexpected character `{c}`"));
                return None;
            }
        };
        self.pos += len;
        Some(kind)
    }
}

const UNDERSCORE: &str = "an underscore in a number separates two digits";

/// Whether every underscore in a number has a digit on both sides. In `x1_F` the leading
/// `x` stands for the prefix of a hexadecimal literal, whose digits include letters.
fn underscores_ok(text: &str) -> bool {
    let hex = text.starts_with('x');
    let digit = |b: u8| if hex { b.is_ascii_hexdigit() } else { b.is_ascii_digit() };
    let bytes = text.as_bytes();
    (0..bytes.len())
        .filter(|&i| bytes[i] == b'_')
        .all(|i| i > 0 && i + 1 < bytes.len() && digit(bytes[i - 1]) && digit(bytes[i + 1]))
}

/// Checks a duration literal such as `1h30min`: number-unit pairs, largest unit first.
/// `Err(None)` means the text is not a duration at all.
fn check_duration(text: &str) -> Result<(), Option<String>> {
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut last_unit: Option<usize> = None;
    while i < bytes.len() {
        let digits_start = i;
        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
            i += 1;
        }
        if i == digits_start {
            return Err(None);
        }
        let unit_start = i;
        while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
            i += 1;
        }
        let unit = &text[unit_start..i];
        let Some(rank) = UNITS.iter().position(|u| *u == unit) else {
            return Err(None);
        };
        if last_unit.is_some_and(|last| rank <= last) {
            return Err(Some(format!(
                "in the duration `{text}` the units must go from largest to smallest, each at most once"
            )));
        }
        last_unit = Some(rank);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<Tok> {
        let lexed = lex(src);
        assert!(
            lexed.diagnostics.is_empty(),
            "unexpected errors: {:?}",
            lexed.diagnostics
        );
        lexed
            .tokens
            .into_iter()
            .map(|t| t.kind)
            .filter(|k| *k != Tok::Eof)
            .collect()
    }

    fn errors(src: &str) -> Vec<String> {
        lex(src).diagnostics.into_iter().map(|d| d.message).collect()
    }

    fn lit(s: &str) -> StrPiece {
        StrPiece::Lit(s.to_string())
    }

    #[test]
    fn numbers() {
        assert_eq!(
            kinds("42 1_000_000 0xFF 0b1010 3.14 1e9 2.5e-3 19.99d 110d 0x1d"),
            vec![
                Tok::Int("42".into()),
                Tok::Int("1_000_000".into()),
                Tok::Int("0xFF".into()),
                Tok::Int("0b1010".into()),
                Tok::Float("3.14".into()),
                Tok::Float("1e9".into()),
                Tok::Float("2.5e-3".into()),
                Tok::Decimal("19.99".into()),
                Tok::Decimal("110".into()),
                Tok::Int("0x1d".into()),
            ]
        );
    }

    #[test]
    fn durations() {
        assert_eq!(
            kinds("30s 5min 2h 250ms 10us 1h30min"),
            ["30s", "5min", "2h", "250ms", "10us", "1h30min"]
                .map(|s| Tok::Duration(s.into()))
                .to_vec()
        );
        assert_eq!(errors("30min1h").len(), 1);
        assert_eq!(errors("5days").len(), 1);
        assert_eq!(errors("1.5s").len(), 1);
    }

    #[test]
    fn ranges_and_members_after_integers() {
        assert_eq!(
            kinds("0..8"),
            vec![Tok::Int("0".into()), Tok::DotDot, Tok::Int("8".into())]
        );
        assert_eq!(
            kinds("1..=9"),
            vec![Tok::Int("1".into()), Tok::DotDotEq, Tok::Int("9".into())]
        );
        assert_eq!(
            kinds("5.abs"),
            vec![Tok::Int("5".into()), Tok::Dot, Tok::Ident("abs".into())]
        );
    }

    #[test]
    fn words() {
        assert_eq!(
            kinds("fn Order get_order _ _cache true none Self self"),
            vec![
                Tok::Kw(Kw::Fn),
                Tok::UpperIdent("Order".into()),
                Tok::Ident("get_order".into()),
                Tok::Underscore,
                Tok::Ident("_cache".into()),
                Tok::True,
                Tok::NoneLit,
                Tok::UpperIdent("Self".into()),
                Tok::Ident("self".into()),
            ]
        );
        assert_eq!(Kw::ALL.len(), 32);
    }

    #[test]
    fn punctuation() {
        assert_eq!(
            kinds(":= => -> ..= ... .. += != <= >= ? ! | @"),
            vec![
                Tok::ColonEq,
                Tok::FatArrow,
                Tok::Arrow,
                Tok::DotDotEq,
                Tok::Ellipsis,
                Tok::DotDot,
                Tok::PlusEq,
                Tok::NotEq,
                Tok::Le,
                Tok::Ge,
                Tok::Question,
                Tok::Bang,
                Tok::Pipe,
                Tok::At,
            ]
        );
        assert_eq!(kinds("List<List<int>>").iter().filter(|k| **k == Tok::Gt).count(), 2);
    }

    #[test]
    fn absent_operators_are_reported() {
        assert_eq!(errors("a && b"), vec!["`&&` does not exist"]);
        assert_eq!(errors("a || b"), vec!["`||` does not exist"]);
        assert_eq!(errors("i++"), vec!["`++` does not exist"]);
        assert_eq!(errors("a & b"), vec!["`&` is not an operator"]);
    }

    #[test]
    fn strings() {
        assert_eq!(kinds(r#""hello""#), vec![Tok::Str(vec![lit("hello")])]);
        assert_eq!(kinds(r#""""#), vec![Tok::Str(vec![])]);
        assert_eq!(
            kinds(r#""a\n\t\"\\\{\u{41}""#),
            vec![Tok::Str(vec![lit("a\n\t\"\\{A")])]
        );
        assert_eq!(
            kinds(r#""http://x/y } z""#),
            vec![Tok::Str(vec![lit("http://x/y } z")])]
        );
        assert_eq!(kinds("`raw \\n {x}`"), vec![Tok::RawStr("raw \\n {x}".into())]);
        assert_eq!(
            kinds(r"'a' '\n' '\u{1F600}'"),
            vec![Tok::Char('a'), Tok::Char('\n'), Tok::Char('\u{1F600}')]
        );
    }

    #[test]
    fn interpolation() {
        let toks = kinds(r#""total: {order.total} for {id:08}""#);
        let Tok::Str(pieces) = &toks[0] else { panic!() };
        assert_eq!(pieces.len(), 4);
        assert_eq!(pieces[0], lit("total: "));
        let StrPiece::Interp { tokens, format, .. } = &pieces[1] else {
            panic!()
        };
        let inner: Vec<Tok> = tokens.iter().map(|t| t.kind.clone()).collect();
        assert_eq!(
            inner,
            vec![Tok::Ident("order".into()), Tok::Dot, Tok::Ident("total".into())]
        );
        assert_eq!(*format, None);
        let StrPiece::Interp { format, .. } = &pieces[3] else {
            panic!()
        };
        assert_eq!(format.as_deref(), Some("08"));
    }

    #[test]
    fn interpolation_nests() {
        let toks = kinds(r#""{names["first"]} {f(a: 1)} {if ok { 1 } else { 2 }}""#);
        let Tok::Str(pieces) = &toks[0] else { panic!() };
        let interps: Vec<_> = pieces.iter().filter(|p| matches!(p, StrPiece::Interp { .. })).collect();
        assert_eq!(interps.len(), 3);
        for p in interps {
            let StrPiece::Interp { format, .. } = p else { panic!() };
            assert_eq!(*format, None);
        }
    }

    #[test]
    fn text_block() {
        let src = "q := \"\"\"\n    select *\n      from orders\n\n    where id = {id}\n    \"\"\"\n";
        let toks = kinds(src);
        let Tok::Str(pieces) = &toks[2] else { panic!("{toks:?}") };
        assert_eq!(pieces[0], lit("select *\n  from orders\n\nwhere id = "));
        assert!(matches!(pieces[1], StrPiece::Interp { .. }));
        assert_eq!(pieces.len(), 2);
    }

    #[test]
    fn string_errors() {
        assert_eq!(errors("\"abc\nx").len(), 1);
        assert_eq!(errors(r#""a {} b""#), vec!["empty interpolation"]);
        assert_eq!(errors(r#""\q""#).len(), 1);
        assert_eq!(errors("\"\"\"\n  a\n").len(), 1);
    }

    #[test]
    fn comments_and_newlines() {
        let lexed = lex("a // one\n/// doc\nb\n");
        assert_eq!(lexed.docs.len(), 1);
        assert_eq!(lexed.docs[0].text, "doc");
        assert!(!lexed.tokens[0].nl_before);
        assert!(lexed.tokens[1].nl_before);
    }

    #[test]
    fn import_paths() {
        assert_eq!(
            kinds("import pkg.koo.io/acme/orders-service v2 as orders"),
            vec![
                Tok::Kw(Kw::Import),
                Tok::ImportPath("pkg.koo.io/acme/orders-service".into()),
                Tok::Ident("v2".into()),
                Tok::Kw(Kw::As),
                Tok::Ident("orders".into()),
            ]
        );
        assert_eq!(
            kinds("import c \"zlib.h\" link \"z\""),
            vec![
                Tok::Kw(Kw::Import),
                Tok::ImportPath("c".into()),
                Tok::Str(vec![lit("zlib.h")]),
                Tok::Ident("link".into()),
                Tok::Str(vec![lit("z")]),
            ]
        );
        // `import` as a member name is not followed by a path.
        assert_eq!(kinds("x.import(y)")[2], Tok::Kw(Kw::Import));
        assert_eq!(kinds("x.import(y)")[3], Tok::LParen);
    }
}
