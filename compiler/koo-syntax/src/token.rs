//! Tokens. See `spec/grammar.ebnf`, "Lexical grammar".

use crate::diag::Span;

/// The 32 reserved words (Section 3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kw {
    Actor,
    And,
    As,
    Break,
    Continue,
    Defer,
    Distinct,
    Else,
    Enum,
    Error,
    Fail,
    Fn,
    For,
    If,
    Import,
    In,
    Interface,
    Is,
    Match,
    Not,
    Or,
    Pub,
    Resource,
    Return,
    Scope,
    Select,
    Spawn,
    Take,
    Type,
    Unsafe,
    Var,
    While,
}

impl Kw {
    pub const ALL: [(&'static str, Kw); 32] = [
        ("actor", Kw::Actor),
        ("and", Kw::And),
        ("as", Kw::As),
        ("break", Kw::Break),
        ("continue", Kw::Continue),
        ("defer", Kw::Defer),
        ("distinct", Kw::Distinct),
        ("else", Kw::Else),
        ("enum", Kw::Enum),
        ("error", Kw::Error),
        ("fail", Kw::Fail),
        ("fn", Kw::Fn),
        ("for", Kw::For),
        ("if", Kw::If),
        ("import", Kw::Import),
        ("in", Kw::In),
        ("interface", Kw::Interface),
        ("is", Kw::Is),
        ("match", Kw::Match),
        ("not", Kw::Not),
        ("or", Kw::Or),
        ("pub", Kw::Pub),
        ("resource", Kw::Resource),
        ("return", Kw::Return),
        ("scope", Kw::Scope),
        ("select", Kw::Select),
        ("spawn", Kw::Spawn),
        ("take", Kw::Take),
        ("type", Kw::Type),
        ("unsafe", Kw::Unsafe),
        ("var", Kw::Var),
        ("while", Kw::While),
    ];

    pub fn from_word(word: &str) -> Option<Kw> {
        Kw::ALL.iter().find(|(s, _)| *s == word).map(|(_, k)| *k)
    }

    pub fn as_str(self) -> &'static str {
        Kw::ALL.iter().find(|(_, k)| *k == self).map(|(s, _)| *s).unwrap()
    }
}

/// The primitive type names, which are predeclared (Sections 3.1 and 4.2).
pub const PRIMITIVES: [&str; 16] = [
    "int", "float", "bool", "string", "char", "byte", "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32",
    "f64",
];

pub fn is_primitive(name: &str) -> bool {
    PRIMITIVES.contains(&name)
}

/// One piece of an interpolated string literal.
#[derive(Debug, Clone, PartialEq)]
pub enum StrPiece {
    /// Literal text, with escapes already processed.
    Lit(String),
    /// `{expr}` or `{expr:format}`. The expression is kept as tokens for the parser.
    Interp {
        tokens: Vec<Token>,
        format: Option<String>,
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    /// An identifier that begins with a lowercase letter or an underscore.
    Ident(String),
    /// An identifier that begins with an uppercase letter.
    UpperIdent(String),
    /// `_` alone.
    Underscore,
    Kw(Kw),
    True,
    False,
    NoneLit,
    Int(String),
    Float(String),
    /// The digits of a decimal literal, without the `d`.
    Decimal(String),
    Duration(String),
    Char(char),
    Str(Vec<StrPiece>),
    RawStr(String),
    /// The path that follows `import`, taken as one token.
    ImportPath(String),

    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Semi,
    Dot,
    DotDot,
    DotDotEq,
    Ellipsis,
    ColonEq,
    Eq,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    Arrow,
    FatArrow,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    Question,
    Bang,
    Pipe,
    At,

    /// A significant line break. Never produced by the lexer: the parser sees one where
    /// Section 3.7 says a newline ends a statement.
    Newline,
    Eof,
}

impl Tok {
    /// Whether a line that begins with this token continues the statement before it
    /// (Section 3.7): `.`, `else`, `|`, and the binary operators of levels 3 to 9 except `-`.
    pub fn continues_line(&self) -> bool {
        matches!(
            self,
            Tok::Dot
                | Tok::Kw(Kw::Else)
                | Tok::Pipe
                | Tok::Star
                | Tok::Slash
                | Tok::Percent
                | Tok::Plus
                | Tok::DotDot
                | Tok::DotDotEq
                | Tok::EqEq
                | Tok::NotEq
                | Tok::Lt
                | Tok::Le
                | Tok::Gt
                | Tok::Ge
                | Tok::Kw(Kw::Is)
                | Tok::Kw(Kw::And)
                | Tok::Kw(Kw::Or)
        )
    }

    /// How the token is named in an error message.
    pub fn describe(&self) -> String {
        let fixed = match self {
            Tok::Ident(s) | Tok::UpperIdent(s) => return format!("`{s}`"),
            Tok::Kw(k) => return format!("`{}`", k.as_str()),
            Tok::Int(s) | Tok::Float(s) | Tok::Duration(s) => return format!("`{s}`"),
            Tok::Decimal(s) => return format!("`{s}d`"),
            Tok::Char(_) => "a character literal",
            Tok::Str(_) | Tok::RawStr(_) => "a string literal",
            Tok::ImportPath(s) => return format!("`{s}`"),
            Tok::Underscore => "`_`",
            Tok::True => "`true`",
            Tok::False => "`false`",
            Tok::NoneLit => "`none`",
            Tok::LParen => "`(`",
            Tok::RParen => "`)`",
            Tok::LBracket => "`[`",
            Tok::RBracket => "`]`",
            Tok::LBrace => "`{`",
            Tok::RBrace => "`}`",
            Tok::Comma => "`,`",
            Tok::Colon => "`:`",
            Tok::Semi => "`;`",
            Tok::Dot => "`.`",
            Tok::DotDot => "`..`",
            Tok::DotDotEq => "`..=`",
            Tok::Ellipsis => "`...`",
            Tok::ColonEq => "`:=`",
            Tok::Eq => "`=`",
            Tok::PlusEq => "`+=`",
            Tok::MinusEq => "`-=`",
            Tok::StarEq => "`*=`",
            Tok::SlashEq => "`/=`",
            Tok::Arrow => "`->`",
            Tok::FatArrow => "`=>`",
            Tok::Plus => "`+`",
            Tok::Minus => "`-`",
            Tok::Star => "`*`",
            Tok::Slash => "`/`",
            Tok::Percent => "`%`",
            Tok::EqEq => "`==`",
            Tok::NotEq => "`!=`",
            Tok::Lt => "`<`",
            Tok::Le => "`<=`",
            Tok::Gt => "`>`",
            Tok::Ge => "`>=`",
            Tok::Question => "`?`",
            Tok::Bang => "`!`",
            Tok::Pipe => "`|`",
            Tok::At => "`@`",
            Tok::Newline => "the end of the line",
            Tok::Eof => "the end of the file",
        };
        fixed.to_string()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: Tok,
    pub span: Span,
    /// Whether a line break separates this token from the one before it.
    pub nl_before: bool,
}
