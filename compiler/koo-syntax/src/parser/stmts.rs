//! Blocks and statements.

use super::*;

impl Parser {
    /// A block whose statements are checked as statements.
    pub(super) fn block(&mut self) -> PResult<Block> {
        let block = self.block_unchecked()?;
        self.check_stmts(&block);
        Ok(block)
    }

    /// Checks the statements of a block. An `if` that is a statement is checked here and not
    /// where it is parsed, because only its place in a block says what its blocks may hold.
    pub(super) fn check_stmts(&mut self, block: &Block) {
        for stmt in &block.stmts {
            self.check_stmt(stmt);
        }
    }

    pub(super) fn check_stmt(&mut self, stmt: &Stmt) {
        match &stmt.kind {
            StmtKind::Expr(Expr {
                kind: ExprKind::If(if_expr),
                ..
            }) => self.check_if(if_expr, false),
            _ => self.check_expr_stmt(stmt),
        }
    }

    /// Checks a block of an `if` that is used for its value (Section 6.6): the block is
    /// either one expression of any form, or statements that end by leaving.
    fn check_value_block(&mut self, block: &Block) {
        match block.stmts.as_slice() {
            [
                Stmt {
                    kind: StmtKind::Expr(expr),
                    ..
                },
            ] => {
                if let ExprKind::If(if_expr) = &expr.kind {
                    self.check_if(if_expr, true);
                }
            }
            _ => self.check_stmts(block),
        }
    }

    /// Checks the blocks of an `if`. `value` says whether the `if` is used for its value.
    pub(super) fn check_if(&mut self, if_expr: &IfExpr, value: bool) {
        let mut current = Some(if_expr);
        while let Some(if_expr) = current {
            current = None;
            let blocks = [
                Some(&if_expr.then),
                match &if_expr.otherwise {
                    Some(ElseBranch::Block(block)) => Some(block),
                    Some(ElseBranch::If(next)) => {
                        current = Some(next);
                        None
                    }
                    None => None,
                },
            ];
            for block in blocks.into_iter().flatten() {
                if value {
                    self.check_value_block(block);
                } else {
                    self.check_stmts(block);
                }
            }
        }
    }

    pub(super) fn block_unchecked(&mut self) -> PResult<Block> {
        self.nested(|p| p.block_unguarded())
    }

    fn block_unguarded(&mut self) -> PResult<Block> {
        let start = self.expect(&Tok::LBrace, "`{`")?.span;
        let saved = self.enter(true);
        let depth = self.nl_sig.len();
        let mut stmts = Vec::new();
        loop {
            self.skip_newlines();
            if self.at(&Tok::RBrace) {
                break;
            }
            if self.at(&Tok::Eof) {
                self.report(start, "this `{` is never closed");
                return Err(());
            }
            let before = self.pos;
            match self.stmt() {
                Ok(stmt) => {
                    stmts.push(stmt);
                    if !self.at_stmt_end() {
                        let _ = self.expected::<()>("the end of the line");
                        self.recover(depth, true);
                    }
                }
                Err(()) => self.recover(depth, true),
            }
            if self.pos == before && !self.at(&Tok::RBrace) {
                self.bump();
            } else if !self.at_stmt_end() {
                // Recovery stopped at a stray `)` or `]`.
                self.bump();
            }
        }
        self.bump();
        self.leave(saved);
        Ok(Block {
            stmts,
            span: start.to(self.prev_span()),
        })
    }

    /// An expression used as a statement must be a call, or end in `?` or in an `else`
    /// alternative (Section 3.7). `if` and `match` are statements in their own right.
    pub(super) fn check_expr_stmt(&mut self, stmt: &Stmt) {
        let StmtKind::Expr(expr) = &stmt.kind else {
            return;
        };
        if !matches!(
            expr.kind,
            ExprKind::Call { .. } | ExprKind::Try(_) | ExprKind::Else { .. } | ExprKind::If(_) | ExprKind::Match { .. }
        ) {
            self.report_help(
                expr.span,
                "this expression is not a statement",
                "a statement is a call, or ends in `?` or `else`; to use the value, bind it with `:=` or `return` it",
            );
        }
    }

    pub(super) fn stmt(&mut self) -> PResult<Stmt> {
        let start = self.span();
        let kind = match self.peek() {
            Tok::Kw(Kw::Var) => {
                self.bump();
                self.let_stmt(true)?
            }
            Tok::Kw(Kw::Return | Kw::Fail | Kw::Break | Kw::Continue) => self.jump()?,
            Tok::Kw(Kw::If) => {
                let if_expr = self.if_expr()?;
                StmtKind::Expr(Expr {
                    kind: ExprKind::If(Box::new(if_expr)),
                    span: start.to(self.prev_span()),
                })
            }
            Tok::Kw(Kw::Match) => StmtKind::Expr(self.match_expr()?),
            Tok::Kw(Kw::While) => self.while_stmt(None)?,
            Tok::Kw(Kw::For) => self.for_stmt(None)?,
            Tok::Kw(Kw::Scope) => {
                self.bump();
                StmtKind::Scope(self.block()?)
            }
            Tok::Kw(Kw::Spawn) => {
                self.bump();
                StmtKind::Spawn(self.block()?)
            }
            Tok::Kw(Kw::Defer) => {
                self.bump();
                StmtKind::Defer(self.block()?)
            }
            Tok::Kw(Kw::Unsafe) => {
                self.bump();
                StmtKind::Unsafe(self.block()?)
            }
            Tok::Kw(Kw::Select) => self.select_stmt()?,
            Tok::Kw(
                Kw::Fn
                | Kw::Type
                | Kw::Enum
                | Kw::Error
                | Kw::Interface
                | Kw::Actor
                | Kw::Resource
                | Kw::Import
                | Kw::Pub,
            ) => {
                let span = self.span();
                self.report_help(
                    span,
                    "a declaration cannot appear inside a block",
                    "types and functions are declared at the top level of a file; a local function is a lambda bound to a name",
                );
                return Err(());
            }
            Tok::Ident(_) if *self.peek2() == Tok::Colon => {
                // `label: for ...` and `label: while ...`, or a typed binding. A keyword
                // follows the colon of a label (Section 3.9).
                match self.raw(2) {
                    Tok::Kw(Kw::For) => {
                        let label = self.label()?;
                        self.for_stmt(Some(label))?
                    }
                    Tok::Kw(Kw::While) => {
                        let label = self.label()?;
                        self.while_stmt(Some(label))?
                    }
                    _ => self.let_stmt(false)?,
                }
            }
            _ if self.at_binding() => self.let_stmt(false)?,
            _ => self.simple_stmt(true)?,
        };
        Ok(Stmt {
            kind,
            span: start.to(self.prev_span()),
        })
    }

    fn label(&mut self) -> PResult<Ident> {
        let label = self.binding_name("label")?;
        self.bump();
        Ok(label)
    }

    /// Whether a `:=` binding begins here: `name :=`, `_ :=` or `(a, b) :=`.
    pub(super) fn at_binding(&self) -> bool {
        match self.peek() {
            Tok::Ident(_) | Tok::UpperIdent(_) | Tok::Underscore => *self.peek2() == Tok::ColonEq,
            Tok::LParen => self.paren_then(&Tok::ColonEq),
            _ => false,
        }
    }

    pub(super) fn bind_pattern(&mut self) -> PResult<BindPattern> {
        self.nested(|p| p.bind_pattern_unguarded())
    }

    fn bind_pattern_unguarded(&mut self) -> PResult<BindPattern> {
        match self.peek() {
            Tok::Underscore => Ok(BindPattern::Wildcard(self.bump().span)),
            Tok::LParen => {
                let start = self.span();
                let parts = self.delimited(&Tok::LParen, &Tok::RParen, |p| p.bind_pattern())?;
                let span = start.to(self.prev_span());
                if parts.len() < 2 {
                    self.report_help(
                        span,
                        "a tuple pattern names at least two elements",
                        "tuples have two or more elements; `()` is the empty tuple (Section 4.5)",
                    );
                }
                Ok(BindPattern::Tuple(parts, span))
            }
            _ => Ok(BindPattern::Name(self.binding_name("variable name")?)),
        }
    }

    /// `x := e`, `x: T = e`, `(a, b) := e`, after any `var`.
    fn let_stmt(&mut self, mutable: bool) -> PResult<StmtKind> {
        let pattern = self.bind_pattern()?;
        if self.eat(&Tok::ColonEq) {
            let value = self.expr()?;
            return Ok(StmtKind::Let {
                mutable,
                pattern,
                ty: None,
                value,
            });
        }
        if !matches!(pattern, BindPattern::Name(_)) {
            return self.expected("`:=`");
        }
        self.expect(&Tok::Colon, "`:=`, or `:` and a type")?;
        let ty = self.ty()?;
        if !self.at(&Tok::Eq) {
            return self.expected("`=` and a value: a binding is always given a value");
        }
        self.bump();
        let value = self.expr()?;
        Ok(StmtKind::Let {
            mutable,
            pattern,
            ty: Some(ty),
            value,
        })
    }

    /// `return`, `fail`, `break` and `continue`.
    fn jump(&mut self) -> PResult<StmtKind> {
        let token = self.bump();
        let ends = |p: &Parser| {
            matches!(
                p.peek(),
                Tok::Newline | Tok::RBrace | Tok::RParen | Tok::RBracket | Tok::Comma | Tok::Eof
            )
        };
        Ok(match token.kind {
            Tok::Kw(Kw::Return) => StmtKind::Return(if ends(self) { None } else { Some(self.expr()?) }),
            Tok::Kw(Kw::Fail) => StmtKind::Fail(self.expr()?),
            Tok::Kw(Kw::Break) => StmtKind::Break(if matches!(self.peek(), Tok::Ident(_)) {
                Some(self.lower_name("label")?)
            } else {
                None
            }),
            _ => StmtKind::Continue(if matches!(self.peek(), Tok::Ident(_)) {
                Some(self.lower_name("label")?)
            } else {
                None
            }),
        })
    }

    /// An expression statement or an assignment.
    fn simple_stmt(&mut self, allow_assign: bool) -> PResult<StmtKind> {
        let expr = self.expr()?;
        let op = match self.peek() {
            Tok::Eq => AssignOp::Set,
            Tok::PlusEq => AssignOp::Add,
            Tok::MinusEq => AssignOp::Sub,
            Tok::StarEq => AssignOp::Mul,
            Tok::SlashEq => AssignOp::Div,
            _ => return Ok(StmtKind::Expr(expr)),
        };
        let op_span = self.span();
        if !allow_assign {
            return self.fail(op_span, "an assignment cannot appear here");
        }
        if !is_place(&expr) {
            self.report_help(
                expr.span,
                "this cannot be assigned to",
                "the left of an assignment is a variable, a field of one, or an element of one",
            );
        }
        self.bump();
        let value = self.expr()?;
        Ok(StmtKind::Assign {
            target: expr,
            op,
            value,
        })
    }

    /// The body of a lambda, a `match` arm or a `select` arm, and the alternative of an
    /// `else`: a block, or one simple statement.
    pub(super) fn body(&mut self, allow_assign: bool) -> PResult<Body> {
        if self.at(&Tok::LBrace) {
            return Ok(Body::Block(self.block()?));
        }
        let start = self.span();
        let kind = match self.peek() {
            Tok::Kw(Kw::Return | Kw::Fail | Kw::Break | Kw::Continue) => self.jump()?,
            _ => self.simple_stmt(allow_assign)?,
        };
        Ok(Body::Stmt(Box::new(Stmt {
            kind,
            span: start.to(self.prev_span()),
        })))
    }

    /// The condition of `if` and `while`. A struct literal in it must be parenthesized
    /// (Section 3.9).
    pub(super) fn cond(&mut self) -> PResult<Cond> {
        let saved = std::mem::replace(&mut self.struct_lit_ok, false);
        let result = if self.at_binding() {
            self.bind_pattern().and_then(|pattern| {
                self.bump();
                Ok(Cond::Bind {
                    pattern,
                    value: self.expr()?,
                })
            })
        } else {
            self.expr().map(Cond::Expr)
        };
        self.struct_lit_ok = saved;
        result
    }

    /// An expression that ends at `=>`: a `match` guard or a `select` operand. An `else` in
    /// it does not bind an error with `name =>`, unless parenthesized (Section 3.9).
    pub(super) fn expr_before_arrow(&mut self) -> PResult<Expr> {
        let saved = std::mem::replace(&mut self.else_bind_ok, false);
        let result = self.expr();
        self.else_bind_ok = saved;
        result
    }

    /// An expression before a block: a `match` scrutinee or a `for` subject.
    pub(super) fn expr_before_block(&mut self) -> PResult<Expr> {
        let saved = std::mem::replace(&mut self.struct_lit_ok, false);
        let result = self.expr();
        self.struct_lit_ok = saved;
        result
    }

    fn while_stmt(&mut self, label: Option<Ident>) -> PResult<StmtKind> {
        self.expect_kw(Kw::While)?;
        let cond = self.cond()?;
        let body = self.block()?;
        Ok(StmtKind::While { label, cond, body })
    }

    fn for_stmt(&mut self, label: Option<Ident>) -> PResult<StmtKind> {
        self.expect_kw(Kw::For)?;
        let pattern = self.bind_pattern()?;
        self.expect_kw(Kw::In)?;
        let take = self.eat_kw(Kw::Take);
        let iter = self.expr_before_block()?;
        let body = self.block()?;
        Ok(StmtKind::For {
            label,
            pattern,
            take,
            iter,
            body,
        })
    }

    fn select_stmt(&mut self) -> PResult<StmtKind> {
        self.bump();
        self.expect(&Tok::LBrace, "`{`")?;
        let saved = self.enter(true);
        let mut arms = Vec::new();
        loop {
            self.skip_newlines();
            if self.at(&Tok::RBrace) {
                break;
            }
            let start = self.span();
            let op = if *self.peek() == Tok::Ident("after".to_string()) {
                // `after` is reserved at the start of a `select` arm (Section 3.2).
                self.bump();
                SelectOp::After(self.expr_before_arrow()?)
            } else if self.at_binding() {
                let pattern = self.bind_pattern()?;
                self.bump();
                SelectOp::Bind {
                    pattern,
                    value: self.expr_before_arrow()?,
                }
            } else {
                SelectOp::Expr(self.expr_before_arrow()?)
            };
            self.expect(&Tok::FatArrow, "`=>`")?;
            let body = self.body(true)?;
            arms.push(SelectArm {
                op,
                body,
                span: start.to(self.prev_span()),
            });
            if !matches!(self.peek(), Tok::Newline | Tok::RBrace) {
                return self.expected("the end of the line");
            }
        }
        self.bump();
        self.leave(saved);
        Ok(StmtKind::Select(arms))
    }
}

/// Whether an expression is a place: a variable, or a field or element of a place.
fn is_place(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Name(name) => !name.is_upper(),
        ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => is_place(base),
        _ => false,
    }
}
