//! Maps and sets. Both are generic types that the prelude declares in Koo; this is the
//! part of them that is syntax: `[K: V]`, map literals, `m[k]`, `m[k] = v`, `m.len` and
//! `for (k, v) in m` (Section 4.5).

use koo_syntax::Span;
use koo_syntax::ast;

use super::Checker;
use super::generics::OwnerKey;
use crate::hir::*;

impl<'a> Checker<'a> {
    /// The key and value types of a map type.
    pub(super) fn map_types(&self, ty: &Ty) -> Option<(Ty, Ty)> {
        match self.instance_of.get(ty) {
            Some((name, args)) if name == "Map" && args.len() == 2 && self.is_prelude_type(ty) => {
                Some((args[0].clone(), args[1].clone()))
            }
            _ => None,
        }
    }

    /// Whether a type is made from a generic type of the prelude, whose fields are its own
    /// business (Section 4.6: a field that begins with `_` is visible only in its package).
    pub(super) fn is_prelude_type(&self, ty: &Ty) -> bool {
        self.instance_of
            .get(ty)
            .is_some_and(|(name, _)| self.generic_types.get(name).is_some_and(|g| g.prelude))
    }

    /// Calls a method of a prelude type, such as `Map.get`, with arguments that are checked
    /// already. The receiver is the first argument.
    pub(super) fn prelude_call(&mut self, method: &str, args: Vec<Expr>, span: Span) -> Expr {
        let ty = args[0].ty.clone();
        let Some((generic, type_args)) = self.instance_of.get(&ty).cloned() else {
            return self.unknown(span);
        };
        let Some(&template) = self
            .generic_methods
            .get(&(OwnerKey::Generic(generic), method.to_string()))
        else {
            return self.unknown(span);
        };
        let Some(id) = self.instantiate(template, type_args, span) else {
            return self.unknown(span);
        };
        let sig = self.sigs[id].clone();
        let kinds = sig.receiver.iter().chain(&sig.params).map(|p| (p.kind, p.ty.clone()));
        let mut out = Vec::new();
        for (expr, (kind, param)) in args.into_iter().zip(kinds) {
            if kind == ParamKind::Var {
                self.require_mutable_place(&expr, "change");
                out.push(Arg { kind, expr });
            } else {
                out.push(Arg {
                    kind,
                    expr: self.coerce(expr, &param),
                });
            }
        }
        Expr {
            kind: ExprKind::Call { func: id, args: out },
            ty: sig.ret,
            span,
        }
    }

    /// What a `for` loop walks: a list as it is, the entries of a map, the items of a set.
    pub(super) fn iterable(&mut self, value: Expr) -> Expr {
        let span = value.span;
        match self.instance_of.get(&value.ty) {
            Some((name, _)) if name == "Map" && self.is_prelude_type(&value.ty) => {
                self.prelude_call("entries", vec![value], span)
            }
            Some((name, _)) if name == "Set" && self.is_prelude_type(&value.ty) => {
                self.prelude_call("items", vec![value], span)
            }
            _ => value,
        }
    }

    /// `["a": 1, "b": 2]` and `[:]`.
    pub(super) fn map_literal(
        &mut self,
        entries: &[(ast::Expr, ast::Expr)],
        span: Span,
        expected: Option<&Ty>,
    ) -> Expr {
        if !self.generic_types.contains_key("Map") {
            self.unsupported(span, "maps are");
            return self.unknown(span);
        }
        let hint = expected.map(super::exprs::peel).and_then(|ty| self.map_types(ty));
        let (mut key_ty, mut value_ty) = match hint {
            Some((key, value)) => (Some(key), Some(value)),
            None => (None, None),
        };
        let mut keys = Vec::new();
        let mut values = Vec::new();
        for (key, value) in entries {
            let key = self.expr(key, key_ty.as_ref());
            let key = match &key_ty {
                Some(ty) => self.coerce(key, ty),
                None => {
                    key_ty = Some(key.ty.clone());
                    key
                }
            };
            let value = self.expr(value, value_ty.as_ref());
            let value = match &value_ty {
                Some(ty) => self.coerce(value, ty),
                None => {
                    value_ty = Some(value.ty.clone());
                    value
                }
            };
            keys.push(key);
            values.push(value);
        }
        if expected.is_some_and(|ty| ty.is_unknown()) && entries.is_empty() {
            // The expected type failed to check, and that was reported.
            return self.unknown(span);
        }
        let (Some(key_ty), Some(value_ty)) = (key_ty, value_ty) else {
            self.error_help(
                span,
                "the types of `[:]` are not known here",
                "give the binding a type: `m: [string: int] = [:]`",
            );
            return self.unknown(span);
        };
        if key_ty.is_unknown() || value_ty.is_unknown() {
            return self.unknown(span);
        }
        let map_ty = self.instantiate_type("Map", vec![key_ty.clone(), value_ty.clone()], span);
        if map_ty.is_unknown() {
            return self.unknown(span);
        }
        let function = if entries.is_empty() { "new" } else { "_from_lists" };
        let Some(&template) = self
            .generic_type_fns
            .get(&(OwnerKey::Generic("Map".to_string()), function.to_string()))
        else {
            return self.unknown(span);
        };
        let Some(id) = self.instantiate(template, vec![key_ty.clone(), value_ty.clone()], span) else {
            return self.unknown(span);
        };
        let sig = self.sigs[id].clone();
        let mut args = Vec::new();
        if !entries.is_empty() {
            let lists = [
                Expr {
                    kind: ExprKind::ListLit(keys),
                    ty: Ty::list(key_ty),
                    span,
                },
                Expr {
                    kind: ExprKind::ListLit(values),
                    ty: Ty::list(value_ty),
                    span,
                },
            ];
            for (expr, param) in lists.into_iter().zip(&sig.params) {
                args.push(Arg { kind: param.kind, expr });
            }
        }
        Expr {
            kind: ExprKind::Call { func: id, args },
            ty: sig.ret,
            span,
        }
    }

    /// Gives every generic type that has a `to_string` its text form (Section 4.6): the
    /// function is made for each set of type arguments the type is used with. Returns
    /// whether any were made, since making one may use more types.
    pub(super) fn generic_to_strings(&mut self) -> bool {
        let mut made = false;
        let instances: Vec<(Ty, (String, Vec<Ty>))> =
            self.instance_of.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        for (ty, (generic, args)) in instances {
            if self.program.to_string.contains_key(&ty) {
                continue;
            }
            let key = (OwnerKey::Generic(generic), "to_string".to_string());
            let Some(&template) = self.generic_methods.get(&key) else {
                continue;
            };
            let sig = &self.templates[template].decl.sig;
            if !sig.params.is_empty() || !sig.type_params.is_empty() {
                continue;
            }
            if let Some(id) = self.instantiate(template, args, Span::default())
                && self.sigs[id].ret == Ty::Str
            {
                self.program.to_string.insert(ty, id);
                made = true;
            }
        }
        made
    }
}
