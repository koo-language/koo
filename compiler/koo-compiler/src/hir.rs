//! The typed program that the checker produces and the code generator consumes.
//!
//! Names are resolved to ids, every expression carries its type, and the implicit
//! conversions of Section 4 (the widenings) are explicit nodes.

use koo_syntax::Span;

pub type StructId = usize;
pub type EnumId = usize;
pub type ErrorId = usize;
pub type FnId = usize;
pub type LocalId = usize;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum Ty {
    Int,
    Float,
    Bool,
    Byte,
    Str,
    #[default]
    Unit,
    /// The type of an expression that does not return: `return`, `fail`, `panic(...)`.
    Never,
    List(Box<Ty>),
    Optional(Box<Ty>),
    /// `T ! E`.
    Result(Box<Ty>, ErrSet),
    Struct(StructId),
    Enum(EnumId),
    /// A value of one error type.
    Error(ErrorId),
    /// A value of any error type: the interface `Failure`.
    Failure,
    /// `(A, B)`, with two or more elements.
    Tuple(Vec<Ty>),
    /// `fn(A, B) -> R ! E`.
    Fn(Box<FnTy>),
    /// The type of an expression that failed to check. It is compatible with every type,
    /// so that one mistake is reported once.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FnTy {
    pub params: Vec<(ParamKind, Ty)>,
    /// The return type, a result type if the function can fail.
    pub ret: Ty,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ErrSet {
    /// `! Failure`: any error.
    Any,
    /// The listed error types, sorted and without duplicates. Never empty.
    Of(Vec<ErrorId>),
}

impl ErrSet {
    pub fn of(mut ids: Vec<ErrorId>) -> ErrSet {
        ids.sort_unstable();
        ids.dedup();
        ErrSet::Of(ids)
    }

    pub fn contains(&self, id: ErrorId) -> bool {
        match self {
            ErrSet::Any => true,
            ErrSet::Of(ids) => ids.contains(&id),
        }
    }

    /// Whether every error of `self` is in `other`.
    pub fn within(&self, other: &ErrSet) -> bool {
        match (self, other) {
            (_, ErrSet::Any) => true,
            (ErrSet::Any, ErrSet::Of(_)) => false,
            (ErrSet::Of(a), ErrSet::Of(b)) => a.iter().all(|id| b.contains(id)),
        }
    }
}

impl Ty {
    pub fn list(element: Ty) -> Ty {
        Ty::List(Box::new(element))
    }

    pub fn optional(inner: Ty) -> Ty {
        Ty::Optional(Box::new(inner))
    }

    pub fn is_unknown(&self) -> bool {
        matches!(self, Ty::Unknown)
    }
}

#[derive(Debug, Clone)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<FieldDef>,
}

#[derive(Debug, Clone)]
pub struct FieldDef {
    pub name: String,
    pub ty: Ty,
}

#[derive(Debug, Clone)]
pub struct EnumDef {
    pub name: String,
    pub variants: Vec<VariantDef>,
}

#[derive(Debug, Clone)]
pub struct VariantDef {
    pub name: String,
    pub fields: Vec<FieldDef>,
}

#[derive(Debug, Clone)]
pub struct ErrorDef {
    pub name: String,
    pub fields: Vec<FieldDef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParamKind {
    Borrow,
    Var,
    Take,
}

/// How a local variable came to be, which decides who owns its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalKind {
    /// A binding that owns its value: `:=`, a pattern binding, an `else` binding.
    Owned,
    Param(ParamKind),
    /// The variable of a `for` loop over a list: it names an element of the list.
    Element,
    /// In a lambda: a variable of the enclosing function. A block uses it in place
    /// (`by_ref`); a closure holds a copy of it.
    Capture {
        by_ref: bool,
    },
}

#[derive(Debug, Clone)]
pub struct Local {
    pub name: String,
    pub ty: Ty,
    pub mutable: bool,
    pub kind: LocalKind,
}

#[derive(Debug, Clone, Default)]
pub struct Function {
    /// The name as the programmer wrote it, for messages: `Order.new`, `Order.describe`.
    pub name: String,
    /// The first `params` locals are the parameters, in order.
    pub params: usize,
    pub locals: Vec<Local>,
    /// The declared return type, a result type if the function has an error set.
    pub ret: Ty,
    pub body: Block,
    pub span: Span,
    /// Whether the function is the body of a lambda. Such a function takes the function
    /// value it belongs to as a hidden first parameter, through which it reaches what the
    /// lambda captured.
    pub closure: bool,
    /// For a lambda: its locals that stand for captured variables, in capture order.
    pub captures: Vec<LocalId>,
    /// Whether the function comes from the prelude, whose source is not the program's.
    pub prelude: bool,
    /// Whether it is a function of the library that is told where it was called: its
    /// last parameter, `location`, is the compiler's to give.
    pub located: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Program {
    /// The types with a hand-written `to_string()`, which replaces the generated text form
    /// (Section 4.6), and the function that is it.
    pub to_string: std::collections::HashMap<Ty, FnId>,
    /// The types whose `==` is a function of the prelude's and not a comparison of
    /// their fields: a map or a set is what it holds, not how it happens to be laid out.
    pub equals: std::collections::HashMap<Ty, FnId>,
    pub structs: Vec<StructDef>,
    pub enums: Vec<EnumDef>,
    pub errors: Vec<ErrorDef>,
    pub functions: Vec<Function>,
    pub main: Option<FnId>,
    /// For `koo test`: the test functions, by name. The program then runs them, and has
    /// no `main`.
    pub tests: Vec<(String, FnId)>,
    pub test_mode: bool,
}

/// Statements, and the value of the block if it is used for one (Section 6.6).
#[derive(Debug, Clone, Default)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub value: Option<Box<Expr>>,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Let {
        local: LocalId,
        init: Expr,
    },
    Assign {
        target: Expr,
        op: Option<BinOp>,
        value: Expr,
        span: Span,
    },
    Expr(Expr),
    While {
        cond: Cond,
        body: Block,
    },
    ForRange {
        local: LocalId,
        lo: Expr,
        hi: Expr,
        inclusive: bool,
        body: Block,
    },
    ForList {
        local: LocalId,
        list: Expr,
        body: Block,
    },
}

#[derive(Debug, Clone)]
pub enum Cond {
    Expr(Expr),
    /// `if x := e`: runs when the optional `e` holds a value, bound to `local`.
    Bind {
        local: LocalId,
        value: Expr,
    },
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub ty: Ty,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Debug, Clone)]
pub struct Arg {
    pub kind: ParamKind,
    pub expr: Expr,
}

#[derive(Debug, Clone)]
pub enum InterpPart {
    Lit(String),
    /// A value and the format written after its colon, if any.
    Value(Expr, Format),
}

/// The format of an interpolation: `{ratio:.2}`, `{id:08}`, `{name:>10}` (Section 3.3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Format {
    /// For a float: the number of digits after the point.
    pub precision: Option<u32>,
    /// The least width, in characters; narrower text is padded.
    pub width: u32,
    pub align: Align,
    /// Whether a number is padded with zeros after its sign.
    pub zero: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Align {
    /// Numbers to the right, everything else to the left.
    #[default]
    Natural,
    Left,
    Right,
    Center,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    /// `print(x)`.
    Print,
    /// `s.len`, `xs.len`.
    Len,
    /// `xs.push(x)`; the first argument is the list, a place.
    ListPush,
    /// `xs.pop()`, giving `T?`.
    ListPop,
    /// `xs.get(i)`, giving `T?`.
    ListGet,
    /// `xs.swap(i, j)`.
    ListSwap,
    /// `xs.clear()`.
    ListClear,
    /// `xs.sort()` for a list of integers, which the support library sorts.
    ListSortInts,
    /// `List<T>.with_size(n)`: `n` zero values.
    ListWithSize,
    /// `List<T>.with_capacity(n)`: empty, with room for `n`.
    ListWithCapacity,
    /// `int.parse(s)`, giving `int ! ParseError`.
    IntParse,
    /// `float.parse(s)`, giving `float ! ParseError`.
    FloatParse,
    /// `float.from(i)`.
    FloatFromInt,
    /// `int.from(b)` for a byte.
    IntFromByte,
    /// `byte.from(i)`, giving `byte ! Overflow`.
    ByteFromInt,
    /// `f.sqrt()`.
    FloatSqrt,
    /// `f.abs()`, `i.abs()`.
    Abs,
    /// `f.floor()`, `f.ceil()`, `f.round()`, `f.trunc()`, each giving `int ! Overflow`.
    FloatFloor,
    FloatCeil,
    FloatRound,
    FloatTrunc,
    /// `a.compare(b)`: negative, zero or positive, by the total order of the type.
    Compare,
    /// `x.hash()`.
    Hash,
    /// The text form of a value as it appears inside another: a string is quoted.
    Repr,
    /// `a.min(b)`, `a.max(b)`.
    Min,
    Max,
    /// `a.wrapping_add(b)`, `a.wrapping_sub(b)`, `a.wrapping_mul(b)`.
    WrappingAdd,
    WrappingSub,
    WrappingMul,
    /// `a.bit_and(b)`, `a.bit_or(b)`, `a.bit_xor(b)`, `a.shl(n)`, `a.shr(n)`.
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Unit,
    Local(LocalId),
    /// A field of a struct or of an error value.
    Field(Box<Expr>, usize),
    Index(Box<Expr>, Box<Expr>),
    Unary(UnOp, Box<Expr>),
    /// The operand types decide the operation: integer, float or string.
    Binary(BinOp, Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Call {
        func: FnId,
        args: Vec<Arg>,
    },
    Builtin(Builtin, Vec<Expr>),
    StructLit(StructId, Vec<Expr>),
    EnumLit(EnumId, usize, Vec<Expr>),
    ErrorLit(ErrorId, Vec<Expr>),
    ListLit(Vec<Expr>),
    /// `(a, b)`. An element is read with `Field`.
    TupleLit(Vec<Expr>),
    Interp(Vec<InterpPart>),
    /// A lambda. `captures` are the variables of the enclosing function it uses, in the
    /// order of the lambda's own `captures`. A block refers to them; a closure copies them.
    Closure {
        func: FnId,
        captures: Vec<LocalId>,
        block: bool,
    },
    /// A named function used as a value.
    FnRef(FnId),
    /// The file and line of the expression, as a string: `main.koo:12`.
    Location,
    /// A call of a function of the support library, from the prelude.
    Intrinsic {
        name: &'static str,
        args: Vec<Expr>,
    },
    /// A call of a function value.
    CallValue {
        callee: Box<Expr>,
        args: Vec<Arg>,
    },
    /// `T` used where `T?` is expected.
    WrapSome(Box<Expr>),
    /// `none`.
    NoneLit,
    /// `T` used where `T ! E` is expected.
    WrapOk(Box<Expr>),
    /// A value retyped without changing its representation: an error set widened, or an
    /// error value used as a `Failure`.
    Retype(Box<Expr>),
    /// `x?`, on an optional or a result.
    Try(Box<Expr>),
    /// `x else alt`. The binding, if any, holds the error.
    Else {
        expr: Box<Expr>,
        binding: Option<LocalId>,
        alt: Box<Block>,
    },
    If {
        cond: Box<Cond>,
        then: Block,
        otherwise: Option<Block>,
    },
    Match {
        scrutinee: Box<Expr>,
        arms: Vec<Arm>,
    },
    Return(Option<Box<Expr>>),
    Fail(Box<Expr>),
    Break,
    Continue,
    Panic(Box<Expr>),
}

#[derive(Debug, Clone)]
pub struct Arm {
    /// The alternatives joined by `|`.
    pub patterns: Vec<Pattern>,
    pub guard: Option<Expr>,
    pub body: Block,
}

#[derive(Debug, Clone)]
pub enum Pattern {
    Wildcard,
    /// Binds the matched value, a copy of it, to a local.
    Bind(LocalId),
    Int(i64),
    IntRange(i64, i64),
    Bool(bool),
    Str(String),
    /// On an optional: `none`.
    None,
    /// On an optional: a pattern for the value it holds.
    Some(Box<Pattern>),
    /// On a result: a pattern for the success value.
    Ok(Box<Pattern>),
    /// On a result: `! Name`, `! e: Name`, `! e`. `error` is `None` for any error.
    Err {
        error: Option<ErrorId>,
        binding: Option<LocalId>,
        fields: Vec<(usize, Pattern)>,
    },
    Variant {
        variant: usize,
        fields: Vec<(usize, Pattern)>,
    },
    Struct {
        fields: Vec<(usize, Pattern)>,
    },
    Tuple(Vec<Pattern>),
}
