//! Checking calls: user functions, methods, and the built-in functions and methods.

use std::collections::HashMap;

use koo_syntax::Span;
use koo_syntax::ast;
use koo_syntax::token::is_primitive;

use super::{Checker, Owner, TypeName};
use crate::hir::*;

/// One step from a variable to a part of it.
#[derive(PartialEq)]
enum Step {
    Field(usize),
    Index,
}

/// The variable a place belongs to, and the path from it to the place.
fn place_path(expr: &Expr) -> Option<(LocalId, Vec<Step>)> {
    match &expr.kind {
        ExprKind::Local(id) => Some((*id, Vec::new())),
        ExprKind::Field(base, index) => {
            let (root, mut path) = place_path(base)?;
            path.push(Step::Field(*index));
            Some((root, path))
        }
        ExprKind::Index(base, _) => {
            let (root, mut path) = place_path(base)?;
            path.push(Step::Index);
            Some((root, path))
        }
        _ => None,
    }
}

/// Whether two places may name the same storage (Section 5.4): the same variable, and no
/// step at which they take different fields.
fn overlap(a: &(LocalId, Vec<Step>), b: &(LocalId, Vec<Step>)) -> bool {
    a.0 == b.0
        && !a
            .1
            .iter()
            .zip(&b.1)
            .any(|(x, y)| matches!((x, y), (Step::Field(i), Step::Field(j)) if i != j))
}

impl<'a> Checker<'a> {
    fn mk_call(&self, kind: ExprKind, ty: Ty, span: Span) -> Expr {
        Expr { kind, ty, span }
    }

    /// Checks that a built-in was given `count` plain arguments, and returns them unchecked.
    fn plain_args<'e>(
        &mut self,
        args: &'e [ast::Arg],
        count: usize,
        what: &str,
        span: Span,
    ) -> Option<Vec<&'e ast::Expr>> {
        for arg in args {
            if let Some(name) = &arg.name {
                self.error(name.span, format!("{what} takes no named arguments"));
            }
            if arg.var {
                self.error(arg.value.span, format!("{what} takes no `var` arguments"));
            }
        }
        if args.len() != count {
            let takes = match count {
                0 => "no arguments".to_string(),
                1 => "one argument".to_string(),
                n => format!("{n} arguments"),
            };
            self.error(span, format!("{what} takes {takes}, found {}", args.len()));
            for arg in args {
                self.expr(&arg.value, None);
            }
            return None;
        }
        Some(args.iter().map(|a| &a.value).collect())
    }

    pub(super) fn call(&mut self, callee: &ast::Expr, args: &[ast::Arg], span: Span, expected: Option<&Ty>) -> Expr {
        // `Point(1, 2)`: a struct made the way another language calls a constructor.
        if let ast::ExprKind::Name(name) = &callee.kind
            && name.is_upper()
            && let Some(fields) = match self.type_names.get(&name.name) {
                Some(TypeName::Struct(id)) => Some(&self.program.structs[*id].fields),
                Some(TypeName::Error(id)) => Some(&self.program.errors[*id].fields),
                _ => None,
            }
            && !fields.is_empty()
        {
            let names: Vec<String> = fields.iter().map(|field| field.name.clone()).collect();
            self.error(callee.span, format!("`{}` is a type, not a function", name.name));
            self.note("a value of a type is made by naming its fields");
            if names.len() == args.len() && args.iter().all(|arg| arg.name.is_none() && !arg.var) {
                // `(` becomes ` { x: `, each further argument gets its field, `)` becomes ` }`.
                let mut edits = Vec::new();
                let mut from = callee.span.end;
                for (arg, field) in args.iter().zip(&names) {
                    let lead = if from == callee.span.end { " { " } else { ", " };
                    edits.push((
                        Span {
                            start: from,
                            end: arg.value.span.start,
                        },
                        format!("{lead}{field}: "),
                    ));
                    from = arg.value.span.end;
                }
                edits.push((
                    Span {
                        start: from,
                        end: span.end,
                    },
                    " }".to_string(),
                ));
                self.fix_edits("name the fields", edits);
            } else {
                let fields: Vec<String> = names.iter().map(|field| format!("{field}: ...")).collect();
                self.help(format!("write `{} {{ {} }}`", name.name, fields.join(", ")));
            }
            for arg in args {
                self.expr(&arg.value, None);
            }
            return self.unknown(span);
        }
        match &callee.kind {
            ast::ExprKind::Name(name) if !name.is_upper() => {
                if self.resolve_local(&name.name).is_some() {
                    // A variable that holds a function. Calling it is one of the two
                    // things a function that was only lent may be used for.
                    self.lent_fn_ok = true;
                    let function = self.expr(callee, None);
                    self.lent_fn_ok = false;
                    return self.call_value(function, args, span);
                }
                if let Some(call) = self.own_method_call(name, args, span) {
                    return call;
                }
                if self.body_in_prelude
                    && let Some(intrinsic) = name.name.strip_prefix("__").and_then(crate::intrinsics::find)
                {
                    // The prelude calling the support library.
                    use crate::intrinsics::I;
                    let mut values = Vec::new();
                    for (arg, param) in args.iter().zip(intrinsic.params) {
                        // What the library keeps without knowing its type is of whatever
                        // type it is given as, and comes back as the type expected of it.
                        values.push(match param {
                            I::Cell | I::Given => self.expr(&arg.value, None),
                            _ => self.expr_as(&arg.value, &param.ty()),
                        });
                    }
                    let ty = match intrinsic.ret {
                        I::Cell => expected.cloned().unwrap_or(Ty::Unknown),
                        ret => ret.ty(),
                    };
                    return self.mk_call(
                        ExprKind::Intrinsic {
                            name: intrinsic.name,
                            args: values,
                        },
                        ty,
                        span,
                    );
                }
                if self.body_in_prelude && name.name == "__before" && args.len() == 2 {
                    // Whether the first comes before the second in the order that sorting
                    // uses. It is `<`, but for floats, where `<` is no order at all once a
                    // NaN is among them: there it is `compare`, which puts NaN last
                    // (Section 4.2).
                    let less = self.binary(ast::BinaryOp::Lt, &args[0].value, &args[1].value, span, None);
                    return match less.kind {
                        ExprKind::Binary(BinOp::Lt, a, b) if a.ty == Ty::Float => {
                            let order = self.mk(ExprKind::Builtin(Builtin::Compare, vec![*a, *b]), Ty::Int, span);
                            let zero = self.mk(ExprKind::Int(0), Ty::Int, span);
                            self.mk(
                                ExprKind::Binary(BinOp::Lt, Box::new(order), Box::new(zero)),
                                Ty::Bool,
                                span,
                            )
                        }
                        kind => Expr { kind, ..less },
                    };
                }
                if self.body_in_prelude && name.name == "__is_bytes" && args.len() == 1 {
                    // Whether a value is a list of bytes, which is known here: the body
                    // of a function with type parameters is checked for each type it is
                    // used with, so that one written for every list can do for bytes
                    // what it does for no other.
                    let value = self.expr(&args[0].value, None);
                    return self.mk(ExprKind::Bool(value.ty == Ty::list(Ty::Byte)), Ty::Bool, span);
                }
                if self.body_in_prelude && name.name == "__repr" && args.len() == 1 {
                    let value = self.expr(&args[0].value, None);
                    return self.mk_call(ExprKind::Builtin(Builtin::Repr, vec![value]), Ty::Str, span);
                }
                match name.name.as_str() {
                    "print" => {
                        let Some(values) = self.plain_args(args, 1, "`print`", span) else {
                            return self.unknown(span);
                        };
                        let value = self.expr(values[0], None);
                        self.require_printable(&value);
                        self.mk_call(ExprKind::Builtin(Builtin::Print, vec![value]), Ty::Unit, span)
                    }
                    "panic" => {
                        let Some(values) = self.plain_args(args, 1, "`panic`", span) else {
                            return self.unknown(span);
                        };
                        let message = self.expr_as(values[0], &Ty::Str);
                        self.mk_call(ExprKind::Panic(Box::new(message)), Ty::Never, span)
                    }
                    // The functions of tasks, which are not built: but a name that is not
                    // kept for them, and that the program has given a function, is its own.
                    "all" | "race" | "with_timeout" | "isolate"
                        if !self.free_fns.contains_key(&self.plain(&name.name))
                            && !self.generic_fns.contains_key(&self.plain(&name.name)) =>
                    {
                        self.unsupported(callee.span, "tasks are");
                        self.unknown(span)
                    }
                    _ if name.name.contains("__") && !self.body_in_prelude => {
                        self.error(callee.span, format!("unknown function `{}`", name.name));
                        for arg in args {
                            self.expr(&arg.value, None);
                        }
                        self.unknown(span)
                    }
                    _ if self.generic_fns.contains_key(&self.plain(&name.name)) => {
                        let template = self.generic_fns[&self.plain(&name.name)];
                        self.generic_call(template, None, HashMap::new(), args, span, expected)
                    }
                    _ => match self.free_fns.get(&self.plain(&name.name)) {
                        Some(&id) => self.user_call(id, None, args, span),
                        None => {
                            let (known, habit) = (self.function_names(), super::suggest::function_habit(&name.name));
                            let message = format!("unknown function `{}`", name.name);
                            self.error_near(callee.span, message, &name.name, known, habit);
                            for arg in args {
                                self.expr(&arg.value, None);
                            }
                            self.unknown(span)
                        }
                    },
                }
            }
            ast::ExprKind::Field { base, name } => {
                // `http.App.new()`: a function of a type of a standard package.
                if let ast::ExprKind::Field {
                    base: package,
                    name: type_name,
                } = &base.kind
                    && let ast::ExprKind::Name(package) = &package.kind
                    && type_name.is_upper()
                    && self.resolve_local(&package.name).is_none()
                    && let Some(full) = self.package_type(&[package.clone(), type_name.clone()])
                {
                    let type_name = ast::Ident {
                        name: full,
                        span: type_name.span,
                    };
                    return self.type_level_call(&type_name, name, args, span, expected);
                }
                // `os.env(...)`: a function of an imported standard package.
                if let ast::ExprKind::Name(module) = &base.kind
                    && !self.body_in_prelude
                    && self.modules.contains_key(&(self.file, module.name.clone()))
                    && self.resolve_local(&module.name).is_none()
                {
                    let import = self.modules.get_mut(&(self.file, module.name.clone())).unwrap();
                    import.1 = true;
                    // The package's own name, which `as` may have replaced in this file.
                    let own = import.2.clone();
                    let function = format!("{own}__{}", name.name);
                    if self.kept_to_its_package(&function, &own) {
                        self.error_help(
                            name.span,
                            format!("`{}.{}` is not offered by its package", module.name, name.name),
                            "what a package offers to those who import it is declared `pub` (Section 4.6)",
                        );
                        for arg in args {
                            self.expr(&arg.value, None);
                        }
                        return self.unknown(span);
                    }
                    if let Some(&template) = self.generic_fns.get(&function) {
                        return self.generic_call(template, None, HashMap::new(), args, span, expected);
                    }
                    if let Some(&id) = self.free_fns.get(&function) {
                        let call = self.user_call(id, None, args, span);
                        let level = ["debug", "info", "warn", "error"]
                            .iter()
                            .position(|level| own == "log" && *level == name.name);
                        return match level {
                            Some(level) => self.log_call(level, call, span),
                            None => call,
                        };
                    }
                    let known = self.package_names(&own);
                    let message = format!("the package `{}` has no function `{}`", module.name, name.name);
                    self.error_near(name.span, message, &name.name, known, None);
                    for arg in args {
                        self.expr(&arg.value, None);
                    }
                    return self.unknown(span);
                }
                // `int.parse(s)`: a function of a primitive type.
                if let ast::ExprKind::Name(type_name) = &base.kind
                    && is_primitive(&type_name.name)
                    && self.lookup(&type_name.name).is_none()
                {
                    return self.primitive_call(&type_name.name, name, args, span);
                }
                // `Order.new(...)`: a type-level function.
                if let ast::ExprKind::Name(type_name) = &base.kind
                    && type_name.is_upper()
                {
                    return self.type_level_call(type_name, name, args, span, expected);
                }
                // `List<int>.new()`.
                if let ast::ExprKind::TypeApp {
                    base: generic,
                    args: type_args,
                } = &base.kind
                {
                    return self.generic_type_call(generic, type_args, name, args, span, expected);
                }
                let receiver = self.expr(base, None);
                self.method_call(receiver, name, args, span, expected)
            }
            _ => {
                let function = self.expr(callee, None);
                self.call_value(function, args, span)
            }
        }
    }

    /// A call of a function value (Section 4.11).
    fn call_value(&mut self, function: Expr, args: &[ast::Arg], span: Span) -> Expr {
        let Ty::Fn(ty) = function.ty.clone() else {
            if !function.ty.is_unknown() {
                let found = self.ty_name(&function.ty);
                self.error(
                    function.span,
                    format!("`{found}` is not a function and cannot be called"),
                );
                self.label(format!("this is {}", super::a(&found)));
            }
            for arg in args {
                self.expr(&arg.value, None);
            }
            return self.unknown(span);
        };
        if args.len() != ty.params.len() {
            self.error(
                span,
                format!(
                    "this function takes {} argument{}, found {}",
                    ty.params.len(),
                    if ty.params.len() == 1 { "" } else { "s" },
                    args.len()
                ),
            );
            let found = self.ty_name(&function.ty);
            self.related(Some(function.span), format!("this is {}", super::a(&found)));
            for arg in args {
                self.expr(&arg.value, None);
            }
            return self.unknown(span);
        }
        let mut out = Vec::new();
        for (arg, (kind, param)) in args.iter().zip(&ty.params) {
            if let Some(name) = &arg.name {
                self.error(name.span, "a function value takes its arguments by position");
            }
            let expr = self.pass(arg, *kind, param, "this function");
            out.push(Arg { kind: *kind, expr });
        }
        self.exclusive(&out, Some(&function));
        self.mk_call(
            ExprKind::CallValue {
                callee: Box::new(function),
                args: out,
            },
            ty.ret,
            span,
        )
    }

    /// Checks one argument against the kind and type of its parameter.
    fn pass(&mut self, arg: &ast::Arg, kind: ParamKind, ty: &Ty, function: &str) -> Expr {
        if kind == ParamKind::Var {
            let value = self.expr(&arg.value, Some(ty));
            if !arg.var {
                self.error_help(
                    arg.value.span,
                    format!("{function} changes this argument, so it is passed with `var`"),
                    "write `var` before the argument, so that the change is visible at the call (Section 5.3)",
                );
            } else if !value.ty.is_unknown() {
                self.require_mutable_place(&value, "pass as `var`");
            }
            if value.ty != *ty && !value.ty.is_unknown() && !ty.is_unknown() {
                let (want, found) = (self.ty_name(ty), self.ty_name(&value.ty));
                self.error(arg.value.span, format!("expected `{want}`, found `{found}`"));
            }
            return value;
        }
        if arg.var {
            self.error_help(
                arg.value.span,
                "this parameter is not `var`",
                "remove `var`: the function does not change this argument",
            );
        }
        // A function value's parameter of a function type is lent, as a lambda's is.
        self.function_argument_context(&arg.value, kind == ParamKind::Borrow && matches!(ty, Ty::Fn(_)));
        let value = self.expr_as(&arg.value, ty);
        self.next_lambda_is_block = false;
        self.lent_fn_ok = false;
        value
    }

    /// Before an argument is checked: says whether a lambda written there is a block, and
    /// whether a lent function may be named there. Both hold exactly when the parameter is
    /// borrowed and its type is a function type (Section 5.7).
    pub(super) fn function_argument_context(&mut self, value: &ast::Expr, lends: bool) {
        // Parentheses around the argument change nothing.
        let mut value = value;
        while let ast::ExprKind::Paren(inner) = &value.kind {
            value = inner;
        }
        self.next_lambda_is_block = lends && matches!(value.kind, ast::ExprKind::Lambda { .. });
        self.lent_fn_ok = lends && matches!(value.kind, ast::ExprKind::Name(_));
    }

    fn type_level_call(
        &mut self,
        type_name: &ast::Ident,
        name: &ast::Ident,
        args: &[ast::Arg],
        span: Span,
        expected: Option<&Ty>,
    ) -> Expr {
        // (By the type's own name, where it is written by another: what is kept of a
        // type is kept from every name for it.)
        let own = match self.type_names.get(&type_name.name) {
            Some(TypeName::Alias(Ty::Struct(id))) => Some(self.program.structs[*id].name.clone()),
            Some(TypeName::Alias(Ty::Enum(id))) => Some(self.program.enums[*id].name.clone()),
            Some(TypeName::Alias(Ty::Error(id))) => Some(self.program.errors[*id].name.clone()),
            _ => None,
        };
        self.offered(own.as_deref().unwrap_or(&type_name.name), name);
        // Another name for a type has the functions of the type it names (Section 6.7).
        if let Some(TypeName::Alias(named)) = self.type_names.get(&type_name.name).cloned() {
            let primitive = match named {
                Ty::Int => Some("int"),
                Ty::Float => Some("float"),
                Ty::Bool => Some("bool"),
                Ty::Str => Some("string"),
                Ty::Byte => Some("byte"),
                _ => None,
            };
            // A list has three functions that no program or library writes.
            if let Ty::List(element) = &named
                && matches!(name.name.as_str(), "new" | "with_capacity" | "with_size")
            {
                let (element, ty) = ((**element).clone(), named.clone());
                let what = format!("`{}.{}`", type_name.name, name.name);
                let wanted = usize::from(name.name != "new");
                let Some(values) = self.plain_args(args, wanted, &what, span) else {
                    return self.unknown(span);
                };
                if name.name == "new" {
                    return self.mk_call(ExprKind::ListLit(Vec::new()), ty, span);
                }
                let n = self.expr_as(values[0], &Ty::Int);
                if name.name == "with_capacity" {
                    return self.mk_call(ExprKind::Builtin(Builtin::ListWithCapacity, vec![n]), ty, span);
                }
                if !matches!(element, Ty::Int | Ty::Float | Ty::Bool | Ty::Byte | Ty::Unknown) {
                    let found = self.ty_name(&element);
                    self.error_help(
                        type_name.span,
                        format!("`List<{found}>.with_size` is not supported by this compiler yet"),
                        "it fills a list with zeros, so the element type is `int`, `float`, `bool` or `byte`",
                    );
                    return self.unknown(span);
                }
                return self.mk_call(ExprKind::Builtin(Builtin::ListWithSize, vec![n]), ty, span);
            }
            return match primitive {
                Some(primitive) => self.primitive_call(primitive, name, args, span),
                None => self.call_on_type(&named, name, args, span, expected),
            };
        }
        if let Some(template) = self.generic_type_fn(&type_name.name, &name.name) {
            // The type arguments of a generic type come from the arguments and the
            // expected type; a type that is not generic is known already.
            let on = match self.type_names.get(&type_name.name) {
                Some(TypeName::Struct(id)) => Some(Ty::Struct(*id)),
                Some(TypeName::Enum(id)) => Some(Ty::Enum(*id)),
                Some(TypeName::Error(id)) => Some(Ty::Error(*id)),
                _ => None,
            };
            return self.generic_type_level_call(template, on, args, span, expected);
        }
        if let Some(bound) = self.type_env.get(&type_name.name).cloned() {
            // `T.new()` in a generic function: a function of the type `T` stands for.
            return self.call_on_type(&bound, name, args, span, expected);
        }
        if self.generic_types.contains_key(&type_name.name) {
            self.error(
                name.span,
                format!(
                    "`{}` has no function `{}`",
                    self.shown_name(&type_name.name, true),
                    name.name
                ),
            );
            for arg in args {
                self.expr(&arg.value, None);
            }
            return self.unknown(span);
        }
        let owner = match self.type_names.get(&type_name.name) {
            Some(TypeName::Struct(id)) => Some(Owner::Struct(*id)),
            Some(TypeName::Enum(id)) => Some(Owner::Enum(*id)),
            Some(TypeName::Error(id)) => Some(Owner::Error(*id)),
            Some(TypeName::Alias(ty)) => self.owner_of(&ty.clone()),
            Some(TypeName::ErrorSet(_)) => None,
            None => {
                self.error(type_name.span, format!("unknown type `{}`", type_name.name));
                for arg in args {
                    self.expr(&arg.value, None);
                }
                return self.unknown(span);
            }
        };
        let function = owner.and_then(|owner| self.type_fns.get(&(owner, name.name.clone())).copied());
        match function {
            Some(id) => self.user_call(id, None, args, span),
            None => {
                let is_method = owner.is_some_and(|owner| self.methods.contains_key(&(owner, name.name.clone())));
                let is_variant = match owner {
                    Some(Owner::Enum(id)) => self.program.enums[id].variants.iter().any(|v| v.name == name.name),
                    _ => false,
                };
                let message = format!(
                    "`{}` has no function `{}`",
                    self.shown_name(&type_name.name, true),
                    name.name
                );
                // The functions the compiler writes for a type that can have them.
                let written = match name.name.as_str() {
                    "from_env" => Some(
                        "`from_env` is written for a struct whose every field is an `int`, a `float`, a `bool`, a `string` or a `Duration`, or an optional of one: each is read from the environment variable of its name in capitals",
                    ),
                    "from_json" => Some(
                        "JSON is written for a struct or an enum whose every field is a number, a string, a boolean, a byte, a list, an optional, a map with string keys, or another such type. A type with a field that is a function, a tuple or an optional of an optional has none, nor an enum with a variant whose field has the key `type`, which says which variant it is (Section 8.7)",
                    ),
                    "from_row" => Some(
                        "`from_row` is written for a struct whose every field is an `int`, a `float`, a `bool`, a `string` or a `Time`, or an optional of one: each is read from the column of its name (Section 10.3)",
                    ),
                    _ => None,
                };
                if let (Some(help), Some(Owner::Struct(_))) = (written, owner) {
                    self.error_help(name.span, message, help);
                } else if let (Some(help), Some(Owner::Enum(_)), "from_json") = (written, owner, name.name.as_str()) {
                    self.error_help(name.span, message, help);
                } else if is_method {
                    self.error_help(
                        name.span,
                        message,
                        format!(
                            "`{}` is a method: call it on a value, as `x.{}(...)`",
                            name.name, name.name
                        ),
                    );
                } else if is_variant {
                    self.error_help(
                        name.span,
                        message,
                        format!(
                            "`{}` is a variant: write `{}.{} {{ ... }}`",
                            name.name, type_name.name, name.name
                        ),
                    );
                } else {
                    self.error(name.span, message);
                }
                for arg in args {
                    self.expr(&arg.value, None);
                }
                self.unknown(span)
            }
        }
    }

    /// A type-level call on a type that is already known, as `T.new()` is once `T` has an
    /// argument.
    fn call_on_type(
        &mut self,
        on: &Ty,
        name: &ast::Ident,
        args: &[ast::Arg],
        span: Span,
        expected: Option<&Ty>,
    ) -> Expr {
        // (Through a type parameter too, what a package keeps of a type is kept.)
        let own = match on {
            Ty::Struct(id) => Some(self.program.structs[*id].name.clone()),
            Ty::Enum(id) => Some(self.program.enums[*id].name.clone()),
            Ty::Error(id) => Some(self.program.errors[*id].name.clone()),
            _ => None,
        };
        if let Some(own) = own {
            self.offered(&own, name);
        }
        if let Some(owner) = self.owner_of(on)
            && let Some(&id) = self.type_fns.get(&(owner, name.name.clone()))
        {
            return self.user_call(id, None, args, span);
        }
        if let Some((generic, _)) = self.instance_of.get(on).cloned()
            && let Some(template) = self.generic_type_fn(&generic, &name.name)
        {
            return self.generic_type_level_call(template, Some(on.clone()), args, span, expected);
        }
        // The prelude reads an optional from JSON by a method, on a `none` of the type.
        if matches!(on, Ty::Optional(_)) && name.name == "_json_read" && self.body_in_prelude {
            let nothing = self.mk(ExprKind::NoneLit, on.clone(), span);
            let method = ast::Ident {
                name: "_json_read_optional".to_string(),
                span: name.span,
            };
            return self.method_call(nothing, &method, args, span, expected);
        }
        // A list has the functions of `List<T>`: `T.from_json` where `T` is `[Order]`.
        if matches!(on, Ty::List(_))
            && let Some(template) = self.generic_type_fn("List", &name.name)
        {
            return self.generic_type_level_call(template, Some(on.clone()), args, span, expected);
        }
        let found = self.ty_name(on);
        self.error(name.span, format!("`{found}` has no function `{}`", name.name));
        for arg in args {
            self.expr(&arg.value, None);
        }
        self.unknown(span)
    }

    fn generic_type_call(
        &mut self,
        generic: &ast::Expr,
        type_args: &[ast::Type],
        name: &ast::Ident,
        args: &[ast::Arg],
        span: Span,
        expected: Option<&Ty>,
    ) -> Expr {
        // `orders.Box<int>.new()`: a generic type of a package the file imports.
        if let Some(of) = self.unqualified(generic) {
            return self.generic_type_call(&of, type_args, name, args, span, expected);
        }
        let ast::ExprKind::Name(generic_name) = &generic.kind else {
            self.unsupported(generic.span, "types from other packages are");
            return self.unknown(span);
        };
        self.offered(&generic_name.name, name);
        let is_list = generic_name.name == "List";
        let builtin = is_list && matches!(name.name.as_str(), "new" | "with_size" | "with_capacity");
        if !builtin {
            // `Stack<int>.new()`: a type-level function of a generic type, named in full.
            let type_args: Vec<Ty> = type_args.iter().map(|arg| self.resolve_type(arg)).collect();
            let on = if is_list && type_args.len() == 1 {
                Ty::list(type_args[0].clone())
            } else if self.generic_types.contains_key(&generic_name.name) {
                self.instantiate_type(&generic_name.name, type_args, generic.span)
            } else {
                self.error(generic.span, format!("`{}` is not a generic type", generic_name.name));
                Ty::Unknown
            };
            return match self.generic_type_fn(&generic_name.name, &name.name) {
                Some(template) if !on.is_unknown() => {
                    self.generic_type_level_call(template, Some(on), args, span, expected)
                }
                Some(_) => self.unknown(span),
                None => {
                    self.error(
                        name.span,
                        format!("`{}` has no function `{}`", generic_name.name, name.name),
                    );
                    for arg in args {
                        self.expr(&arg.value, None);
                    }
                    self.unknown(span)
                }
            };
        }
        if type_args.len() != 1 {
            self.error(generic.span, "`List` takes one type argument: `List<T>`");
            return self.unknown(span);
        }
        let element = self.resolve_type(&type_args[0]);
        let ty = Ty::list(element.clone());
        match name.name.as_str() {
            "new" => {
                if self.plain_args(args, 0, "`List<T>.new`", span).is_none() {
                    return self.unknown(span);
                }
                self.mk_call(ExprKind::ListLit(Vec::new()), ty, span)
            }
            "with_capacity" => {
                let Some(values) = self.plain_args(args, 1, "`List<T>.with_capacity`", span) else {
                    return self.unknown(span);
                };
                let n = self.expr_as(values[0], &Ty::Int);
                self.mk_call(ExprKind::Builtin(Builtin::ListWithCapacity, vec![n]), ty, span)
            }
            "with_size" => {
                let Some(values) = self.plain_args(args, 1, "`List<T>.with_size`", span) else {
                    return self.unknown(span);
                };
                let n = self.expr_as(values[0], &Ty::Int);
                if !matches!(element, Ty::Int | Ty::Float | Ty::Bool | Ty::Byte | Ty::Unknown) {
                    let found = self.ty_name(&element);
                    self.error_help(
                        type_args[0].span,
                        format!("`List<{found}>.with_size` is not supported by this compiler yet"),
                        "it fills a list with zeros, so the element type is `int`, `float`, `bool` or `byte`",
                    );
                    return self.unknown(span);
                }
                self.mk_call(ExprKind::Builtin(Builtin::ListWithSize, vec![n]), ty, span)
            }
            other => {
                self.error(name.span, format!("`List` has no function `{other}`"));
                self.unknown(span)
            }
        }
    }

    /// `int.parse(s)`, `float.from(x)` and the other functions of the primitive types.
    fn primitive_call(&mut self, type_name: &str, name: &ast::Ident, args: &[ast::Arg], span: Span) -> Expr {
        // The prelude declares more functions on the primitive types.
        let owner = match type_name {
            "string" => Some(Owner::Str),
            "int" => Some(Owner::Int),
            "float" => Some(Owner::Float),
            "bool" => Some(Owner::Bool),
            "byte" => Some(Owner::Byte),
            _ => None,
        };
        if let Some(owner) = owner
            && let Some(&id) = self.type_fns.get(&(owner, name.name.clone()))
        {
            return self.user_call(id, None, args, span);
        }
        let what = format!("`{type_name}.{}`", name.name);
        let Some(values) = self.plain_args(args, 1, &what, span) else {
            return self.unknown(span);
        };
        let failing = |ok: Ty, error: ErrorId| Ty::Result(Box::new(ok), ErrSet::of(vec![error]));
        match (type_name, name.name.as_str()) {
            ("int", "parse") => {
                let text = self.expr_as(values[0], &Ty::Str);
                let ty = failing(Ty::Int, self.parse_error);
                self.mk_call(ExprKind::Builtin(Builtin::IntParse, vec![text]), ty, span)
            }
            ("float", "parse") => {
                let text = self.expr_as(values[0], &Ty::Str);
                let ty = failing(Ty::Float, self.parse_error);
                self.mk_call(ExprKind::Builtin(Builtin::FloatParse, vec![text]), ty, span)
            }
            ("float", "from") => {
                let value = self.expr(values[0], None);
                match value.ty {
                    Ty::Int => self.mk_call(ExprKind::Builtin(Builtin::FloatFromInt, vec![value]), Ty::Float, span),
                    Ty::Float | Ty::Unknown => value,
                    _ => {
                        let found = self.ty_name(&value.ty);
                        self.error(span, format!("`float.from` takes an `int`, found `{found}`"));
                        self.unknown(span)
                    }
                }
            }
            ("int", "from") => {
                let value = self.expr(values[0], None);
                match value.ty {
                    Ty::Byte => self.mk_call(ExprKind::Builtin(Builtin::IntFromByte, vec![value]), Ty::Int, span),
                    Ty::Int | Ty::Unknown => value,
                    Ty::Float => {
                        self.error_help(
                            span,
                            "`int.from` does not take a `float`",
                            "choose the rounding: `x.round()`, `x.floor()`, `x.ceil()` or `x.trunc()` (Section 4.2)",
                        );
                        self.unknown(span)
                    }
                    _ => {
                        let found = self.ty_name(&value.ty);
                        self.error(span, format!("`int.from` does not take `{found}`"));
                        self.unknown(span)
                    }
                }
            }
            ("byte", "from") => {
                let value = self.expr_as(values[0], &Ty::Int);
                let ty = failing(Ty::Byte, self.overflow);
                self.mk_call(ExprKind::Builtin(Builtin::ByteFromInt, vec![value]), ty, span)
            }
            _ => {
                self.error(name.span, format!("`{type_name}` has no function `{}`", name.name));
                for value in values {
                    self.expr(value, None);
                }
                self.unknown(span)
            }
        }
    }

    /// A call of a method. Where the method is one of the library's that is told the
    /// place of its call, the place is that of the method's name, which in a chain
    /// over several lines is the line the call is written on and not the chain's first.
    pub(super) fn method_call(
        &mut self,
        receiver: Expr,
        name: &ast::Ident,
        args: &[ast::Arg],
        span: Span,
        expected: Option<&Ty>,
    ) -> Expr {
        let mut call = self.method_call_at(receiver, name, args, span, expected);
        if let ExprKind::Call { args, .. } = &mut call.kind
            && let Some(last) = args.last_mut()
            && matches!(last.expr.kind, ExprKind::Location)
        {
            last.expr.span = name.span;
        }
        call
    }

    fn method_call_at(
        &mut self,
        receiver: Expr,
        name: &ast::Ident,
        args: &[ast::Arg],
        span: Span,
        expected: Option<&Ty>,
    ) -> Expr {
        let _ = expected;
        if receiver.ty.is_unknown() {
            for arg in args {
                self.expr(&arg.value, None);
            }
            return self.unknown(span);
        }
        // A method that its package has not offered, of whatever kind: of a struct, of
        // a type with parameters, of an actor.
        let on = match super::exprs::peel(&receiver.ty) {
            Ty::Struct(id) => Some(self.program.structs[*id].name.clone()),
            Ty::Enum(id) => Some(self.program.enums[*id].name.clone()),
            Ty::Error(id) => Some(self.program.errors[*id].name.clone()),
            _ => None,
        };
        if let Some(on) = on {
            self.offered(&on, name);
        }
        if let Some(owner) = self.owner_of(&receiver.ty)
            && let Some(&id) = self.methods.get(&(owner, name.name.clone()))
        {
            return self.user_call(id, Some(receiver), args, span);
        }
        // `route.handler(request)`: a field that holds a function is called as it is named.
        let fields = match &receiver.ty {
            Ty::Struct(id) => Some(&self.program.structs[*id].fields),
            Ty::Error(id) => Some(&self.program.errors[*id].fields),
            _ => None,
        };
        if let Some(fields) = fields
            && let Some(index) = fields.iter().position(|field| field.name == name.name)
            && matches!(fields[index].ty, Ty::Fn(_))
            && (self.body_in_prelude || !self.is_prelude_type(&receiver.ty))
            && !(name.name.starts_with('_') && self.kept_field(&receiver.ty))
        {
            let ty = fields[index].ty.clone();
            let function = self.mk(ExprKind::Field(Box::new(receiver), index), ty, name.span);
            return self.call_value(function, args, span);
        }
        // Integers that are equal are the same, so their sort need not be stable, and the
        // support library has one made for them. Every other list is sorted by the
        // prelude's `sort`.
        if receiver.ty == Ty::List(Box::new(Ty::Int)) && name.name == "sort" && args.is_empty() {
            self.require_mutable_place(&receiver, "change");
            return Expr {
                kind: ExprKind::Builtin(Builtin::ListSortInts, vec![receiver]),
                ty: Ty::Unit,
                span,
            };
        }
        if let Some(template) = self.generic_method(&receiver.ty, &name.name) {
            return self.generic_call(template, Some(receiver), HashMap::new(), args, span, expected);
        }
        let method = name.name.as_str();
        let what = format!("`{method}`");
        let overflow = Ty::Result(Box::new(Ty::Int), ErrSet::of(vec![self.overflow]));
        let builtin = |b: Builtin, args: Vec<Expr>, ty: Ty| Expr {
            kind: ExprKind::Builtin(b, args),
            ty,
            span,
        };
        match (receiver.ty.clone(), method) {
            // Every value has a text form (Section 4.6).
            (ty, "to_string") if !matches!(ty, Ty::Unit | Ty::Result(..)) => {
                if self.plain_args(args, 0, &what, span).is_none() {
                    return self.unknown(span);
                }
                Expr {
                    kind: ExprKind::Interp(vec![InterpPart::Value(receiver, Format::default())]),
                    ty: Ty::Str,
                    span,
                }
            }
            (Ty::List(element), "push") => {
                let Some(values) = self.plain_args(args, 1, &what, span) else {
                    return self.unknown(span);
                };
                self.require_mutable_place(&receiver, "change");
                let value = self.expr_as(values[0], &element);
                builtin(Builtin::ListPush, vec![receiver, value], Ty::Unit)
            }
            (Ty::List(element), "pop") => {
                if self.plain_args(args, 0, &what, span).is_none() {
                    return self.unknown(span);
                }
                self.require_mutable_place(&receiver, "change");
                builtin(Builtin::ListPop, vec![receiver], Ty::Optional(element))
            }
            (Ty::List(element), "get") => {
                let Some(values) = self.plain_args(args, 1, &what, span) else {
                    return self.unknown(span);
                };
                let index = self.expr_as(values[0], &Ty::Int);
                builtin(Builtin::ListGet, vec![receiver, index], Ty::Optional(element))
            }
            (Ty::List(_), "swap") => {
                let Some(values) = self.plain_args(args, 2, &what, span) else {
                    return self.unknown(span);
                };
                self.require_mutable_place(&receiver, "change");
                let i = self.expr_as(values[0], &Ty::Int);
                let j = self.expr_as(values[1], &Ty::Int);
                builtin(Builtin::ListSwap, vec![receiver, i, j], Ty::Unit)
            }
            (Ty::List(_), "clear") => {
                if self.plain_args(args, 0, &what, span).is_none() {
                    return self.unknown(span);
                }
                self.require_mutable_place(&receiver, "change");
                builtin(Builtin::ListClear, vec![receiver], Ty::Unit)
            }
            (Ty::Int | Ty::Float, "abs") => {
                if self.plain_args(args, 0, &what, span).is_none() {
                    return self.unknown(span);
                }
                let ty = receiver.ty.clone();
                builtin(Builtin::Abs, vec![receiver], ty)
            }
            (Ty::Int | Ty::Float | Ty::Str | Ty::Byte | Ty::Bool, "compare") => {
                let Some(values) = self.plain_args(args, 1, &what, span) else {
                    return self.unknown(span);
                };
                let ty = receiver.ty.clone();
                let other = self.expr_as(values[0], &ty);
                builtin(Builtin::Compare, vec![receiver, other], Ty::Int)
            }
            (ty, "hash") if self.is_hashable(&ty) => {
                if self.plain_args(args, 0, &what, span).is_none() {
                    return self.unknown(span);
                }
                builtin(Builtin::Hash, vec![receiver], Ty::Int)
            }
            (Ty::Int | Ty::Float, "min" | "max") => {
                let Some(values) = self.plain_args(args, 1, &what, span) else {
                    return self.unknown(span);
                };
                let ty = receiver.ty.clone();
                let other = self.expr_as(values[0], &ty);
                let b = if method == "min" { Builtin::Min } else { Builtin::Max };
                builtin(b, vec![receiver, other], ty)
            }
            (
                Ty::Int,
                "wrapping_add" | "wrapping_sub" | "wrapping_mul" | "bit_and" | "bit_or" | "bit_xor" | "shl" | "shr",
            ) => {
                let Some(values) = self.plain_args(args, 1, &what, span) else {
                    return self.unknown(span);
                };
                let other = self.expr_as(values[0], &Ty::Int);
                let b = match method {
                    "wrapping_add" => Builtin::WrappingAdd,
                    "wrapping_sub" => Builtin::WrappingSub,
                    "wrapping_mul" => Builtin::WrappingMul,
                    "bit_and" => Builtin::BitAnd,
                    "bit_or" => Builtin::BitOr,
                    "bit_xor" => Builtin::BitXor,
                    "shl" => Builtin::Shl,
                    _ => Builtin::Shr,
                };
                builtin(b, vec![receiver, other], Ty::Int)
            }
            (Ty::Float, "sqrt") => {
                if self.plain_args(args, 0, &what, span).is_none() {
                    return self.unknown(span);
                }
                builtin(Builtin::FloatSqrt, vec![receiver], Ty::Float)
            }
            (Ty::Float, "floor" | "ceil" | "round" | "trunc") => {
                if self.plain_args(args, 0, &what, span).is_none() {
                    return self.unknown(span);
                }
                let b = match method {
                    "floor" => Builtin::FloatFloor,
                    "ceil" => Builtin::FloatCeil,
                    "round" => Builtin::FloatRound,
                    _ => Builtin::FloatTrunc,
                };
                builtin(b, vec![receiver], overflow)
            }
            (ty, _) => {
                let found = self.ty_name(&ty);
                let message = format!("`{found}` has no method `{method}`");
                match &ty {
                    Ty::Optional(inner) => {
                        let inner = self.ty_name(inner);
                        self.error(name.span, message);
                        self.related(
                            Some(receiver.span),
                            format!("this is {}: it may be `none`", super::a(&found)),
                        );
                        self.help(format!(
                            "`{method}` is called on the `{inner}` inside: give a value to use when there is none, `(x else fallback).{method}(...)`, or call it only when the value is there, `if value := x {{ ... }}` (Section 4.8)"
                        ));
                    }
                    Ty::Result(inner, _) => {
                        let inner = self.ty_name(inner);
                        self.error(name.span, message);
                        self.related(
                            Some(receiver.span),
                            format!("this is {}: it may be an error", super::a(&found)),
                        );
                        self.help(format!(
                            "`{method}` is called on the `{inner}` inside: `x?.{method}(...)` passes the error on to the caller, and `(x else fallback).{method}(...)` gives a value in its place (Section 6.4)"
                        ));
                    }
                    // A type that JSON is not written for: why not.
                    Ty::Struct(_) | Ty::Enum(_) if method == "to_json" => {
                        self.error_help(name.span, message, "JSON is written for a struct or an enum whose every field is a number, a string, a boolean, a byte, a list, an optional, a map with string keys, or another such type. A type with a field that is a function, a tuple or an optional of an optional has none, nor an enum with a variant whose field has the key `type`, which says which variant it is (Section 8.7)");
                    }
                    // `xs.length()`: the number of elements, which is a field.
                    Ty::List(_) | Ty::Str
                        if matches!(method, "length" | "size" | "count" | "len") && args.is_empty() =>
                    {
                        self.error(name.span, message);
                        self.note("the number of elements is a field, not a method");
                        let call = Span {
                            start: name.span.start,
                            end: span.end,
                        };
                        self.fix("write `.len`", call, "len");
                    }
                    // `handle.send.method(...)` for a method that has something to say back.
                    Ty::Struct(id)
                        if self.program.structs[*id]
                            .name
                            .strip_prefix("Actor")
                            .and_then(|rest| rest.strip_suffix("Send").or(rest.strip_suffix("Try")))
                            .is_some_and(|actor| {
                                self.free_fns
                                    .contains_key(&crate::derive::method_function(actor, method))
                            }) =>
                    {
                        self.error(name.span, format!("`{method}` cannot be sent and forgotten"));
                        self.label("it returns a value, or may fail");
                        self.note("`send` is for a method that returns nothing and cannot fail, so that nothing is thrown away (Section 7.3)");
                        self.help(format!("ask, and wait for the answer: `handle.{method}(...)`"));
                    }
                    _ => match self.method_rename(&ty, method) {
                        Some(koo) => {
                            self.error(name.span, message);
                            self.fix(format!("in Koo that is `.{koo}`"), name.span, koo);
                        }
                        None => {
                            let (known, habit) = (self.method_names(&ty), self.method_habit(&ty, method));
                            self.error_near(name.span, message, method, known, habit);
                        }
                    },
                }
                for arg in args {
                    self.expr(&arg.value, None);
                }
                self.unknown(span)
            }
        }
    }

    /// Pairs the arguments of a call with the parameters, by position and by name. The
    /// second result is whether every argument found a parameter: if not, that is
    /// reported, and what the call is missing is not worth saying as well. `at` is where
    /// the function is declared, if the program declares it.
    fn match_args<'e>(
        &mut self,
        function: &str,
        names: &[String],
        args: &'e [ast::Arg],
        at: Option<Span>,
        located: bool,
    ) -> (Vec<Option<&'e ast::Arg>>, bool) {
        let mut slots: Vec<Option<&ast::Arg>> = vec![None; names.len()];
        // A function of the library may end with a parameter for where it was called
        // from, which the compiler fills in (`finish_call`): it is not a program's to
        // give, by its name or by its place.
        let hidden = located && !self.body_in_prelude;
        let names = &names[..names.len() - usize::from(hidden)];
        let mut positional = 0;
        let mut paired = true;
        let mut extra: Option<Span> = None;
        for arg in args {
            let index = match &arg.name {
                None => {
                    positional += 1;
                    positional - 1
                }
                Some(name) => match names.iter().position(|p| *p == name.name) {
                    Some(index) => index,
                    None => {
                        let message = format!("`{function}` has no parameter `{}`", name.name);
                        self.error_near(name.span, message, &name.name, names.to_vec(), None);
                        self.related(at, format!("`{function}` is declared here"));
                        self.expr(&arg.value, None);
                        paired = false;
                        continue;
                    }
                },
            };
            if index >= names.len() {
                self.expr(&arg.value, None);
                extra = Some(extra.map_or(arg.value.span, |first| first.to(arg.value.span)));
                continue;
            }
            if let Some(first) = slots[index] {
                self.error(
                    arg.value.span,
                    format!("the parameter `{}` is given twice", names[index]),
                );
                self.label("given again here");
                self.related(Some(first.value.span), "first given here");
                paired = false;
                continue;
            }
            slots[index] = Some(arg);
        }
        if let Some(extra) = extra {
            let takes = match names.len() {
                0 => "no arguments".to_string(),
                1 => "1 argument".to_string(),
                n => format!("{n} arguments"),
            };
            self.error(extra, format!("`{function}` takes {takes}, found {positional}"));
            self.label(if positional - names.len() == 1 {
                "this argument is one too many"
            } else {
                "these arguments are too many"
            });
            self.related(at, format!("`{function}` is declared here"));
            paired = false;
        }
        (slots, paired)
    }

    /// A call of a function declared in the program. `receiver` is the value a method is
    /// called on.
    pub(super) fn user_call(&mut self, id: FnId, receiver: Option<Expr>, args: &[ast::Arg], span: Span) -> Expr {
        let names: Vec<String> = self.sigs[id].params.iter().map(|p| p.name.clone()).collect();
        let function = self.sigs[id].name.clone();
        let at = self.sigs[id].at.map(|at| at.name);
        let located = self.located.contains(&id);
        let (slots, paired) = self.match_args(&function, &names, args, at, located);
        let checked = vec![None; slots.len()];
        self.finish_call(id, receiver, slots, checked, paired, span)
    }

    /// What a function of the library is told of where it was called: the place of the
    /// call, if a program makes it. A function of the library that calls another passes
    /// on what it was told itself, its own place being of no use to anyone, and one
    /// that was told nothing has nothing to pass on.
    pub(super) fn call_location(&mut self) -> ExprKind {
        if !self.body_in_prelude {
            return ExprKind::Location;
        }
        match self
            .resolve_local("location")
            .filter(|id| self.locals[*id].ty == Ty::Str)
        {
            Some(local) => {
                self.used[local] = true;
                ExprKind::Local(local)
            }
            None => ExprKind::Str(String::new()),
        }
    }

    /// Completes a call once its arguments are paired with parameters. An argument that
    /// inference already checked comes in `checked`.
    fn finish_call(
        &mut self,
        id: FnId,
        receiver: Option<Expr>,
        slots: Vec<Option<&ast::Arg>>,
        mut checked: Vec<Option<Expr>>,
        paired: bool,
        span: Span,
    ) -> Expr {
        let sig = self.sigs[id].clone();
        let mut out: Vec<Arg> = Vec::new();
        if let (Some(receiver), Some(param)) = (receiver, &sig.receiver) {
            // A receiver is not marked at the call (Section 5.3).
            if param.kind == ParamKind::Var {
                self.require_mutable_place(&receiver, "change");
            }
            let receiver = if param.kind == ParamKind::Var {
                receiver
            } else {
                self.coerce(receiver, &param.ty)
            };
            out.push(Arg {
                kind: param.kind,
                expr: receiver,
            });
        }
        let mut missing = Vec::new();
        let mut missing_at: Option<Span> = None;
        for (index, (param, slot)) in sig.params.iter().zip(slots).enumerate() {
            let Some(arg) = slot else {
                // A function of the standard library that reports where it was called
                // from takes that as a last parameter, which the call does not write.
                if index + 1 == sig.params.len() && self.located.contains(&id) {
                    // Where the program called the library is what a function of the
                    // library passes on to another: its own place is of no use to
                    // anyone. One that was not told has nothing to pass on.
                    let kind = self.call_location();
                    out.push(Arg {
                        kind: param.kind,
                        expr: Expr {
                            kind,
                            ty: Ty::Str,
                            span,
                        },
                    });
                    continue;
                }
                if let Some(default) = param.default {
                    // The default is a constant, checked where nothing of the caller is
                    // in scope, so it means the same at every call.
                    let default = self.param_defaults[default];
                    let scopes = std::mem::replace(&mut self.scopes, vec![HashMap::new()]);
                    let enclosing = std::mem::take(&mut self.enclosing);
                    let lambda = self.lambda.take();
                    let value = self.expr(default, Some(&param.ty));
                    let mut value = self.coerce(value, &param.ty);
                    self.scopes = scopes;
                    self.enclosing = enclosing;
                    self.lambda = lambda;
                    // An error in it is reported at the declaration; the value belongs
                    // to the call.
                    value.span = span;
                    out.push(Arg {
                        kind: param.kind,
                        expr: value,
                    });
                    continue;
                }
                missing.push(format!("`{}`", param.name));
                missing_at = missing_at.or(param.at);
                continue;
            };
            let done = checked[index].take();
            let expr = if param.kind == ParamKind::Var {
                let value = match done {
                    Some(value) => value,
                    None => self.expr(&arg.value, Some(&param.ty)),
                };
                let declared = format!("the parameter `{}` is declared here", param.name);
                if !arg.var {
                    self.error(
                        arg.value.span,
                        format!("`{}` changes this argument, so it is passed with `var`", sig.name),
                    );
                    self.related(param.at, format!("the parameter `{}` is `var`", param.name));
                    self.note("`var` at the call shows the reader that the argument may change (Section 5.3)");
                    let before = Span {
                        start: arg.value.span.start,
                        end: arg.value.span.start,
                    };
                    self.fix("write `var` before the argument", before, "var ");
                } else if !value.ty.is_unknown() {
                    self.require_mutable_place(&value, "pass as `var`");
                }
                if value.ty != param.ty && !value.ty.is_unknown() && !param.ty.is_unknown() {
                    let (want, found) = (self.ty_name(&param.ty), self.ty_name(&value.ty));
                    self.error(arg.value.span, format!("expected `{want}`, found `{found}`"));
                    self.label(format!("this is {}", super::a(&found)));
                    self.related(param.at, declared);
                    self.note("a `var` argument has exactly the parameter's type: the function may store any such value in it");
                }
                value
            } else {
                let declared = format!("the parameter `{}` is declared here", param.name);
                if arg.var {
                    self.error(arg.value.span, format!("the parameter `{}` is not `var`", param.name));
                    self.label(format!("`{}` does not change this argument", sig.name));
                    self.related(param.at, declared.clone());
                    let var = Span {
                        start: arg.var_at,
                        end: arg.value.span.start,
                    };
                    self.fix("remove `var`", var, "");
                }
                let reported = self.diags.len();
                let value = match done {
                    Some(value) => self.coerce(value, &param.ty),
                    None => {
                        self.function_argument_context(&arg.value, param.lent);
                        let value = self.expr_as(&arg.value, &param.ty);
                        self.next_lambda_is_block = false;
                        self.lent_fn_ok = false;
                        value
                    }
                };
                // Only a mismatch of the argument itself is about the parameter; an
                // error inside the argument is not.
                self.explain_mismatch(reported, arg.value.span, param.at, declared);
                value
            };
            out.push(Arg { kind: param.kind, expr });
        }
        if !missing.is_empty() {
            // An argument that found no parameter is likely the one that is missing.
            if paired {
                self.error(
                    span,
                    format!("this call of `{}` is missing {}", sig.name, missing.join(", ")),
                );
                self.label(format!("no value for {}", missing.join(", ")));
                match missing_at {
                    Some(at) if missing.len() == 1 => {
                        self.related(Some(at), format!("the parameter {} is declared here", missing[0]))
                    }
                    _ => self.related(sig.at.map(|at| at.name), format!("`{}` is declared here", sig.name)),
                }
            }
            return self.unknown(span);
        }

        self.exclusive(&out, None);
        self.mk_call(ExprKind::Call { func: id, args: out }, sig.ret, span)
    }

    /// The law of exclusivity for one call (Section 5.4): while the call can change
    /// something through a `var` argument, nothing else the call is given may reach the
    /// same thing. `callee` is the function value being called, if the call is of one.
    pub(super) fn exclusive(&mut self, args: &[Arg], callee: Option<&Expr>) {
        // Two `var` arguments that overlap.
        let writes: Vec<(Span, (LocalId, Vec<Step>))> = args
            .iter()
            .filter(|arg| arg.kind == ParamKind::Var)
            .filter_map(|arg| place_path(&arg.expr).map(|path| (arg.expr.span, path)))
            .collect();
        for (i, (first_span, first)) in writes.iter().enumerate() {
            for (second_span, second) in &writes[i + 1..] {
                if overlap(first, second) {
                    self.error(
                        *second_span,
                        "this argument overlaps another `var` argument of the same call",
                    );
                    self.label("the same thing, lent to be changed a second time");
                    self.related(Some(*first_span), "first lent to be changed here");
                    self.note("one thing cannot be changed through two names at once (Section 5.4)");
                    self.help("pass a copy as one of the two: `var copy := x`");
                }
            }
        }
        if writes.is_empty() {
            return;
        }
        // A block works on the variables around it, so a block the call is given must not
        // use a variable the call changes.
        let mut lent: Option<(Span, String)> = None;
        for arg in args {
            match &arg.expr.kind {
                ExprKind::Closure {
                    captures, block: true, ..
                } => {
                    for (_, (root, _)) in &writes {
                        if captures.contains(root) {
                            let name = self.locals[*root].name.clone();
                            self.error_help(
                                arg.expr.span,
                                format!("this block uses `{name}`, which the same call changes"),
                                format!(
                                    "`{name}` is passed as `var`, or is what the method changes, so nothing else may use it until the call returns (Section 5.4); work on a copy, or take what the block needs out of `{name}` before the call"
                                ),
                            );
                        }
                    }
                    if let Some(&function) = captures.iter().find(|capture| self.borrowed_fns.contains(capture)) {
                        lent = Some((arg.expr.span, self.locals[function].name.clone()));
                    }
                }
                ExprKind::Local(local) if self.borrowed_fns.contains(local) => {
                    lent = Some((arg.expr.span, self.locals[*local].name.clone()));
                }
                _ => {}
            }
        }
        if let Some(Expr {
            kind: ExprKind::Local(local),
            span,
            ..
        }) = callee
            && self.borrowed_fns.contains(local)
        {
            lent = Some((*span, self.locals[*local].name.clone()));
        }
        // A variable of the enclosing function can be reached by any block written there,
        // and a lent function may be one of them.
        if let Some((span, function)) = lent {
            for (_, (root, _)) in &writes {
                if matches!(self.locals[*root].kind, LocalKind::Capture { by_ref: true }) {
                    let name = self.locals[*root].name.clone();
                    self.error_help(
                        span,
                        format!("`{function}` may use `{name}`, which the same call changes"),
                        format!(
                            "`{name}` belongs to the enclosing function, and so may `{function}`, a function that was lent (Section 5.4); change a copy of `{name}` and assign it back after the call"
                        ),
                    );
                }
            }
        }
    }

    /// A call of a generic function: finds its type arguments from the receiver, the
    /// arguments and the expected type, makes the function for them, and calls it.
    /// `env` holds type arguments that are already known.
    pub(super) fn generic_call(
        &mut self,
        template: usize,
        receiver: Option<Expr>,
        mut env: HashMap<String, Ty>,
        args: &[ast::Arg],
        span: Span,
        expected: Option<&Ty>,
    ) -> Expr {
        let t = &self.templates[template];
        let decl = t.decl;
        let names: Vec<String> = t.params.iter().map(|p| p.name.clone()).collect();
        let implicit = t.implicit.clone();
        let display = t.display.clone();
        let at = (!t.prelude).then_some(decl.sig.name.span);
        let of_prelude = t.prelude;
        if let Some(receiver) = &receiver
            && let ast::FnTarget::Method { ty: Some(pattern), .. } = &decl.sig.target
        {
            let ty = receiver.ty.clone();
            self.bind(pattern, &ty, &names, &mut env);
        }
        let param_names: Vec<String> = decl.sig.params.iter().map(|p| p.name.name.clone()).collect();
        let reported = self.diags.len();
        // A generic function of the prelude that ends with `location: string` is told
        // where it was called (`check.rs`, `takes_location`).
        let located = of_prelude
            && decl.sig.params.last().is_some_and(|last| {
                last.name.name == "location"
                    && matches!(&last.ty.kind, ast::TypeKind::Named { path, args }
                        if path.len() == 1 && args.is_empty() && path[0].name == "string")
            });
        let (slots, paired) = self.match_args(&display, &param_names, args, at, located);
        let mut checked: Vec<Option<Expr>> = vec![None; slots.len()];
        for (index, param) in decl.sig.params.iter().enumerate() {
            let Some(arg) = slots[index] else { continue };
            if let Some(name) = implicit.get(&index) {
                // A parameter of interface type takes the argument's own type.
                let value = self.expr(&arg.value, None);
                if !value.ty.is_unknown() && value.ty != Ty::Never {
                    env.entry(name.clone()).or_insert_with(|| value.ty.clone());
                }
                checked[index] = Some(value);
                continue;
            }
            let hint = self.resolve_bound(&param.ty, &names, &env);
            self.argument_kind = super::param_kind(param.kind);
            let value = self.argument(&arg.value, &param.ty, hint.as_ref(), &names, &env);
            let ty = value.ty.clone();
            self.bind(&param.ty, &ty, &names, &mut env);
            checked[index] = Some(value);
        }
        // What the arguments leave open, the expected type may settle.
        if let (Some(expected), Some(ret)) = (expected, &decl.sig.ret)
            && names.iter().any(|name| !env.contains_key(name))
        {
            self.bind(ret, expected, &names, &mut env);
            let inner = super::exprs::peel(expected).clone();
            self.bind(ret, &inner, &names, &mut env);
        }
        let mut type_args = Vec::new();
        for name in &names {
            match env.get(name) {
                Some(ty) => type_args.push(ty.clone()),
                // The expected type failed to check, and that was reported.
                None if expected.is_some_and(|ty| ty.is_unknown()) => return self.unknown(span),
                // A mistake in an argument is why the type is not known, and it is
                // the mistake to correct.
                None if self.diags.len() > reported => return self.unknown(span),
                None => {
                    self.error_help(
                        span,
                        format!("the type parameter `{name}` of `{display}` is not known here"),
                        "give the result a type, as in `x: Stack<int> = ...` (Section 3.9)",
                    );
                    return self.unknown(span);
                }
            }
        }
        let Some(id) = self.instantiate(template, type_args, span) else {
            return self.unknown(span);
        };
        self.finish_call(id, receiver, slots, checked, paired, span)
    }

    /// Checks one argument of a generic call. `pattern` is the parameter's type as
    /// written, and `hint` is that type if its type parameters are already known.
    fn argument(
        &mut self,
        value: &ast::Expr,
        pattern: &ast::Type,
        hint: Option<&Ty>,
        names: &[String],
        env: &HashMap<String, Ty>,
    ) -> Expr {
        let ast::TypeKind::Fn {
            params, ret, errors, ..
        } = &pattern.kind
        else {
            return self.expr(value, hint);
        };
        if !matches!(value.kind, ast::ExprKind::Lambda { .. } | ast::ExprKind::Name(_)) {
            return self.expr(value, hint);
        }
        // The parameter is declared with a function type, so a borrowed one is lent.
        let lends = self.argument_kind == ParamKind::Borrow;
        // A function parameter: a lambda's parameter types come from the type arguments
        // found so far, and its return type may be what settles another of them.
        let mut wanted = Vec::new();
        for (kind, ty) in params {
            match self.resolve_bound(ty, names, env) {
                Some(resolved) => wanted.push((super::param_kind(*kind), resolved)),
                None => {
                    self.function_argument_context(value, lends);
                    let checked = self.expr(value, hint);
                    self.next_lambda_is_block = false;
                    self.lent_fn_ok = false;
                    return checked;
                }
            }
        }
        let ok = match ret {
            Some(ret) => self.resolve_bound(ret, names, env).unwrap_or(Ty::Unknown),
            None => Ty::Unit,
        };
        // The errors the function may fail with, when the declaration says: then a
        // lambda that fails with fewer of them is still one that may fail with them.
        let open = super::generics::errors_unbound(errors, names, env);
        let saved = std::mem::replace(&mut self.type_env, env.clone());
        let ret = if errors.is_empty() || open {
            ok
        } else if ok.is_unknown() {
            self.lambda_errors = match self.result_of(Ty::Unit, errors) {
                Ty::Result(_, set) => Some(set),
                _ => None,
            };
            ok
        } else {
            self.result_of(ok, errors)
        };
        self.type_env = saved;
        // With the errors still to be found, the whole of what the lambda returns is.
        let ret = if open { Ty::Unknown } else { ret };
        let expected = Ty::Fn(Box::new(FnTy { params: wanted, ret }));
        self.function_argument_context(value, lends);
        let checked = self.expr(value, Some(&expected));
        self.lambda_errors = None;
        self.next_lambda_is_block = false;
        self.lent_fn_ok = false;
        checked
    }

    /// A call of a generic type-level function. `on` is the type it is called on, when the
    /// call names it in full (`Stack<int>.new()`).
    pub(super) fn generic_type_level_call(
        &mut self,
        template: usize,
        on: Option<Ty>,
        args: &[ast::Arg],
        span: Span,
        expected: Option<&Ty>,
    ) -> Expr {
        let t = &self.templates[template];
        let decl = t.decl;
        let names: Vec<String> = t.params.iter().map(|p| p.name.clone()).collect();
        let mut env = HashMap::new();
        if let (Some(on), ast::FnTarget::TypeLevel(pattern)) = (&on, &decl.sig.target) {
            self.bind(pattern, on, &names, &mut env);
        }
        self.generic_call(template, None, env, args, span, expected)
    }
}
