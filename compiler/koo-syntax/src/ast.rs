//! The syntax tree. Its shape follows `spec/grammar.ebnf`.

use crate::diag::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

impl Ident {
    /// Whether the name begins with an uppercase letter, which makes it a type, variant,
    /// actor, interface or error name (Section 3.1).
    pub fn is_upper(&self) -> bool {
        self.name.as_bytes().first().is_some_and(|b| b.is_ascii_uppercase())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct File {
    pub imports: Vec<Import>,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub kind: ImportKind,
    /// The name given with `as`.
    pub alias: Option<Ident>,
    /// The name the import binds in the file.
    pub binding: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImportKind {
    Package { path: String, major: Option<u32> },
    C { header: String, link: Option<String> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    pub name: Ident,
    pub args: Vec<AttrArg>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AttrArg {
    pub name: Option<Ident>,
    pub value: AttrValue,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AttrValue {
    Literal(Expr),
    /// `skip`, `yaml.codec`, `restart(max: 5, within: 1min)`.
    Path {
        path: Vec<Ident>,
        args: Option<Vec<AttrArg>>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub docs: Vec<String>,
    pub attrs: Vec<Attribute>,
    pub public: bool,
    pub kind: ItemKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ItemKind {
    Type(TypeDecl),
    Enum(EnumDecl),
    Error(ErrorDecl),
    Interface(InterfaceDecl),
    Actor(ActorDecl),
    Fn(FnDecl),
    Const(ConstDecl),
    /// A statement at the top level. Only `parse_fragment` produces it.
    Stmt(Stmt),
}

/// `type Name = ...` and `resource Name = { ... }`.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeDecl {
    pub resource: bool,
    pub name: Ident,
    pub type_params: Vec<TypeParam>,
    pub satisfies: Vec<Type>,
    pub body: TypeBody,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeBody {
    Struct(Vec<Field>),
    Distinct(Type),
    Alias(Type),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub docs: Vec<String>,
    pub name: Ident,
    pub ty: Type,
    pub attrs: Vec<Attribute>,
    pub default: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeParam {
    pub name: Ident,
    pub constraint: Constraint,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Constraint {
    None,
    /// `T: resource`.
    Resource,
    /// `T: Ordered + Hashable`.
    Interfaces(Vec<Type>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumDecl {
    pub name: Ident,
    pub type_params: Vec<TypeParam>,
    pub satisfies: Vec<Type>,
    pub variants: Vec<Variant>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub docs: Vec<String>,
    pub name: Ident,
    pub fields: Option<Vec<Field>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ErrorDecl {
    pub name: Ident,
    pub body: ErrorBody,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ErrorBody {
    /// `error NotFound { id: Id }`, `error Conflict`.
    Fields(Vec<Field>),
    /// `error OrderError = NotFound | sql.Error`.
    Set(Vec<Type>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceDecl {
    pub resource: bool,
    pub name: Ident,
    pub type_params: Vec<TypeParam>,
    pub members: Vec<InterfaceMember>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceMember {
    pub docs: Vec<String>,
    pub sig: FnSig,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActorDecl {
    pub name: Ident,
    pub type_params: Vec<TypeParam>,
    pub satisfies: Vec<Type>,
    pub fields: Vec<ActorField>,
    pub methods: Vec<ActorMethod>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActorField {
    pub docs: Vec<String>,
    pub mutable: bool,
    pub name: Ident,
    pub ty: Type,
    pub default: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActorMethod {
    pub docs: Vec<String>,
    pub public: bool,
    pub decl: FnDecl,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FnDecl {
    pub sig: FnSig,
    /// `None` for a signature in a fragment; `parse_file` requires a body.
    pub body: Option<Block>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FnSig {
    pub target: FnTarget,
    pub name: Ident,
    pub type_params: Vec<TypeParam>,
    pub params: Vec<Param>,
    pub ret: Option<Type>,
    pub errors: ErrorSpec,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FnTarget {
    /// `fn name(...)`.
    Free,
    /// `fn (self: var Order) name(...)`. The type is absent inside an interface.
    Method { kind: ParamKind, ty: Option<Type> },
    /// `fn Order.name(...)`, and `fn Self.name(...)` inside an interface.
    TypeLevel(Type),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ErrorSpec {
    /// No `!`: the function cannot fail.
    None,
    /// `!` with nothing after it: the set is inferred (Section 6.3).
    Inferred,
    Set(Vec<Type>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    Borrow,
    Var,
    Take,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: Ident,
    pub kind: ParamKind,
    pub variadic: bool,
    pub ty: Type,
    pub default: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConstDecl {
    pub name: Ident,
    pub ty: Option<Type>,
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Type {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeKind {
    /// `int`, `Order`, `sql.Pool`, `Cache<string, Order>`, `Reply.User`, `c.order`.
    /// The type arguments belong to the first type name in the path.
    Named {
        path: Vec<Ident>,
        args: Vec<Type>,
    },
    List(Box<Type>),
    Map(Box<Type>, Box<Type>),
    Array(Box<Type>, Box<Expr>),
    Tuple(Vec<Type>),
    Optional(Box<Type>),
    Fn {
        resource: bool,
        params: Vec<(ParamKind, Type)>,
        ret: Option<Box<Type>>,
        errors: Vec<Type>,
    },
    Struct(Vec<Field>),
    Result {
        ok: Box<Type>,
        errors: Vec<Type>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    /// `x := e`, `var x := e`, `x: T = e`, `(a, b) := e`.
    Let {
        mutable: bool,
        pattern: BindPattern,
        ty: Option<Type>,
        value: Expr,
    },
    Assign {
        target: Expr,
        op: AssignOp,
        value: Expr,
    },
    /// An expression statement, including `if` and `match` used as statements.
    Expr(Expr),
    Return(Option<Expr>),
    Fail(Expr),
    Break(Option<Ident>),
    Continue(Option<Ident>),
    While {
        label: Option<Ident>,
        cond: Cond,
        body: Block,
    },
    For {
        label: Option<Ident>,
        pattern: BindPattern,
        take: bool,
        iter: Expr,
        body: Block,
    },
    Scope(Block),
    Spawn(Block),
    Defer(Block),
    Unsafe(Block),
    Select(Vec<SelectArm>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    Set,
    Add,
    Sub,
    Mul,
    Div,
}

/// What `:=` and `for` bind: a name, `_`, or a tuple of them.
#[derive(Debug, Clone, PartialEq)]
pub enum BindPattern {
    Name(Ident),
    Wildcard(Span),
    Tuple(Vec<BindPattern>, Span),
}

/// The condition of `if` and `while`.
#[derive(Debug, Clone, PartialEq)]
pub enum Cond {
    Expr(Expr),
    /// `if user := find_user(email) { ... }`.
    Bind {
        pattern: BindPattern,
        value: Expr,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelectArm {
    pub op: SelectOp,
    pub body: Body,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SelectOp {
    After(Expr),
    Bind { pattern: BindPattern, value: Expr },
    Expr(Expr),
}

/// The body of a lambda, a `match` arm or a `select` arm, and the alternative of an `else`:
/// a block, or one simple statement (an expression, an assignment, `return`, `fail`,
/// `break` or `continue`).
#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    Block(Block),
    Stmt(Box<Stmt>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Int(String),
    Float(String),
    Decimal(String),
    Duration(String),
    Char(char),
    Str(Vec<StrPart>),
    RawStr(String),
    Bool(bool),
    None,
    /// A variable, function, package, type or variant name.
    Name(Ident),
    /// A type name with type arguments: `Channel<int>`, `cache.Cache<string, Order>`.
    TypeApp {
        base: Box<Expr>,
        args: Vec<Type>,
    },
    Field {
        base: Box<Expr>,
        name: Ident,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Arg>,
    },
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
    },
    /// `x?`.
    Try(Box<Expr>),
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Is {
        expr: Box<Expr>,
        ty: Type,
    },
    /// `x else alt` and `x else e => alt`.
    Else {
        expr: Box<Expr>,
        binding: Option<Ident>,
        alt: Box<Body>,
    },
    List(Vec<Expr>),
    Map(Vec<(Expr, Expr)>),
    Tuple(Vec<Expr>),
    Paren(Box<Expr>),
    StructLit {
        ty: Box<Expr>,
        fields: Vec<FieldInit>,
    },
    Lambda {
        params: Vec<LambdaParam>,
        body: Box<Body>,
    },
    If(Box<IfExpr>),
    Match {
        scrutinee: Box<Expr>,
        arms: Vec<MatchArm>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum StrPart {
    Lit(String),
    Interp { expr: Expr, format: Option<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Mul,
    Div,
    Rem,
    Add,
    Sub,
    Range,
    RangeInclusive,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

impl BinaryOp {
    pub fn as_str(self) -> &'static str {
        match self {
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Rem => "%",
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Range => "..",
            BinaryOp::RangeInclusive => "..=",
            BinaryOp::Eq => "==",
            BinaryOp::Ne => "!=",
            BinaryOp::Lt => "<",
            BinaryOp::Le => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::Ge => ">=",
            BinaryOp::And => "and",
            BinaryOp::Or => "or",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Arg {
    pub name: Option<Ident>,
    /// Whether the argument is marked `var` (Section 5.3).
    pub var: bool,
    /// Where `var` begins, when it is written.
    pub var_at: u32,
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldInit {
    pub name: Ident,
    /// `None` for the shorthand `Order { id }`.
    pub value: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LambdaParam {
    /// `None` for `_`.
    pub name: Option<Ident>,
    pub kind: ParamKind,
    pub ty: Option<Type>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IfExpr {
    pub cond: Cond,
    pub then: Block,
    pub otherwise: Option<ElseBranch>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ElseBranch {
    Block(Block),
    If(Box<IfExpr>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    /// Whether the arm begins with `!` and so matches errors (Section 6.4).
    pub error: bool,
    pub patterns: Vec<Pattern>,
    pub guard: Option<Expr>,
    pub body: Body,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pattern {
    pub kind: PatternKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PatternKind {
    Wildcard,
    None,
    Literal(Expr),
    Range {
        lo: Expr,
        hi: Expr,
        inclusive: bool,
    },
    /// `name` and `name: Type`.
    Bind {
        name: Ident,
        ty: Option<Type>,
    },
    /// `Cash`, `Card { brand: "amex", last4, .. }`, `sql.Error`.
    Ctor {
        path: Vec<Ident>,
        fields: Option<Vec<FieldPattern>>,
        rest: bool,
    },
    Tuple(Vec<Pattern>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldPattern {
    pub name: Ident,
    /// `None` for the shorthand that binds the field under its own name.
    pub pattern: Option<Pattern>,
}
