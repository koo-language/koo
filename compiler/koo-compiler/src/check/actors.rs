//! Actors (Section 7.3).
//!
//! An actor is three struct types and a function for each of its methods. `Name` is the
//! handle, which holds the actor's address; `handle.send` is a value of a second type,
//! whose methods send without waiting; the third holds the actor's fields. The body of
//! a method is checked as a function that is given the handle as `self` and each field
//! as a parameter of the field's name, a `var` one if the field is `var`: that is all it
//! takes for a method to use its fields by bare name and to change only those it may.
//!
//! What carries a message from a handle to these functions and back is written by the
//! compiler as Koo (`derive.rs`), on top of the support library (`koo-rt/src/actor.rs`).

use std::collections::HashMap;

use koo_syntax::{Span, ast};

use super::{Checker, FnAt, FnSig, ParamSig, TypeName};
use crate::derive::{method_function, send_type, state_type};
use crate::hir::*;

impl<'a> Checker<'a> {
    /// Declares the types of an actor, before anything refers to them.
    pub(super) fn declare_actor(&mut self, decl: &'a ast::ActorDecl) {
        let name = &decl.name;
        if !decl.type_params.is_empty() || !decl.satisfies.is_empty() {
            self.unsupported(
                name.span,
                "generic actors, and actors that declare what they satisfy, are",
            );
            return;
        }
        let names = [name.name.clone(), send_type(&name.name), state_type(&name.name)];
        if names.iter().any(|name| self.is_type_name(name)) {
            self.already_declared(name.span, &name.name, &format!("type {}", name.name));
            return;
        }
        for name in names {
            let id = self.program.structs.len();
            self.program.structs.push(StructDef {
                name: name.clone(),
                fields: Vec::new(),
            });
            self.defaults.push(Vec::new());
            self.type_names.insert(name, TypeName::Struct(id));
        }
    }

    fn actor_struct(&self, name: &str) -> Option<StructId> {
        match self.type_names.get(name) {
            Some(TypeName::Struct(id)) => Some(*id),
            _ => None,
        }
    }

    /// Gives an actor's types their fields.
    pub(super) fn actor_fields(&mut self, decl: &'a ast::ActorDecl) {
        let name = &decl.name.name;
        let (Some(handle), Some(send), Some(state)) = (
            self.actor_struct(name),
            self.actor_struct(&send_type(name)),
            self.actor_struct(&state_type(name)),
        ) else {
            return;
        };
        // Which actor, and the process it lives in.
        let name_it = vec![
            FieldDef {
                name: "id".to_string(),
                ty: Ty::Int,
            },
            FieldDef {
                name: "home".to_string(),
                ty: Ty::Str,
            },
        ];
        self.program.structs[send].fields = name_it.clone();
        self.defaults[send] = vec![None, None];
        let mut fields = name_it;
        fields.push(FieldDef {
            name: "send".to_string(),
            ty: Ty::Struct(send),
        });
        self.program.structs[handle].fields = fields;
        self.defaults[handle] = vec![None, None, None];
        let mut fields: Vec<FieldDef> = Vec::new();
        let mut defaults = Vec::new();
        for field in &decl.fields {
            if let Some(first) = decl.fields.iter().find(|f| f.name.name == field.name.name)
                && first.name.span != field.name.span
                && fields.iter().any(|f| f.name == field.name.name)
            {
                self.error(
                    field.name.span,
                    format!("the field `{}` is declared twice", field.name.name),
                );
                self.related(Some(first.name.span), "first declared here");
                continue;
            }
            let ty = self.resolve_type(&field.ty);
            // A default is a constant, as a struct's is: it is what the field is when
            // the actor starts, and again when it starts over.
            let default = field
                .default
                .as_ref()
                .and_then(|default| self.const_expr(default, Some(&ty)).map(|e| self.coerce(e, &ty)));
            fields.push(FieldDef {
                name: field.name.name.clone(),
                ty,
            });
            defaults.push(default);
        }
        self.program.structs[state].fields = fields;
        self.defaults[state] = defaults;
    }

    /// Declares the function behind each method of an actor: `self`, then the fields,
    /// then the method's own parameters.
    pub(super) fn actor_methods(&mut self, decl: &'a ast::ActorDecl, bodies: &mut Vec<(FnId, &'a ast::FnDecl)>) {
        let name = &decl.name.name;
        let (Some(handle), Some(_)) = (self.actor_struct(name), self.actor_struct(&state_type(name))) else {
            return;
        };
        for method in &decl.methods {
            let sig = &method.decl.sig;
            if !sig.type_params.is_empty() {
                self.unsupported(sig.name.span, "generic methods of actors are");
                continue;
            }
            let function = method_function(name, &sig.name.name);
            if self.free_fns.contains_key(&function) {
                self.error(sig.name.span, format!("`{}` is already declared", sig.name.name));
                self.label("an actor has one method of each name");
                continue;
            }
            let (params, ret) = self.params_and_ret(&method.decl, &HashMap::new());
            let mut all = vec![ParamSig {
                name: "self".to_string(),
                kind: ParamKind::Borrow,
                ty: Ty::Struct(handle),
                default: None,
                lent: false,
                at: None,
            }];
            for field in &decl.fields {
                // A parameter of a field's name would hide the field from its own actor.
                if let Some(param) = sig.params.iter().find(|param| param.name.name == field.name.name) {
                    self.error(
                        param.name.span,
                        format!("`{}` is a field of this actor", param.name.name),
                    );
                    self.related(Some(field.name.span), "the field is declared here");
                    self.help("give the parameter another name: inside an actor, a field is used by its name alone");
                }
                all.push(ParamSig {
                    name: field.name.name.clone(),
                    kind: if field.mutable {
                        ParamKind::Var
                    } else {
                        ParamKind::Borrow
                    },
                    ty: self.resolve_type(&field.ty),
                    default: None,
                    lent: false,
                    at: Some(field.name.span.to(field.ty.span)),
                });
            }
            all.extend(params);
            let id = self.sigs.len();
            let shown = format!("{name}.{}", sig.name.name);
            self.free_fns.insert(function, id);
            self.sigs.push(FnSig {
                name: shown.clone(),
                receiver: None,
                params: all,
                ret: ret.clone(),
                at: Some(FnAt::of(sig)),
            });
            self.program.functions.push(Function {
                // The handle has a method of the same name, which is another function.
                name: format!("actor.{shown}"),
                params: 0,
                locals: Vec::new(),
                ret,
                body: Block::default(),
                span: sig.span,
                ..Function::default()
            });
            self.actor_of.insert(id, decl);
            bodies.push((id, &method.decl));
        }
    }

    /// `reset()` inside a method of an actor that has a method `reset`: an ordinary call,
    /// within the same message, of the function behind the method, on the same fields.
    pub(super) fn own_method_call(&mut self, name: &ast::Ident, args: &[ast::Arg], span: Span) -> Option<Expr> {
        let actor = self.actor?;
        if !actor
            .methods
            .iter()
            .any(|method| method.decl.sig.name.name == name.name)
        {
            return None;
        }
        let id = *self.free_fns.get(&method_function(&actor.name.name, &name.name))?;
        if self.lambda.is_some() {
            self.error(
                name.span,
                format!(
                    "`{}` is a method of this actor, and cannot be called from inside a lambda",
                    name.name
                ),
            );
            self.help("a lambda holds copies of the actor's fields, so the call would change nothing; call it from the method itself");
            return Some(self.unknown(span));
        }
        let here = |text: &str| ast::Expr {
            kind: ast::ExprKind::Name(ast::Ident {
                name: text.to_string(),
                span: name.span,
            }),
            span: name.span,
        };
        let mut all = vec![ast::Arg {
            name: None,
            var: false,
            var_at: name.span.start,
            value: here("self"),
        }];
        for field in &actor.fields {
            all.push(ast::Arg {
                name: None,
                var: field.mutable,
                var_at: name.span.start,
                value: here(&field.name.name),
            });
        }
        all.extend(args.iter().cloned());
        Some(self.user_call(id, None, &all, span))
    }
}
