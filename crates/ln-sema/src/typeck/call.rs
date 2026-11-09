//! Value paths, calls, method resolution and struct literals.

use super::FnCtx;
use crate::defs::*;
use crate::resolve::ModRes;
use crate::thir::{self, Arg, Callee, Coercion, Expr, ExprKind, ReadMode};
use crate::ty::{ArrKind, TraitRef, Ty};
use ln_ast as ast;
use ln_diag::{Diagnostic, Span, codes::name, codes::trait_, codes::ty as tyc};

/// What a value path refers to.
pub(crate) enum ValueRes {
    Local(thir::LocalId),
    Fn { def: DefId, args: Vec<Ty> },
    Const(DefId),
    Static(DefId),
    Ctor { adt: DefId, variant: usize, args: Vec<Ty> },
    TraitMethod { trait_ref: TraitRef, self_ty: Ty, def: DefId, method_args: Vec<Ty> },
}

/// A resolved method candidate.
struct MethodPick {
    /// The method's definition.
    def: DefId,
    /// All generic args for the method (parent args first).
    args: Vec<Ty>,
    /// Trait call info when the method comes from a trait.
    trait_call: Option<(TraitRef, Ty)>,
}

impl<'s, 'a> FnCtx<'s, 'a> {
    // ===================================================================== generic args

    /// Fresh variables or explicit arguments for an item's own generics.
    fn own_args(&mut self, def: DefId, explicit: Option<&[ast::Ty]>, span: Span) -> Vec<Ty> {
        let n = self.s.generics(def).map_or(0, |g| g.own_count());
        match explicit {
            Some(ts) => {
                if ts.len() != n {
                    let name = self.s.p.def(def).name.clone();
                    self.err(Diagnostic::error(tyc::GENERIC_COUNT, span, format!("`{name}` takes {n} generic argument(s) but {} were supplied", ts.len())));
                    return (0..n).map(|_| Ty::Error).collect();
                }
                ts.iter().map(|t| self.lower_local_ty(t)).collect()
            }
            None => (0..n).map(|_| self.new_var(span)).collect(),
        }
    }

    /// Instantiate a definition's predicates with `args` and record them.
    fn oblige_preds(&mut self, def: DefId, args: &[Ty], span: Span) {
        let mut cur = Some(def);
        while let Some(d) = cur {
            if let Some(g) = self.s.generics(d) {
                for p in g.preds.clone() {
                    let ty = p.ty.subst(args);
                    let tr = p.trait_ref.subst(args);
                    self.oblige(ty, tr, span);
                }
            }
            cur = match self.s.p.def(d).kind {
                DefKind::Fn => self.s.p.def(d).parent.filter(|p| self.s.p.def(*p).kind == DefKind::Impl),
                _ => None,
            };
        }
    }

    // ===================================================================== value paths

    pub(crate) fn resolve_value_path(&mut self, p: &ast::Path, span: Span) -> Option<ValueRes> {
        let r = self.resolve_value_path_inner(p, span)?;
        let last = p.segments.last().unwrap().ident.span;
        match &r {
            ValueRes::Local(l) => self.record_local(last, *l),
            ValueRes::Fn { def, .. } => {
                self.record_def(last, *def);
                self.check_method_privacy(*def, last);
            }
            ValueRes::Const(def) | ValueRes::Static(def) | ValueRes::TraitMethod { def, .. } => self.record_def(last, *def),
            ValueRes::Ctor { adt, variant, .. } => {
                let v = self.s.p.adt(*adt).variants[*variant].def;
                self.record_def(last, v);
            }
        }
        Some(r)
    }

    /// Inherent methods and associated functions are private to the impl's
    /// module (and its children) unless marked `pub`. Trait methods are as
    /// visible as their trait.
    fn check_method_privacy(&mut self, def: DefId, span: Span) {
        let d = self.s.p.def(def);
        let Some(parent) = d.parent else { return };
        if d.is_pub || self.s.p.def(parent).kind != DefKind::Impl || self.s.p.impl_data(parent).trait_ref.is_some() {
            return;
        }
        if self.s.is_ancestor_or_self(d.module, self.module()) {
            return;
        }
        let what = if self.s.p.fn_data(def).has_self { "method" } else { "associated function" };
        let (name, dspan) = (d.name.clone(), d.span);
        self.err(
            Diagnostic::error(name::PRIVATE, span, format!("{what} `{name}` is private"))
                .with_secondary(dspan, "defined here")
                .with_help("mark it `pub` to use it from other modules"),
        );
    }

    /// `name::not_a_value` for a path (`shown`) that names `def`, an item
    /// that exists but is not a value (a module, a type, a trait, ...).
    fn not_a_value(&self, def: DefId, shown: &str, span: Span) -> Diagnostic {
        let kind = self.s.p.def(def).kind;
        let d = Diagnostic::error(name::NOT_A_VALUE, span, format!("expected a value, found {} `{shown}`", kind.describe()));
        if kind == DefKind::Struct {
            return d.with_help(format!("build a value with a struct literal: `{shown} {{ ... }}`"));
        }
        d
    }

    fn resolve_value_path_inner(&mut self, p: &ast::Path, span: Span) -> Option<ValueRes> {
        let segs = &p.segments;
        if p.is_single()
            && let Some(l) = self.lookup_local(&segs[0].ident.name, segs[0].ident.ctx)
        {
            return Some(ValueRes::Local(l));
        }
        // `Self::f`, `T::f`
        if segs.len() == 2 {
            let first = &segs[0].ident.name;
            let ty = if first == "Self" {
                self.cx.self_ty.clone()
            } else {
                self.cx.generic_names.iter().rposition(|n| n == first).map(|i| Ty::Param(i as u32, first.clone()))
            };
            if let Some(ty) = ty {
                if first == "Self"
                    && let Ty::Adt(d, args) = &ty
                    && self.s.p.is_enum(*d)
                    && let Some(vi) = self.s.p.adt(*d).variant_index(&segs[1].ident.name)
                {
                    return Some(ValueRes::Ctor { adt: *d, variant: vi, args: args.clone() });
                }
                return self.assoc_fn(&ty, &segs[1], span);
            }
        }
        let idents: Vec<ast::Ident> = segs.iter().map(|s| s.ident.clone()).collect();
        let module = self.module();
        let res = self.s.resolve_prefix(module, &idents, Ns::Value);
        let Some((res, used)) = res else {
            if idents.len() == 1 {
                // Maybe it names a type, e.g. a named-field struct used as a value.
                let d = match self.s.lookup_ident(module, &idents[0], Ns::Type) {
                    Some(ModRes::Def(def)) => self.not_a_value(def, &idents[0].name, span),
                    Some(ModRes::Prim(_)) => Diagnostic::error(name::NOT_A_VALUE, span, format!("expected a value, found builtin type `{}`", idents[0].name)),
                    None => self.s.unresolved_diag(module, &idents[0], Ns::Value),
                };
                self.err(d);
            }
            return None;
        };
        let ty_prefix = |fcx: &mut Self, upto: usize| -> Option<Ty> {
            match &res {
                ModRes::Prim(t) => Some(t.clone()),
                ModRes::Def(d) => {
                    let kind = fcx.s.p.def(*d).kind;
                    match kind {
                        DefKind::Struct | DefKind::Enum => {
                            let args = fcx.own_args(*d, segs[upto - 1].args.as_deref(), span);
                            Some(Ty::Adt(*d, args))
                        }
                        DefKind::TypeAlias => {
                            let path = ast::Path { segments: segs[..upto].to_vec(), span: p.span, id: p.id };
                            let t = ast::Ty { kind: ast::TyKind::Path(path), span: p.span, id: p.id };
                            Some(fcx.lower_local_ty(&t))
                        }
                        k => {
                            let prefix = segs[..upto].iter().map(|s| s.ident.name.as_str()).collect::<Vec<_>>().join("::");
                            let sp = segs[upto - 1].ident.span;
                            fcx.err(Diagnostic::error(
                                name::NOT_A_TYPE,
                                sp,
                                format!("expected a type, trait or module before `::{}`, found {} `{prefix}`", segs[upto].ident.name, k.describe()),
                            ));
                            None
                        }
                    }
                }
            }
        };
        if used == idents.len() {
            let ModRes::Def(def) = res else {
                self.err(Diagnostic::error(name::NOT_A_VALUE, span, format!("expected a value, found type `{}`", p.to_string_lossy())));
                return None;
            };
            let d = self.s.p.def(def);
            return match d.kind {
                DefKind::Fn => {
                    if d.parent.is_some() {
                        // Methods are not module items, so module paths never
                        // reach them; report rather than fail silently.
                        let msg = format!("`{}` is a method; call it through its type, e.g. `Type::{}`", p.to_string_lossy(), d.name);
                        self.err(Diagnostic::error(name::NOT_A_VALUE, span, msg));
                        return None;
                    }
                    let args = self.own_args(def, segs.last().unwrap().args.as_deref(), span);
                    Some(ValueRes::Fn { def, args })
                }
                DefKind::Const => Some(ValueRes::Const(def)),
                DefKind::Static => Some(ValueRes::Static(def)),
                DefKind::Struct if self.s.p.adt(def).variants[0].ctor == CtorKind::Named => {
                    // Reached through the type namespace: a named-field struct
                    // is not a value.
                    let d = self.not_a_value(def, &p.to_string_lossy(), span);
                    self.err(d);
                    None
                }
                DefKind::Struct => {
                    let args = self.own_args(def, segs.last().unwrap().args.as_deref(), span);
                    Some(ValueRes::Ctor { adt: def, variant: 0, args })
                }
                DefKind::Variant => {
                    let DefData::Variant { enum_id, index } = d.data else { unreachable!() };
                    // Generic args may sit on the enum segment: `Option::<i32>::None`.
                    let explicit = if segs.len() >= 2 { segs[segs.len() - 2].args.as_deref() } else { None };
                    let explicit = explicit.or(segs.last().unwrap().args.as_deref());
                    let args = self.own_args(enum_id, explicit, span);
                    Some(ValueRes::Ctor { adt: enum_id, variant: index, args })
                }
                _ => {
                    let d = self.not_a_value(def, &p.to_string_lossy(), span);
                    self.err(d);
                    None
                }
            };
        }
        // Type-relative: `Type::assoc` or `Trait::method`.
        if used + 1 != idents.len() {
            self.err(Diagnostic::error(name::UNRESOLVED, idents[used].span, format!("cannot find `{}`", idents[used].name)));
            return None;
        }
        let last = &segs[used];
        if let ModRes::Def(d) = &res
            && self.s.p.def(*d).kind == DefKind::Trait
        {
            let tid = *d;
            let td = self.s.p.trait_data(tid).clone();
            let Some(m) = td.methods.iter().find(|m| m.name == last.ident.name) else {
                self.err(Diagnostic::error(tyc::NO_METHOD, last.ident.span, format!("trait `{}` has no method `{}`", self.s.p.def(tid).name, last.ident.name)));
                return None;
            };
            let targs = self.own_args(tid, segs[used - 1].args.as_deref(), span);
            let self_ty = self.new_var(span);
            let method_args = self.own_args(m.def, last.args.as_deref(), span);
            let tr = TraitRef { trait_id: tid, args: targs };
            self.oblige(self_ty.clone(), tr.clone(), span);
            return Some(ValueRes::TraitMethod { trait_ref: tr, self_ty, def: m.def, method_args });
        }
        let ty = ty_prefix(self, used)?;
        self.assoc_fn(&ty, last, span)
    }

    /// `Type::name` lookup: inherent impls, then traits.
    fn assoc_fn(&mut self, ty: &Ty, seg: &ast::PathSegment, span: Span) -> Option<ValueRes> {
        let name = &seg.ident.name;
        // Enum variants through an alias or `Self`.
        if let Ty::Adt(d, args) = self.shallow(ty)
            && self.s.p.is_enum(d)
            && let Some(vi) = self.s.p.adt(d).variant_index(name)
        {
            return Some(ValueRes::Ctor { adt: d, variant: vi, args });
        }
        if let Some((def, parent_args)) = self.inherent_method(ty, name, false) {
            let mut args = parent_args;
            let own = self.own_args(def, seg.args.as_deref(), span);
            args.extend(own);
            return Some(ValueRes::Fn { def, args });
        }
        match self.trait_method(ty, name, false, span) {
            Some((tr, mdef)) => {
                let method_args = self.own_args(mdef, seg.args.as_deref(), span);
                Some(ValueRes::TraitMethod { trait_ref: tr, self_ty: ty.clone(), def: mdef, method_args })
            }
            None => {
                let t = self.ty_str(ty);
                self.err(Diagnostic::error(tyc::NO_METHOD, seg.ident.span, format!("no function or associated item named `{name}` found for `{t}`")));
                None
            }
        }
    }

    /// Find an inherent method on `ty`. Returns the method and the impl's args.
    fn inherent_method(&mut self, ty: &Ty, name: &str, need_self: bool) -> Option<(DefId, Vec<Ty>)> {
        let impls = self.s.p.impls.clone();
        for imp in impls {
            let DefData::Impl(d) = &self.s.p.def(imp).data else { continue };
            if d.trait_ref.is_some() {
                continue;
            }
            let Some(&m) = d.methods.get(name) else { continue };
            if need_self && !self.s.p.fn_data(m).has_self {
                continue;
            }
            if let Some(args) = self.match_impl(imp, ty) {
                return Some((m, args));
            }
        }
        None
    }

    /// Unify an impl's self type (with fresh variables for its generics)
    /// against `ty`; commits and returns the impl args on success.
    fn match_impl(&mut self, imp: DefId, ty: &Ty) -> Option<Vec<Ty>> {
        let d = self.s.p.impl_data(imp).clone();
        let snapshot = (self.vars.clone(), self.lens.clone());
        let args: Vec<Ty> = (0..d.generics.count()).map(|_| self.new_var(Span::DUMMY)).collect();
        let self_ty = d.self_ty.subst(&args);
        if self.unify(&self_ty, ty) {
            // Impl bounds must hold (checked later as obligations).
            for p in &d.generics.preds {
                let pty = self.resolve(&p.ty.subst(&args));
                let ptr = p.trait_ref.subst(&args);
                if !pty.has_infer()
                    && !self.s.p.implements(&pty, &TraitRef { trait_id: ptr.trait_id, args: ptr.args.iter().map(|a| self.resolve(a)).collect() }, &self.env)
                {
                    (self.vars, self.lens) = snapshot;
                    return None;
                }
            }
            Some(args)
        } else {
            (self.vars, self.lens) = snapshot;
            None
        }
    }

    /// Find a trait method `name` applicable to `ty`. Returns the trait ref and method def.
    fn trait_method(&mut self, ty: &Ty, name: &str, need_self: bool, span: Span) -> Option<(TraitRef, DefId)> {
        let ty = self.resolve(ty);
        let mut found: Vec<(TraitRef, DefId)> = Vec::new();
        let traits: Vec<DefId> = self.s.p.defs.iter().enumerate().filter(|(_, d)| d.kind == DefKind::Trait).map(|(i, _)| DefId(i as u32)).collect();
        for tid in traits {
            let td = self.s.p.trait_data(tid);
            let Some(m) = td.methods.iter().find(|m| m.name == name) else { continue };
            let mdef = m.def;
            if need_self && !self.s.p.fn_data(mdef).has_self {
                continue;
            }
            // Bounds in scope (generic params and `Self`).
            if let Some(p) = self.env.iter().find(|p| p.ty == ty && p.trait_ref.trait_id == tid) {
                found.push((p.trait_ref.clone(), mdef));
                continue;
            }
            if ty.has_infer() {
                continue;
            }
            let own = td.generics.names.len();
            if own == 0 {
                let tr = TraitRef { trait_id: tid, args: vec![] };
                if self.s.p.implements(&ty, &tr, &self.env) {
                    found.push((tr, mdef));
                }
                continue;
            }
            // Generic trait: take the args from a matching impl.
            for imp in self.s.p.impls.clone() {
                let DefData::Impl(d) = &self.s.p.def(imp).data else { continue };
                let Some(itr) = &d.trait_ref else { continue };
                if itr.trait_id != tid {
                    continue;
                }
                let itr = itr.clone();
                if let Some(args) = self.match_impl(imp, &ty) {
                    found.push((itr.subst(&args), mdef));
                    break;
                }
            }
        }
        if found.len() > 1 {
            let names: Vec<String> = found.iter().map(|(t, _)| format!("`{}`", self.s.p.def(t.trait_id).name)).collect();
            self.err(
                Diagnostic::error(trait_::AMBIGUOUS_METHOD, span, format!("multiple traits provide a method named `{name}`: {}", names.join(", ")))
                    .with_help("call it through the trait: `Trait::method(value, ...)`"),
            );
        }
        found.into_iter().next()
    }

    // ===================================================================== path expressions

    pub(crate) fn check_path_value(&mut self, p: &ast::Path, expect: Option<&Ty>, span: Span) -> Expr {
        let Some(r) = self.resolve_value_path(p, span) else { return self.error_expr(span) };
        match r {
            ValueRes::Local(l) => {
                let ty = self.locals[l.0 as usize].ty.clone();
                let place = self.mk(ExprKind::Local(l), ty, span);
                self.read(place, ReadMode::Move)
            }
            ValueRes::Const(def) => {
                let DefData::Const(c) = &self.s.p.def(def).data else { unreachable!() };
                let ty = c.ty.clone();
                match c.value.clone() {
                    Some(v) => self.mk(ExprKind::Lit(v), ty, span),
                    None => self.error_expr(span),
                }
            }
            ValueRes::Static(def) => {
                let ty = match &self.s.p.def(def).data {
                    DefData::Static(s) => s.ty.clone(),
                    _ => Ty::Error,
                };
                let place = self.mk(ExprKind::Static(def), ty, span);
                self.read(place, ReadMode::Copy)
            }
            ValueRes::Ctor { adt, variant, args } => {
                let v = &self.s.p.adt(adt).variants[variant];
                match v.ctor {
                    CtorKind::Unit => {
                        let ty = Ty::Adt(adt, args.clone());
                        if let Some(e) = expect {
                            let _ = self.unify(&ty, e);
                        }
                        self.mk(ExprKind::Adt { def: adt, args, variant, fields: vec![] }, ty, span)
                    }
                    _ => {
                        let n = v.name.clone();
                        self.err(
                            Diagnostic::error(name::NOT_A_VALUE, span, format!("`{n}` must be called with its fields"))
                                .with_help(format!("write `{n}(...)` or `{n} {{ ... }}`")),
                        );
                        self.error_expr(span)
                    }
                }
            }
            ValueRes::Fn { def, args } => {
                let fd = self.s.p.fn_data(def).clone();
                // The pointer type records each parameter's mode, so calls
                // through it pass arguments exactly as a direct call would.
                let params: Vec<(ParamMode, Ty)> = fd.sig.params.iter().map(|p| (p.mode, p.ty.subst(&args))).collect();
                let ret = fd.sig.ret.subst(&args);
                if self.s.p.def(def).attrs.intrinsic {
                    self.err(Diagnostic::error(tyc::NOT_CALLABLE, span, "intrinsics cannot be used as function pointers; wrap the call in a function"));
                } else if fd.abi == Abi::C && fd.sig.params.iter().any(|p| p.mode != ParamMode::Take) {
                    // `extern` functions take every argument by value.
                    self.err(Diagnostic::error(
                        tyc::NOT_CALLABLE,
                        span,
                        "an `extern` function with `&` or `&mut` parameters cannot be used as a function pointer",
                    ));
                }
                self.oblige_preds(def, &args, span);
                self.mk(ExprKind::FnRef { def, args }, Ty::FnPtr(params, Box::new(ret)), span)
            }
            ValueRes::TraitMethod { .. } => {
                self.err(Diagnostic::error(tyc::NOT_CALLABLE, span, "trait methods cannot be used as function pointers; call them instead"));
                self.error_expr(span)
            }
        }
    }

    // ===================================================================== calls

    pub(crate) fn check_call(&mut self, f: &ast::Expr, args: &[ast::Expr], expect: Option<&Ty>, span: Span) -> Expr {
        if let ast::ExprKind::Path(p) = &f.kind {
            let local_fnptr = p.is_single() && self.lookup_local(&p.segments[0].ident.name, p.segments[0].ident.ctx).is_some();
            if !local_fnptr {
                let Some(r) = self.resolve_value_path(p, f.span) else {
                    for a in args {
                        let _ = self.check_expr(a, None);
                    }
                    return self.error_expr(span);
                };
                return self.call_resolved(r, args, expect, f.span, span);
            }
        }
        // Call through a function pointer value.
        let callee = self.check_expr(f, None);
        let ct = self.shallow(&callee.ty);
        match ct {
            Ty::FnPtr(params, ret) => {
                let sig: Vec<(Ty, ParamMode)> = params.iter().map(|(m, t)| (t.clone(), *m)).collect();
                let a = self.check_args(&sig, false, args, span);
                self.mk(ExprKind::Call { callee: Callee::Indirect(Box::new(callee)), args: a }, *ret, span)
            }
            Ty::Error => self.error_expr(span),
            t => {
                self.err(Diagnostic::error(tyc::NOT_CALLABLE, f.span, format!("expected a function, found `{}`", self.ty_str(&t))));
                self.error_expr(span)
            }
        }
    }

    fn call_resolved(&mut self, r: ValueRes, args: &[ast::Expr], expect: Option<&Ty>, fspan: Span, span: Span) -> Expr {
        match r {
            ValueRes::Fn { def, args: gargs } => {
                let fd = self.s.p.fn_data(def).clone();
                let sig: Vec<(Ty, ParamMode)> = fd.sig.params.iter().map(|p| (p.ty.subst(&gargs), p.mode)).collect();
                let ret = fd.sig.ret.subst(&gargs);
                if let Some(e) = expect {
                    // Help inference flow from the expected result.
                    let snapshot = (self.vars.clone(), self.lens.clone());
                    if !self.unify(&ret, e) {
                        (self.vars, self.lens) = snapshot;
                    }
                }
                if fd.is_unsafe {
                    let n = self.s.p.path_of(def);
                    let what = if fd.abi == Abi::C { format!("calling extern function `{n}`") } else { format!("calling unsafe function `{n}`") };
                    self.require_unsafe(fspan, &what);
                }
                let a = self.check_args(&sig, fd.variadic, args, span);
                self.oblige_preds(def, &gargs, fspan);
                let callee = if self.s.p.def(def).attrs.intrinsic {
                    Callee::Intrinsic { def, name: self.s.p.def(def).name.clone(), args: gargs }
                } else {
                    Callee::Fn { def, args: gargs }
                };
                self.mk(ExprKind::Call { callee, args: a }, ret, span)
            }
            ValueRes::TraitMethod { trait_ref, self_ty, def, method_args } => {
                let mut all = vec![self_ty.clone()];
                all.extend(trait_ref.args.iter().cloned());
                all.extend(method_args.iter().cloned());
                let fd = self.s.p.fn_data(def).clone();
                let sig: Vec<(Ty, ParamMode)> = fd.sig.params.iter().map(|p| (p.ty.subst(&all), p.mode)).collect();
                let ret = fd.sig.ret.subst(&all);
                let a = self.check_args(&sig, false, args, span);
                self.oblige_preds(def, &all, fspan);
                let callee = Callee::Trait { trait_ref, self_ty, method: self.s.p.def(def).name.clone(), method_args };
                self.mk(ExprKind::Call { callee, args: a }, ret, span)
            }
            ValueRes::Ctor { adt, variant, args: gargs } => {
                let v = self.s.p.adt(adt).variants[variant].clone();
                // Constructing a tuple struct names every field.
                if v.ctor == CtorKind::Tuple {
                    for i in 0..v.fields.len() {
                        self.check_field_privacy(adt, i, fspan);
                    }
                }
                let ty = Ty::Adt(adt, gargs.clone());
                if let Some(e) = expect {
                    let snapshot = (self.vars.clone(), self.lens.clone());
                    if !self.unify(&ty, e) {
                        (self.vars, self.lens) = snapshot;
                    }
                }
                if v.ctor != CtorKind::Tuple {
                    let msg = if v.ctor == CtorKind::Unit {
                        format!("`{}` takes no fields", v.name)
                    } else {
                        format!("`{}` has named fields; use `{} {{ ... }}`", v.name, v.name)
                    };
                    self.err(Diagnostic::error(tyc::NOT_CALLABLE, fspan, msg));
                    return self.error_expr(span);
                }
                if v.fields.len() != args.len() {
                    self.err(Diagnostic::error(tyc::ARG_COUNT, span, format!("`{}` has {} field(s) but {} were supplied", v.name, v.fields.len(), args.len())));
                }
                let mut fields = Vec::new();
                for (i, (f, a)) in v.fields.iter().zip(args).enumerate() {
                    let ft = f.ty.subst(&gargs);
                    let x = self.check_expr(a, Some(&ft));
                    fields.push((i, self.coerce(x, &ft, a.span)));
                }
                self.mk(ExprKind::Adt { def: adt, args: gargs, variant, fields }, ty, span)
            }
            ValueRes::Local(_) | ValueRes::Const(_) | ValueRes::Static(_) => {
                self.err(Diagnostic::error(tyc::NOT_CALLABLE, fspan, "this value is not callable"));
                self.error_expr(span)
            }
        }
    }

    /// Check call arguments against parameter types and modes.
    pub(crate) fn check_args(&mut self, sig: &[(Ty, ParamMode)], variadic: bool, args: &[ast::Expr], span: Span) -> Vec<Arg> {
        if args.len() < sig.len() || (!variadic && args.len() > sig.len()) {
            self.err(Diagnostic::error(tyc::ARG_COUNT, span, format!("this function takes {} argument(s) but {} were supplied", sig.len(), args.len())));
        }
        let mut out = Vec::new();
        for (i, a) in args.iter().enumerate() {
            // `&x` / `&mut x` say how the argument is lent; check the marker,
            // then check what it lends.
            let (marker, a) = match &a.kind {
                ast::ExprKind::Borrow { mutable, expr } => (Some(*mutable), &**expr),
                _ => (None, a),
            };
            self.check_marker(marker, sig.get(i).map(|(_, m)| *m), a, args[i].span);
            match sig.get(i) {
                Some((pt, ParamMode::InOut)) => out.push(self.check_inout_arg(a, pt)),
                Some((pt, ParamMode::Borrow)) => {
                    let expect = pt.strip_bang().clone();
                    let v = self.check_expr_borrow(a, Some(&expect));
                    let v = self.coerce_arg(v, &expect, a.span);
                    out.push(Arg::Value(v));
                }
                Some((pt, ParamMode::Take)) => {
                    // The value moves into the call (or is copied, if Dull).
                    let expect = pt.strip_bang().clone();
                    let v = self.check_expr(a, Some(&expect));
                    let v = self.coerce_arg(v, &expect, a.span);
                    out.push(Arg::Value(v));
                }
                None => {
                    let v = self.check_expr(a, None);
                    let _ = self.default_var(&v.ty);
                    out.push(Arg::Value(v));
                }
            }
        }
        out
    }

    /// Borrows are written at the call site: `&x` for a `&T` parameter,
    /// `&mut x` for a `&mut T` one, nothing for one that takes ownership. A
    /// parameter that already is a borrow is passed on without a marker
    /// (an in-out one also where only a borrow is needed).
    fn check_marker(&mut self, marker: Option<bool>, mode: Option<ParamMode>, arg: &ast::Expr, span: Span) {
        use ln_diag::codes::borrow::{MISSING_MARKER, WRONG_MARKER};
        let lent = self.lent_param(arg);
        let shown = ast::pretty::expr(arg);
        let d = match (mode, marker) {
            (Some(ParamMode::InOut), Some(true)) | (Some(ParamMode::Borrow), Some(false)) => return,
            (Some(ParamMode::InOut), None) if lent == Some(ParamMode::InOut) => return,
            (Some(ParamMode::Borrow), None) if lent.is_some() => return,
            (Some(ParamMode::Take) | None, None) => return,
            (Some(ParamMode::InOut), None) => Diagnostic::error(MISSING_MARKER, span, "this argument is passed in-out, but the call does not say so")
                .with_label("the parameter is `&mut`")
                .with_help(format!("write `&mut {shown}`")),
            (Some(ParamMode::Borrow), None) => Diagnostic::error(MISSING_MARKER, span, "this argument is borrowed, but the call does not say so")
                .with_label("the parameter is `&`")
                .with_help(format!("write `&{shown}`")),
            (Some(ParamMode::InOut), Some(false)) => Diagnostic::error(WRONG_MARKER, span, "`&` lends read-only, but this parameter is in-out")
                .with_label("the parameter is `&mut`")
                .with_help(format!("write `&mut {shown}`")),
            (Some(ParamMode::Borrow), Some(true)) => Diagnostic::error(WRONG_MARKER, span, "`&mut` lends for changing, but this parameter only borrows")
                .with_label("the parameter is `&`")
                .with_help(format!("write `&{shown}`")),
            (Some(ParamMode::Take) | None, Some(m)) => Diagnostic::error(
                WRONG_MARKER,
                span,
                format!("this parameter takes ownership, so the argument is not lent with `{}`", if m { "&mut" } else { "&" }),
            )
            .with_label("the parameter takes ownership")
            .with_help(format!("pass `{shown}` itself (it is moved, or copied if it is Dull)")),
        };
        self.err(d);
    }

    /// The mode of the parameter `e` names, if `e` is just a borrowed or
    /// in-out parameter of the current function.
    fn lent_param(&self, e: &ast::Expr) -> Option<ParamMode> {
        let ast::ExprKind::Path(p) = &e.kind else { return None };
        if !p.is_single() {
            return None;
        }
        let l = self.lookup_local(&p.segments[0].ident.name, p.segments[0].ident.ctx)?;
        match self.local_kind(l) {
            thir::LocalKind::Param { inout: true, .. } => Some(ParamMode::InOut),
            thir::LocalKind::Param { owned: false, .. } => Some(ParamMode::Borrow),
            _ => None,
        }
    }

    /// An argument for an in-out (`&mut`) parameter: a mutable place.
    pub(crate) fn check_inout_arg(&mut self, a: &ast::Expr, pt: &Ty) -> Arg {
        let Some(place) = self.check_place(a) else {
            let v = self.check_expr(a, Some(pt));
            self.err(
                Diagnostic::error(tyc::NOT_A_PLACE, a.span, "this argument is passed in-out (`&mut` parameter) and must be a variable or field")
                    .with_help("bind the value with `let mut` first"),
            );
            let p = self.expr_as_place(v);
            return Arg::InOut(p);
        };
        if matches!(place.kind, ExprKind::Static(_))
            && matches!(&self.s.p.def(match place.kind { ExprKind::Static(d) => d, _ => unreachable!() }).data, DefData::Static(s) if s.mutable)
        {
            self.require_unsafe(a.span, "accessing a `static mut`");
        }
        if !self.is_mutable_place(&place) && !place.ty.is_error() {
            self.immutable_err_for(&place, a.span, "pass this as a `&mut` argument", matches!(pt, Ty::Bang(_)));
        }
        let want = self.shallow(pt.strip_bang());
        let have = self.shallow(place.ty.strip_bang());
        // Owner kinds subsume: a `[T; -..+]` may be passed where `[T; ..+]` or `[T]` is expected.
        if let (Ty::Array(we, wk), Ty::Array(he, hk)) = (&want, &have)
            && wk.is_owner()
            && hk.is_owner()
            && kind_rank(*hk) >= kind_rank(*wk)
        {
            let (we, he) = ((**we).clone(), (**he).clone());
            if !self.unify(&we, &he) {
                self.mismatch(&want, &have, a.span);
            }
            return Arg::InOut(place);
        }
        if !self.unify(&want, &have) {
            self.mismatch(&want, &have, a.span);
        }
        Arg::InOut(place)
    }

    // ===================================================================== method calls

    /// Receiver types to try, in order (autoslicing through array kinds).
    fn method_steps(&self, t: &Ty) -> Vec<Ty> {
        let mut out = vec![t.clone()];
        match t {
            Ty::Array(e, k) => {
                let chain: &[ArrKind] = match k {
                    ArrKind::Shrink => &[ArrKind::Grow, ArrKind::Dyn, ArrKind::SliceMut, ArrKind::Slice],
                    ArrKind::Grow => &[ArrKind::Dyn, ArrKind::SliceMut, ArrKind::Slice],
                    ArrKind::Dyn | ArrKind::Fixed(_) | ArrKind::FixedVar(_) => &[ArrKind::SliceMut, ArrKind::Slice],
                    ArrKind::SliceMut => &[ArrKind::Slice],
                    ArrKind::Slice => &[],
                };
                out.extend(chain.iter().map(|k| Ty::Array(e.clone(), *k)));
            }
            Ty::String => out.push(Ty::Str),
            _ => {}
        }
        out
    }

    fn pick_method(&mut self, step: &Ty, name: &str, span: Span) -> Option<MethodPick> {
        if let Some((def, args)) = self.inherent_method(step, name, true) {
            return Some(MethodPick { def, args, trait_call: None });
        }
        let (tr, mdef) = self.trait_method(step, name, true, span)?;
        let mut args = vec![step.clone()];
        args.extend(tr.args.iter().cloned());
        Some(MethodPick { def: mdef, args, trait_call: Some((tr, step.clone())) })
    }

    pub(crate) fn check_method_call(
        &mut self,
        recv: &ast::Expr,
        name: &ast::Ident,
        generics: Option<&[ast::Ty]>,
        args: &[ast::Expr],
        expect: Option<&Ty>,
        span: Span,
    ) -> Expr {
        // `[].capped(n)`: an empty array literal has no type of its own; take
        // the one the call is expected to produce (the method returns `Self`).
        let r = match (&recv.kind, expect) {
            (ast::ExprKind::Array(elems), Some(e)) if elems.is_empty() && matches!(self.shallow(e), Ty::Array(..)) => {
                let v = self.check_expr(recv, Some(e));
                self.expr_as_place(v)
            }
            _ => self.check_place_or_temp(recv),
        };
        if matches!(r.kind, ExprKind::Static(_)) {
            if let ExprKind::Static(d) = r.kind
                && matches!(&self.s.p.def(d).data, DefData::Static(s) if s.mutable)
            {
                self.require_unsafe(recv.span, "accessing a `static mut`");
            }
        }
        let rt = self.default_var(&r.ty);
        let rt = self.resolve(rt.strip_bang());
        if rt.is_error() {
            for a in args {
                let _ = self.check_expr(a, None);
            }
            return self.error_expr(span);
        }
        if self.is_unbound_var(&rt) {
            self.annotations_needed(recv.span);
            return self.error_expr(span);
        }
        let steps = self.method_steps(&rt);
        let mut picked = None;
        for (i, st) in steps.iter().enumerate() {
            if let Some(p) = self.pick_method(st, &name.name, name.span) {
                picked = Some((i, st.clone(), p));
                break;
            }
        }
        let Some((step_idx, step_ty, pick)) = picked else {
            let t = self.ty_str(&rt);
            let mut d = Diagnostic::error(tyc::NO_METHOD, name.span, format!("no method named `{}` found for `{t}`", name.name));
            if let Ty::Adt(id, _) = &rt
                && !self.s.p.is_enum(*id)
                && self.s.p.adt(*id).struct_fields().iter().any(|f| f.name == name.name)
            {
                d = d.with_note(format!("`{}` is a field, not a method", name.name));
            }
            if matches!(rt, Ty::Param(..)) {
                d = d.with_help("add a trait bound that provides this method");
            }
            self.err(d);
            for a in args {
                let _ = self.check_expr(a, None);
            }
            return self.error_expr(span);
        };
        self.record_def(name.span, pick.def);
        self.check_method_privacy(pick.def, name.span);
        let fd = self.s.p.fn_data(pick.def).clone();
        let mut all = pick.args.clone();
        let own = self.own_args(pick.def, generics, name.span);
        all.extend(own.iter().cloned());
        let params: Vec<(Ty, ParamMode)> = fd.sig.params.iter().map(|p| (p.ty.subst(&all), p.mode)).collect();
        let ret = fd.sig.ret.subst(&all);
        if fd.is_unsafe {
            let n = self.s.p.path_of(pick.def);
            self.require_unsafe(name.span, &format!("calling unsafe method `{n}`"));
        }
        // Receiver adjustment.
        let (self_ty, self_mode) = params[0].clone();
        let recv_arg = self.adjust_receiver(r, &rt, &step_ty, step_idx, &self_ty, self_mode, recv.span);
        let mut out = vec![recv_arg];
        out.extend(self.check_args(&params[1..], false, args, span));
        self.oblige_preds(pick.def, &all, name.span);
        let callee = match pick.trait_call {
            Some((tr, st)) => Callee::Trait { trait_ref: tr, self_ty: st, method: name.name.clone(), method_args: own },
            None if self.s.p.def(pick.def).attrs.intrinsic => Callee::Intrinsic { def: pick.def, name: self.s.p.def(pick.def).name.clone(), args: all },
            None => Callee::Fn { def: pick.def, args: all },
        };
        self.mk(ExprKind::Call { callee, args: out }, ret, span)
    }

    #[allow(clippy::too_many_arguments)]
    fn adjust_receiver(&mut self, r: Expr, rt: &Ty, step: &Ty, step_idx: usize, self_ty: &Ty, mode: ParamMode, span: Span) -> Arg {
        let want = self.shallow(self_ty.strip_bang());
        if mode == ParamMode::InOut {
            if !self.is_mutable_place(&r) && !r.ty.is_error() {
                self.immutable_err_for(&r, span, "call a method taking `&mut self` on this value", matches!(self_ty, Ty::Bang(_)));
            }
            if step_idx > 0 && matches!(step, Ty::Array(_, k) if k.is_slice()) {
                self.err(Diagnostic::error(tyc::MISMATCH, span, "cannot pass a slice view as `&mut self`"));
            }
            let _ = self.unify(&want, step);
            return Arg::InOut(r);
        }
        let _ = self.unify(&want, step);
        // A `self` receiver takes the value: it moves (or is copied, if Dull).
        let read_mode = if mode == ParamMode::Take { ReadMode::Move } else { ReadMode::Borrow };
        if step_idx == 0 {
            return Arg::Value(self.read(r, read_mode));
        }
        match (rt, step) {
            (Ty::Array(_, k), Ty::Array(_, sk)) if sk.is_slice() && !k.is_slice() => {
                let mutable = *sk == ArrKind::SliceMut;
                if mutable && !self.is_mutable_place(&r) {
                    self.immutable_err(&r, span, "borrow this array as `[T; &mut]`");
                }
                Arg::Value(self.mk(ExprKind::Coerce { kind: Coercion::ToSlice { mutable }, expr: Box::new(r) }, step.clone(), span))
            }
            (Ty::String, Ty::Str) => Arg::Value(self.mk(ExprKind::Coerce { kind: Coercion::StringToStr, expr: Box::new(r) }, Ty::Str, span)),
            _ => {
                // Owner-to-owner kinds and `&mut` -> `&` share a layout.
                let v = self.read(r, read_mode);
                Arg::Value(self.mk(ExprKind::Coerce { kind: Coercion::Weaken, expr: Box::new(v) }, step.clone(), span))
            }
        }
    }

    // ===================================================================== struct literals

    pub(crate) fn check_struct_lit(&mut self, path: &ast::Path, fields: &[ast::FieldInit], expect: Option<&Ty>, span: Span) -> Expr {
        let Some((adt, variant, args)) = self.resolve_struct_path(path, span) else {
            for f in fields {
                if let Some(v) = &f.value {
                    let _ = self.check_expr(v, None);
                }
            }
            return self.error_expr(span);
        };
        let ty = Ty::Adt(adt, args.clone());
        if let Some(e) = expect {
            let snapshot = (self.vars.clone(), self.lens.clone());
            if !self.unify(&ty, e) {
                (self.vars, self.lens) = snapshot;
            }
        }
        let v = self.s.p.adt(adt).variants[variant].clone();
        let owner_mod = self.s.p.def(adt).module;
        let mut out: Vec<(usize, Expr)> = Vec::new();
        for f in fields {
            let Some(idx) = v.fields.iter().position(|x| x.name == f.name.name) else {
                self.err(Diagnostic::error(tyc::NO_FIELD, f.name.span, format!("`{}` has no field named `{}`", v.name, f.name.name)));
                continue;
            };
            if out.iter().any(|(i, _)| *i == idx) {
                self.err(Diagnostic::error(tyc::DUPLICATE_FIELD, f.name.span, format!("field `{}` specified more than once", f.name.name)));
                continue;
            }
            let fd = &v.fields[idx];
            if !fd.is_pub && !self.s.p.is_enum(adt) && !self.s.is_ancestor_or_self(owner_mod, self.module()) {
                self.err(Diagnostic::error(name::PRIVATE, f.name.span, format!("field `{}` of `{}` is private", f.name.name, v.name)));
            }
            let ft = fd.ty.subst(&args);
            let x = match &f.value {
                Some(e) => self.check_expr(e, Some(&ft)),
                None => {
                    let p = ast::Path { segments: vec![ast::PathSegment { ident: f.name.clone(), args: None }], span: f.name.span, id: path.id };
                    self.check_path_value(&p, Some(&ft), f.name.span)
                }
            };
            out.push((idx, self.coerce(x, &ft, f.span)));
        }
        let missing: Vec<&str> = v.fields.iter().enumerate().filter(|(i, _)| !out.iter().any(|(j, _)| j == i)).map(|(_, f)| f.name.as_str()).collect();
        if !missing.is_empty() {
            self.err(Diagnostic::error(
                tyc::MISSING_FIELD,
                path.span,
                format!("missing field(s) {} in initializer of `{}`", missing.iter().map(|m| format!("`{m}`")).collect::<Vec<_>>().join(", "), v.name),
            ));
        }
        out.sort_by_key(|(i, _)| *i);
        self.mk(ExprKind::Adt { def: adt, args, variant, fields: out }, ty, span)
    }

    /// Resolve the path of a struct literal or struct pattern: (adt, variant, generic args).
    pub(crate) fn resolve_struct_path(&mut self, path: &ast::Path, span: Span) -> Option<(DefId, usize, Vec<Ty>)> {
        let r = self.resolve_struct_path_inner(path, span)?;
        let v = self.s.p.adt(r.0).variants[r.1].def;
        self.record_def(path.segments.last().unwrap().ident.span, v);
        Some(r)
    }

    fn resolve_struct_path_inner(&mut self, path: &ast::Path, span: Span) -> Option<(DefId, usize, Vec<Ty>)> {
        if path.is_single() && path.segments[0].ident.name == "Self" {
            return match self.cx.self_ty.clone() {
                Some(Ty::Adt(d, args)) if !self.s.p.is_enum(d) => Some((d, 0, args)),
                _ => {
                    self.err(Diagnostic::error(name::NOT_A_TYPE, span, "`Self` is not a struct here"));
                    None
                }
            };
        }
        if path.segments.len() == 2 && path.segments[0].ident.name == "Self" {
            if let Some(Ty::Adt(d, args)) = self.cx.self_ty.clone()
                && self.s.p.is_enum(d)
                && let Some(vi) = self.s.p.adt(d).variant_index(&path.segments[1].ident.name)
            {
                return Some((d, vi, args));
            }
        }
        let idents: Vec<ast::Ident> = path.segments.iter().map(|s| s.ident.clone()).collect();
        let module = self.module();
        let (res, used) = match self.s.resolve_prefix(module, &idents, Ns::Type) {
            Some(r) => r,
            None => {
                if idents.len() == 1 {
                    let d = self.s.unresolved_diag(module, &idents[0], Ns::Type);
                    self.err(d);
                }
                return None;
            }
        };
        let ModRes::Def(def) = res else {
            self.err(Diagnostic::error(name::NOT_A_TYPE, span, "expected a struct"));
            return None;
        };
        if used != idents.len() {
            self.err(Diagnostic::error(name::UNRESOLVED, span, format!("cannot find struct `{}`", path.to_string_lossy())));
            return None;
        }
        let d = self.s.p.def(def);
        match d.kind {
            DefKind::Struct => {
                let args = self.own_args(def, path.segments.last().unwrap().args.as_deref(), span);
                Some((def, 0, args))
            }
            DefKind::Variant => {
                let DefData::Variant { enum_id, index } = d.data else { unreachable!() };
                let explicit = if path.segments.len() >= 2 { path.segments[path.segments.len() - 2].args.as_deref() } else { None };
                let args = self.own_args(enum_id, explicit, span);
                Some((enum_id, index, args))
            }
            DefKind::TypeAlias => {
                let t = ast::Ty { kind: ast::TyKind::Path(path.clone()), span, id: path.id };
                match self.lower_local_ty(&t) {
                    Ty::Adt(d, args) if !self.s.p.is_enum(d) => Some((d, 0, args)),
                    _ => {
                        self.err(Diagnostic::error(name::NOT_A_TYPE, span, "this alias does not name a struct"));
                        None
                    }
                }
            }
            k => {
                let msg = format!("expected a struct or enum variant, found {} `{}`", k.describe(), d.name);
                self.err(Diagnostic::error(name::NOT_A_TYPE, span, msg));
                None
            }
        }
    }
}

/// Capability rank of owner kinds: a higher rank can stand in for a lower one.
fn kind_rank(k: ArrKind) -> u8 {
    match k {
        ArrKind::Dyn => 0,
        ArrKind::Grow => 1,
        ArrKind::Shrink => 2,
        _ => 0,
    }
}
