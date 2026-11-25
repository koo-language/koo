//! Chains of list methods made into one pass (Section 4.5).
//!
//! `xs.map(f).filter(g).fold(0, h)` means three passes and two lists that live only to be
//! read once. When nothing could tell the difference, it becomes one loop that builds no
//! list at all. What a program could see is the order in which `f`, `g` and `h` run: all
//! of `f`, then all of `g`, as written, or `f`, `g`, `h` for each element in turn. So a
//! chain is fused only when every function in it prints nothing, reads nothing from
//! outside, changes no variable, and calls nothing the compiler cannot see into. What is
//! left to see is a panic: which of two is reported, and, for `any`, `all` and `find`,
//! which stop at the first answer, whether one in a later element is reached at all.
//!
//! This runs after every body is checked, when every call says which function it calls.
//! A fused chain calls a function written here as Koo source, one for each shape of chain
//! the program uses, and checked like any other function of the prelude.

use std::collections::{BTreeSet, HashMap};

use koo_syntax::ast;

use super::Checker;
use super::generics::OwnerKey;
use crate::hir::*;

/// What a chain ends with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum End {
    /// Its last stage: the chain is a list.
    List,
    Fold,
    Any,
    All,
    Count,
    Find,
    Sum,
}

impl End {
    fn named(name: &str) -> Option<End> {
        Some(match name {
            "fold" => End::Fold,
            "any" => End::Any,
            "all" => End::All,
            "count" => End::Count,
            "find" => End::Find,
            "sum" => End::Sum,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            End::List => "list",
            End::Fold => "fold",
            End::Any => "any",
            End::All => "all",
            End::Count => "count",
            End::Find => "find",
            End::Sum => "sum",
        }
    }
}

/// The shape of a chain: for each stage whether it is `map` (or else `filter`), and the end.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Shape {
    maps: Vec<bool>,
    end: End,
}

impl Shape {
    fn name(&self) -> String {
        let stages: String = self.maps.iter().map(|&map| if map { 'm' } else { 'f' }).collect();
        format!("__fuse_{stages}_{}", self.end.name())
    }

    /// The function that does the whole chain in one loop, as Koo source.
    fn source(&self) -> String {
        let mut type_params = Vec::new();
        let mut params = Vec::new();
        let mut body = String::new();
        // The type and the name of the element as it stands after the stages so far.
        let (mut ty, mut value) = ("T".to_string(), "x0".to_string());
        for (index, &map) in self.maps.iter().enumerate() {
            if map {
                let next = format!("U{}", index + 1);
                params.push(format!("f{index}: fn({ty}) -> {next}"));
                let mapped = format!("x{}", index + 1);
                body.push_str(&format!("        {mapped} := f{index}({value})\n"));
                type_params.push(next.clone());
                (ty, value) = (next, mapped);
            } else {
                params.push(format!("f{index}: fn({ty}) -> bool"));
                body.push_str(&format!("        if not f{index}({value}) {{ continue }}\n"));
            }
        }
        let filters = self.maps.iter().any(|map| !map);
        let (ret, before, each, after) = match self.end {
            End::List => {
                let before = if filters {
                    format!("    var out: [{ty}] = []\n")
                } else {
                    format!("    var out := List<{ty}>.with_capacity(self.len)\n")
                };
                (format!("[{ty}]"), before, format!("out.push({value})"), "return out")
            }
            End::Fold => {
                type_params.push("A".to_string());
                params.push("start: A".to_string());
                params.push(format!("last: fn(A, {ty}) -> A"));
                (
                    "A".to_string(),
                    "    var acc := start\n".to_string(),
                    format!("acc = last(acc, {value})"),
                    "return acc",
                )
            }
            End::Any => {
                params.push(format!("last: fn({ty}) -> bool"));
                (
                    "bool".to_string(),
                    String::new(),
                    format!("if last({value}) {{ return true }}"),
                    "return false",
                )
            }
            End::All => {
                params.push(format!("last: fn({ty}) -> bool"));
                (
                    "bool".to_string(),
                    String::new(),
                    format!("if not last({value}) {{ return false }}"),
                    "return true",
                )
            }
            End::Count => {
                params.push(format!("last: fn({ty}) -> bool"));
                (
                    "int".to_string(),
                    "    var n := 0\n".to_string(),
                    format!("if last({value}) {{ n += 1 }}"),
                    "return n",
                )
            }
            End::Find => {
                params.push(format!("last: fn({ty}) -> bool"));
                (
                    format!("{ty}?"),
                    String::new(),
                    format!("if last({value}) {{ return {value} }}"),
                    "return none",
                )
            }
            End::Sum => (
                ty.clone(),
                format!("    var total := {ty}.zero()\n"),
                format!("total += {value}"),
                "return total",
            ),
        };
        let generics = if type_params.is_empty() {
            String::new()
        } else {
            format!("<{}>", type_params.join(", "))
        };
        let ret = if ret.is_empty() { ret } else { format!(" -> {ret}") };
        format!(
            "fn (self: [T]) {}{generics}({}){ret} {{\n{before}    for x0 in self {{\n{body}        {each}\n    }}\n    {after}\n}}\n\n",
            self.name(),
            params.join(", "),
        )
    }
}

/// What a function may do that someone outside it could notice.
#[derive(Debug, Clone, Default)]
struct Effects {
    /// Prints, reads or writes a file or a stream, reads the clock, or ends the program.
    io: bool,
    /// Calls a function value that is not one of its own parameters, or hands one on: what
    /// that does is not known.
    unknown: bool,
    /// Changes a variable, its own or not. Only asked of an expression.
    writes: bool,
    /// For a lambda: which of the variables it captured it may change, by their position
    /// among its captures.
    changed: BTreeSet<usize>,
    /// The functions it calls, names as a value, or makes a function value of.
    calls: Vec<FnId>,
    /// The lambdas written in it, with the variables each captures.
    lambdas: Vec<(FnId, Vec<LocalId>)>,
    /// Whether it makes text of a value whose type could have a `to_string` of its own.
    formats: bool,
}

/// One pass over a tree, outermost first.
trait Pass {
    fn stmt(&mut self, _stmt: &mut Stmt) {}
    fn expr(&mut self, _expr: &mut Expr) {}
}

fn walk_block(pass: &mut impl Pass, block: &mut Block) {
    for stmt in &mut block.stmts {
        pass.stmt(stmt);
        match stmt {
            Stmt::Let { init, .. } => walk_expr(pass, init),
            Stmt::Assign { target, value, .. } => {
                walk_expr(pass, target);
                walk_expr(pass, value);
            }
            Stmt::Expr(expr) => walk_expr(pass, expr),
            Stmt::While { cond, body } => {
                walk_cond(pass, cond);
                walk_block(pass, body);
            }
            Stmt::ForRange { lo, hi, body, .. } => {
                walk_expr(pass, lo);
                walk_expr(pass, hi);
                walk_block(pass, body);
            }
            Stmt::ForList { list, body, .. } => {
                walk_expr(pass, list);
                walk_block(pass, body);
            }
        }
    }
    if let Some(value) = &mut block.value {
        walk_expr(pass, value);
    }
}

fn walk_cond(pass: &mut impl Pass, cond: &mut Cond) {
    match cond {
        Cond::Expr(expr) => walk_expr(pass, expr),
        Cond::Bind { value, .. } => walk_expr(pass, value),
    }
}

fn walk_expr(pass: &mut impl Pass, expr: &mut Expr) {
    pass.expr(expr);
    match &mut expr.kind {
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::Str(_)
        | ExprKind::Unit
        | ExprKind::Local(_)
        | ExprKind::Closure { .. }
        | ExprKind::FnRef(_)
        | ExprKind::Location
        | ExprKind::NoneLit
        | ExprKind::Break
        | ExprKind::Continue
        | ExprKind::Return(None) => {}
        ExprKind::Field(inner, _)
        | ExprKind::Unary(_, inner)
        | ExprKind::WrapSome(inner)
        | ExprKind::WrapOk(inner)
        | ExprKind::Retype(inner)
        | ExprKind::Try(inner)
        | ExprKind::Fail(inner)
        | ExprKind::Panic(inner)
        | ExprKind::Return(Some(inner)) => walk_expr(pass, inner),
        ExprKind::Index(a, b) | ExprKind::Binary(_, a, b) | ExprKind::And(a, b) | ExprKind::Or(a, b) => {
            walk_expr(pass, a);
            walk_expr(pass, b);
        }
        ExprKind::Call { args, .. } => {
            for arg in args {
                walk_expr(pass, &mut arg.expr);
            }
        }
        ExprKind::CallValue { callee, args } => {
            walk_expr(pass, callee);
            for arg in args {
                walk_expr(pass, &mut arg.expr);
            }
        }
        ExprKind::Builtin(_, values)
        | ExprKind::Intrinsic { args: values, .. }
        | ExprKind::StructLit(_, values)
        | ExprKind::EnumLit(_, _, values)
        | ExprKind::ErrorLit(_, values)
        | ExprKind::ListLit(values)
        | ExprKind::TupleLit(values) => {
            for value in values {
                walk_expr(pass, value);
            }
        }
        ExprKind::Interp(parts) => {
            for part in parts {
                if let InterpPart::Value(value, _) = part {
                    walk_expr(pass, value);
                }
            }
        }
        ExprKind::Else { expr, alt, .. } => {
            walk_expr(pass, expr);
            walk_block(pass, alt);
        }
        ExprKind::If { cond, then, otherwise } => {
            walk_cond(pass, cond);
            walk_block(pass, then);
            if let Some(otherwise) = otherwise {
                walk_block(pass, otherwise);
            }
        }
        ExprKind::Match { scrutinee, arms } => {
            walk_expr(pass, scrutinee);
            for arm in arms {
                if let Some(guard) = &mut arm.guard {
                    walk_expr(pass, guard);
                }
                walk_block(pass, &mut arm.body);
            }
        }
    }
}

/// The variable at the bottom of a place.
fn root(place: &Expr) -> Option<LocalId> {
    match &place.kind {
        ExprKind::Local(local) => Some(*local),
        ExprKind::Field(base, _) | ExprKind::Index(base, _) | ExprKind::Retype(base) => root(base),
        _ => None,
    }
}

fn is_function(ty: &Ty) -> bool {
    matches!(ty, Ty::Fn(_))
}

/// Whether text can be made of a value of this type without calling into the program.
fn plain_text(ty: &Ty) -> bool {
    matches!(ty, Ty::Int | Ty::Float | Ty::Bool | Ty::Byte | Ty::Str | Ty::Unit)
}

/// Whether a function of the support library only computes.
fn computes(intrinsic: &str) -> bool {
    intrinsic.starts_with("str_") || matches!(intrinsic, "bytes_to_str" | "args" | "env" | "env_has")
}

/// Gathers what one body does. `locals` and `captures` are those of its function.
struct Gather<'p> {
    locals: &'p [Local],
    captures: &'p [LocalId],
    found: Effects,
}

impl Gather<'_> {
    /// A place is changed.
    fn change(&mut self, place: &Expr) {
        self.found.writes = true;
        let Some(local) = root(place) else { return };
        if matches!(self.locals[local].kind, LocalKind::Capture { by_ref: true })
            && let Some(position) = self.captures.iter().position(|capture| *capture == local)
        {
            self.found.changed.insert(position);
        }
    }

    /// A function value is passed on or called. A lambda and a named function are found
    /// where they are written. A parameter is the caller's business: whoever passed it
    /// answers for it. Of anything else nothing is known.
    fn function_value(&mut self, value: &Expr) {
        match &value.kind {
            ExprKind::Closure { .. } | ExprKind::FnRef(_) => {}
            ExprKind::Retype(inner) => self.function_value(inner),
            ExprKind::Local(local) if matches!(self.locals[*local].kind, LocalKind::Param(_)) => {}
            _ => self.found.unknown = true,
        }
    }

    fn arguments(&mut self, args: &[Arg]) {
        for arg in args {
            // `var` changes the argument, and `take` empties the variable it names.
            if arg.kind != ParamKind::Borrow {
                self.change(&arg.expr);
            }
            if is_function(&arg.expr.ty) {
                self.function_value(&arg.expr);
            }
        }
    }
}

impl Pass for Gather<'_> {
    fn stmt(&mut self, stmt: &mut Stmt) {
        if let Stmt::Assign { target, .. } = stmt {
            self.change(target);
        }
    }

    fn expr(&mut self, expr: &mut Expr) {
        match &expr.kind {
            ExprKind::Call { func, args } => {
                self.found.calls.push(*func);
                self.arguments(args);
            }
            ExprKind::CallValue { callee, args } => {
                self.function_value(callee);
                self.arguments(args);
            }
            ExprKind::Closure { func, captures, .. } => {
                self.found.calls.push(*func);
                self.found.lambdas.push((*func, captures.clone()));
            }
            ExprKind::FnRef(func) => self.found.calls.push(*func),
            ExprKind::Builtin(builtin, args) => match builtin {
                Builtin::Print => self.found.io = true,
                Builtin::ListPush
                | Builtin::ListPop
                | Builtin::ListSwap
                | Builtin::ListClear
                | Builtin::ListSortInts => {
                    self.change(&args[0]);
                }
                Builtin::Repr if !plain_text(&args[0].ty) => self.found.formats = true,
                _ => {}
            },
            ExprKind::Intrinsic { name, .. } if !computes(name) => self.found.io = true,
            ExprKind::Interp(parts) => {
                for part in parts {
                    if let InterpPart::Value(value, _) = part
                        && !plain_text(&value.ty)
                    {
                        self.found.formats = true;
                    }
                }
            }
            _ => {}
        }
    }
}

/// A chain found in a body: what `plan` decided, for `Rewrite` to carry out.
struct Chain {
    shape: Shape,
    /// The type arguments of the function for the shape: the element type, the type each
    /// `map` gives, and for `fold` the type of the result.
    type_args: Vec<Ty>,
}

impl<'a> Checker<'a> {
    /// Fuses the chains of the program. Every body is checked by now.
    pub(super) fn fuse_chains(&mut self) {
        let Some(lists) = ListMethods::find(self) else { return };
        let effects = self.effects();

        // First which shapes are needed, then the functions for them, then the calls.
        let bodies: Vec<FnId> = (0..self.program.functions.len())
            .filter(|&id| !self.program.functions[id].prelude)
            .collect();
        let mut shapes = BTreeSet::new();
        for &id in &bodies {
            let mut body = std::mem::take(&mut self.program.functions[id].body);
            let mut find = Find {
                checker: self,
                lists: &lists,
                effects: &effects,
                function: id,
                shapes: &mut shapes,
            };
            walk_block(&mut find, &mut body);
            self.program.functions[id].body = body;
        }
        if shapes.is_empty() {
            return;
        }
        let text: String = shapes.iter().map(Shape::source).collect();
        let parsed = koo_syntax::parse_file(&text);
        assert!(
            parsed.diagnostics.is_empty(),
            "a fused chain does not parse: {:?}\n{text}",
            parsed.diagnostics
        );
        let file: &'a ast::File = self.extra.get_or_init(|| parsed.file);
        self.in_prelude = true;
        for item in &file.items {
            if let ast::ItemKind::Fn(decl) = &item.kind {
                self.declare_template(decl);
            }
        }
        self.in_prelude = false;

        for &id in &bodies {
            let mut body = std::mem::take(&mut self.program.functions[id].body);
            let mut rewrite = Rewrite {
                checker: self,
                lists: &lists,
                effects: &effects,
                function: id,
            };
            walk_block(&mut rewrite, &mut body);
            self.program.functions[id].body = body;
        }
        self.check_pending();
    }

    /// What every function of the program may do, its callees included.
    fn effects(&mut self) -> Vec<Effects> {
        let count = self.program.functions.len();
        let mut effects = Vec::with_capacity(count);
        for function in &mut self.program.functions {
            let mut body = std::mem::take(&mut function.body);
            let mut gather = Gather {
                locals: &function.locals,
                captures: &function.captures,
                found: Effects::default(),
            };
            walk_block(&mut gather, &mut body);
            effects.push(gather.found);
            function.body = body;
        }
        let to_strings: Vec<FnId> = self.program.to_string.values().copied().collect();
        for found in &mut effects {
            if found.formats {
                found.calls.extend(&to_strings);
            }
        }
        // What a function calls, it does. A lambda that changes a variable it captured
        // changes one of the function it is written in; if that function captured the
        // variable in turn, the change goes further out.
        loop {
            let mut grew = false;
            for id in 0..count {
                let (mut io, mut unknown) = (effects[id].io, effects[id].unknown);
                for &callee in &effects[id].calls {
                    io |= effects[callee].io;
                    unknown |= effects[callee].unknown;
                }
                let mut changed = effects[id].changed.clone();
                let function = &self.program.functions[id];
                for (lambda, captured) in &effects[id].lambdas {
                    for &position in &effects[*lambda].changed {
                        let local = captured[position];
                        if matches!(function.locals[local].kind, LocalKind::Capture { by_ref: true })
                            && let Some(outer) = function.captures.iter().position(|capture| *capture == local)
                        {
                            changed.insert(outer);
                        }
                    }
                }
                if io != effects[id].io || unknown != effects[id].unknown || changed != effects[id].changed {
                    effects[id].io = io;
                    effects[id].unknown = unknown;
                    effects[id].changed = changed;
                    grew = true;
                }
            }
            if !grew {
                return effects;
            }
        }
    }

    /// Whether the expression is a chain that may be fused, and how.
    fn plan(&self, expr: &Expr, lists: &ListMethods, effects: &[Effects], function: FnId) -> Option<Chain> {
        let (name, args) = lists.call(expr)?;
        let (end, mut link) = match name {
            "map" | "filter" => (End::List, expr),
            other => (End::named(other)?, &args[0].expr),
        };
        // The stages, from the last to the first, then the list they begin with.
        let mut stages = Vec::new();
        while let Some((name @ ("map" | "filter"), args)) = lists.call(link) {
            stages.push((name == "map", link, &args[1].expr));
            link = &args[0].expr;
        }
        stages.reverse();
        // One stage and an end, or two stages: anything less builds no list to save.
        if stages.len() + usize::from(end != End::List) < 2 {
            return None;
        }
        let Ty::List(element) = &link.ty else { return None };
        let mut type_args = vec![(**element).clone()];
        for (map, stage, by) in &stages {
            if !self.silent(by, effects) {
                return None;
            }
            if *map {
                let Ty::List(mapped) = &stage.ty else { return None };
                type_args.push((**mapped).clone());
            }
        }
        if end != End::List {
            // The last function runs between the steps of the stages before it, so it
            // is held to what they are: a panic in a later element must not find the
            // work of the earlier ones already printed.
            let last = args.len() - 1;
            if end != End::Sum && !self.silent(&args[last].expr, effects) {
                return None;
            }
            if end == End::Fold {
                if !self.computes_only(&args[1].expr, effects, function) {
                    return None;
                }
                type_args.push(expr.ty.clone());
            }
        }
        let fits = |ty: &Ty| {
            !matches!(ty, Ty::Unit | Ty::Never | Ty::Unknown) && self.ty_depth(ty) < super::generics::MAX_TYPE_DEPTH
        };
        if !type_args.iter().all(fits) {
            return None;
        }
        Some(Chain {
            shape: Shape {
                maps: stages.iter().map(|(map, ..)| *map).collect(),
                end,
            },
            type_args,
        })
    }

    /// Whether a function given to a chain is a lambda or a named function that changes
    /// nothing outside itself, calls nothing unknown and does no input or output.
    fn silent(&self, value: &Expr, effects: &[Effects]) -> bool {
        let func = match &value.kind {
            ExprKind::Closure { func, block: true, .. } => *func,
            ExprKind::FnRef(func) => *func,
            _ => return false,
        };
        let does = &effects[func];
        !does.unknown && does.changed.is_empty() && !does.io
    }

    /// Whether an expression of `function` only computes a value: it could be evaluated
    /// before or after anything else with the same result.
    fn computes_only(&self, expr: &Expr, effects: &[Effects], function: FnId) -> bool {
        let function = &self.program.functions[function];
        let mut gather = Gather {
            locals: &function.locals,
            captures: &function.captures,
            found: Effects::default(),
        };
        // The walk is one for trees that may be changed; this one is only read.
        let mut copy = expr.clone();
        walk_expr(&mut gather, &mut copy);
        let found = gather.found;
        !(found.io || found.unknown || found.writes || found.formats)
            && found.calls.iter().all(|&callee| {
                let does = &effects[callee];
                !does.io && !does.unknown && does.changed.is_empty()
            })
    }

    /// The function for a chain, made for its types.
    fn fused(&mut self, chain: &Chain, span: koo_syntax::Span) -> Option<FnId> {
        let template = *self.generic_methods.get(&(OwnerKey::List, chain.shape.name()))?;
        self.instantiate(template, chain.type_args.clone(), span)
    }
}

/// The methods of lists that a chain is made of: which function is which.
struct ListMethods {
    /// For each instance of one of them: its name.
    names: HashMap<FnId, &'static str>,
}

impl ListMethods {
    fn find(checker: &Checker) -> Option<ListMethods> {
        const NAMES: [&str; 8] = ["map", "filter", "fold", "any", "all", "count", "find", "sum"];
        let mut templates = HashMap::new();
        for name in NAMES {
            let template = checker.generic_methods.get(&(OwnerKey::List, name.to_string()))?;
            templates.insert(*template, name);
        }
        let mut names = HashMap::new();
        for ((template, _), id) in &checker.fn_instances {
            if let Some(name) = templates.get(template) {
                names.insert(*id, *name);
            }
        }
        Some(ListMethods { names })
    }

    /// If the expression calls one of the methods: which, and with what. The first
    /// argument is the list.
    fn call<'e>(&self, expr: &'e Expr) -> Option<(&'static str, &'e [Arg])> {
        let ExprKind::Call { func, args } = &expr.kind else {
            return None;
        };
        let name = self.names.get(func)?;
        Some((name, args))
    }
}

/// Notes the shapes of the chains that will be fused.
struct Find<'c, 'a> {
    checker: &'c Checker<'a>,
    lists: &'c ListMethods,
    effects: &'c [Effects],
    function: FnId,
    shapes: &'c mut BTreeSet<Shape>,
}

impl Pass for Find<'_, '_> {
    fn expr(&mut self, expr: &mut Expr) {
        if let Some(chain) = self.checker.plan(expr, self.lists, self.effects, self.function) {
            self.shapes.insert(chain.shape);
        }
    }
}

/// Replaces each chain that may be fused by one call.
struct Rewrite<'c, 'a> {
    checker: &'c mut Checker<'a>,
    lists: &'c ListMethods,
    effects: &'c [Effects],
    function: FnId,
}

impl Pass for Rewrite<'_, '_> {
    fn expr(&mut self, expr: &mut Expr) {
        let Some(chain) = self.checker.plan(expr, self.lists, self.effects, self.function) else {
            return;
        };
        let Some(func) = self.checker.fused(&chain, expr.span) else {
            return;
        };
        // Take the chain apart, outermost call first: each gives up its arguments after
        // the list, and the list of the innermost is the one the loop reads.
        let mut calls = Vec::new();
        let mut link = std::mem::replace(&mut expr.kind, ExprKind::Unit);
        let stages = chain.shape.maps.len();
        let links = stages + usize::from(chain.shape.end != End::List);
        let mut list = None;
        for _ in 0..links {
            let ExprKind::Call { args, .. } = link else {
                unreachable!("a chain is made of calls")
            };
            let mut args = args.into_iter();
            let receiver = args.next().expect("a method has a receiver");
            calls.push(args.collect::<Vec<Arg>>());
            link = receiver.expr.kind.clone();
            list = Some(receiver);
        }
        let mut args = vec![list.expect("a chain has a call")];
        for call in calls.into_iter().rev() {
            args.extend(call);
        }
        expr.kind = ExprKind::Call { func, args };
    }
}

/// The names of the functions that fused chains call, for a test of the texts.
#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn every_shape_is_a_function_that_parses() {
        let ends = [
            End::List,
            End::Fold,
            End::Any,
            End::All,
            End::Count,
            End::Find,
            End::Sum,
        ];
        let mut seen = HashSet::new();
        for end in ends {
            for maps in [vec![true], vec![false], vec![true, false], vec![false, true, true]] {
                let shape = Shape { maps, end };
                let text = shape.source();
                let parsed = koo_syntax::parse_file(&text);
                assert!(parsed.diagnostics.is_empty(), "{:?}\n{text}", parsed.diagnostics);
                assert!(text.contains(&shape.name()));
                assert!(seen.insert(shape.name()));
            }
        }
    }
}
