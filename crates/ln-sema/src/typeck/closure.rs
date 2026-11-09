//! Closures (docs/design/closures.md): checking a closure expression,
//! inferring its captures and kind, and calls through `Fn` traits.
//!
//! A closure is compiled to a function of its own whose leading parameters
//! are the captured variables, with the mode the body needs (read: `&`,
//! changed: `&mut`, consumed: owned), followed by the closure's parameters.
//! Borrow checking and code generation then treat it like any function;
//! only building the closure's value and calling it are special.

use super::FnCtx;
use crate::defs::*;
use crate::thir::{self, Arg, Callee, Expr, ExprKind, LocalId, LocalKind, ReadMode};
use crate::ty::Ty;
use crate::{AstRef, Capture, ClosureInfo, ClosureKind};
use ln_ast as ast;
use ln_diag::{Diagnostic, Span, codes::ty as tyc};

/// How a callable value is called: its parameters, result and kind.
pub(crate) type CallSig = (Vec<(ParamMode, Ty)>, Ty, ClosureKind);

/// A closure checked in the current function, finished with it.
pub(crate) struct PendingClosure {
    pub def: DefId,
    /// The locals made for the closure (its parameters and everything
    /// declared in its body): `first..end`.
    pub first: u32,
    pub end: u32,
    pub params: Vec<LocalId>,
    /// The captured variables, in capture order.
    pub captured: Vec<LocalId>,
    pub body: Expr,
}

/// How a closure body uses a variable from outside it.
#[derive(Clone, Copy, Default)]
struct Use {
    changed: bool,
    moved: bool,
}

/// The local a place expression is rooted at (not through a pointer).
fn root_local(e: &Expr) -> Option<LocalId> {
    match &e.kind {
        ExprKind::Local(l) => Some(*l),
        ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => root_local(base),
        _ => None,
    }
}

impl<'s, 'a> FnCtx<'s, 'a> {
    /// The signature a closure is expected to have: from a function pointer
    /// type, or from an `Fn(..)` bound on the inference variable or generic
    /// parameter it is checked against.
    fn expected_closure_sig(&self, t: &Ty) -> Option<(Vec<(ParamMode, Ty)>, Ty)> {
        let t = self.shallow(t);
        let from = |sig: &Ty| match self.shallow(sig) {
            Ty::FnPtr(ps, r) => Some((ps.clone(), *r.clone())),
            _ => None,
        };
        let is_fn = |id: DefId| self.s.p.lang_name(id).and_then(ClosureKind::from_lang).is_some();
        match &t {
            Ty::FnPtr(ps, r) => Some((ps.clone(), *r.clone())),
            // Where another closure is expected (both returned as one
            // `impl Fn`), take its signature; the types will still differ.
            Ty::Closure(def, args) if self.s.p.closures.contains_key(def) => from(&self.s.p.closure_sig(*def, args)),
            Ty::Infer(_) => {
                let bound = self
                    .obligations
                    .iter()
                    .find_map(|(ty, tr, _)| if is_fn(tr.trait_id) && self.shallow(ty) == t { tr.args.first().and_then(from) } else { None });
                // A closure this function returns as `impl Fn(..)`.
                bound.or_else(|| {
                    let (i, _, _) = self.opaque_vars.iter().find(|(_, v, _)| self.shallow(v) == t)?;
                    let o = self.s.p.opaques.get(&(self.def, *i))?;
                    o.bounds.iter().find(|b| is_fn(b.trait_id)).and_then(|b| b.args.first()).and_then(from)
                })
            }
            Ty::Opaque(..) | Ty::Param(..) => self.known_preds(&t).iter().find_map(|p| {
                let is_fn = self.s.p.lang_name(p.trait_ref.trait_id).and_then(ClosureKind::from_lang).is_some();
                if is_fn && p.ty == t { p.trait_ref.args.first().and_then(from) } else { None }
            }),
            _ => None,
        }
    }

    /// The generic parameters of the function being checked, as types: a
    /// closure has its enclosing function's generics.
    fn identity_args(&self) -> Vec<Ty> {
        self.cx.generic_names.iter().enumerate().map(|(i, n)| Ty::Param(i as u32, n.clone())).collect()
    }

    pub(crate) fn check_closure(&mut self, c: &ast::Closure, expect: Option<&Ty>, span: Span) -> Expr {
        let expected = expect.and_then(|t| self.expected_closure_sig(t)).filter(|(ps, _)| ps.len() == c.params.len());
        let module = self.s.p.def(self.def).module;
        let def = self.s.new_def("<closure>", DefKind::Fn, Some(module), span, AstRef::None);

        // The body is checked in a context of its own: `return` leaves the
        // closure, and loops outside it cannot be broken from inside.
        self.push_scope(None);
        let first = self.locals.len() as u32;
        let depth = self.depth();
        let mut params = Vec::new();
        let mut sig_params = Vec::new();
        for (i, p) in c.params.iter().enumerate() {
            let exp = expected.as_ref().map(|(ps, _)| ps[i].clone());
            let ty = match &p.ty {
                Some(t) => {
                    let t = self.lower_local_ty(t);
                    if let Some((_, et)) = &exp
                        && !self.unify(&t, et)
                    {
                        self.mismatch(et, &t, p.span);
                    }
                    t
                }
                None => exp.as_ref().map(|(_, t)| t.clone()).unwrap_or_else(|| self.new_var(p.span)),
            };
            let mode = p.mode.or(exp.map(|(m, _)| m)).unwrap_or(ParamMode::Take);
            let name = p.name.as_ref().map_or("_".to_string(), |n| n.name.clone());
            let l = self.new_local(&name, ty.clone(), p.is_mut || mode == ParamMode::InOut, LocalKind::Let, p.span, depth);
            if let Some(n) = &p.name {
                self.bind_name(&n.name, n.ctx, l);
                self.record_local(n.span, l);
            }
            params.push(l);
            sig_params.push((mode, ty));
        }
        let ret = match &c.ret {
            Some(t) => self.lower_local_ty(t),
            None => expected.as_ref().map(|(_, r)| r.clone()).unwrap_or_else(|| self.new_var(span)),
        };
        let saved_ret = std::mem::replace(&mut self.ret_ty, ret.clone());
        let saved_loops = std::mem::take(&mut self.loops);
        let body = self.check_expr(&c.body, Some(&ret));
        let body = self.coerce(body, &ret, c.body.span);
        self.ret_ty = saved_ret;
        self.loops = saved_loops;
        self.pop_scope();
        let end = self.locals.len() as u32;

        // Captures: every variable from outside the closure that its body
        // uses, and how.
        let mut uses: Vec<(LocalId, Use)> = Vec::new();
        self.collect_uses(&body, first, &mut uses);
        let any_moved = uses.iter().any(|(_, u)| u.moved);
        let any_changed = uses.iter().any(|(_, u)| u.changed);
        let kind = if any_moved {
            ClosureKind::FnOnce
        } else if any_changed {
            ClosureKind::FnMut
        } else {
            ClosureKind::Fn
        };
        let mut captures = Vec::new();
        let mut capture_exprs = Vec::new();
        let mut captured = Vec::new();
        for (l, u) in &uses {
            let local = &self.locals[l.0 as usize];
            let ty = local.ty.clone();
            let by_ref = !c.is_move && !u.moved;
            // A closure that is called once owns what it holds by value.
            let mode = if !by_ref && kind == ClosureKind::FnOnce {
                ParamMode::Take
            } else if u.changed {
                ParamMode::InOut
            } else {
                ParamMode::Borrow
            };
            captures.push(Capture { name: local.name.clone(), ty: ty.clone(), by_ref, mode });
            let place = self.mk(ExprKind::Local(*l), ty.clone(), span);
            capture_exprs.push(if by_ref && mode == ParamMode::InOut {
                Arg::InOut(place)
            } else if by_ref {
                Arg::Value(place)
            } else {
                Arg::Value(self.read(place, ReadMode::Move))
            });
            captured.push(*l);
        }
        self.register_closure(def, captures, kind, sig_params, ret.clone());
        self.closures.push(PendingClosure { def, first, end, params, captured, body });
        let ty = Ty::Closure(def, self.identity_args());
        self.mk(ExprKind::Closure { def, captures: capture_exprs }, ty, span)
    }

    /// Record a closure's function and captures in the program, with types
    /// as resolved so far (they are resolved again when the function is
    /// finished).
    fn register_closure(&mut self, def: DefId, captures: Vec<Capture>, kind: ClosureKind, params: Vec<(ParamMode, Ty)>, ret: Ty) {
        let mut all = Vec::new();
        for c in &captures {
            all.push(Param {
                name: c.name.clone(),
                ctx: ast::SyntaxCtx::ROOT,
                ty: c.ty.clone(),
                mode: c.mode,
                inout: c.mode == ParamMode::InOut,
                mutable: c.mode == ParamMode::InOut,
                is_self: false,
                span: Span::DUMMY,
            });
        }
        for (i, (m, t)) in params.into_iter().enumerate() {
            all.push(Param {
                name: format!("<arg{i}>"),
                ctx: ast::SyntaxCtx::ROOT,
                ty: t,
                mode: m,
                inout: m == ParamMode::InOut,
                mutable: m == ParamMode::InOut,
                is_self: false,
                span: Span::DUMMY,
            });
        }
        let parent_count = self.cx.generic_names.len() as u32;
        let fd = FnData {
            generics: Generics { parent_count, names: Vec::new(), preds: Vec::new(), span: Span::DUMMY },
            sig: FnSig { params: all, ret, ret_span: Span::DUMMY },
            is_unsafe: false,
            abi: Abi::Lunar,
            variadic: false,
            has_body: true,
            owner: None,
            has_self: false,
        };
        self.s.p.defs[def.0 as usize].data = DefData::Fn(Box::new(fd));
        self.s.p.closures.insert(def, ClosureInfo { parent: self.def, captures, kind });
    }

    /// Re-record every closure of this function with its final types.
    pub(crate) fn resolve_closures(&mut self) {
        let defs: Vec<DefId> = self.closures.iter().map(|c| c.def).collect();
        for def in defs {
            let mut fd = self.s.p.fn_data(def).clone();
            for p in &mut fd.sig.params {
                p.ty = super::default_ty(&self.resolve(&p.ty));
            }
            fd.sig.ret = super::default_ty(&self.resolve(&fd.sig.ret));
            self.s.p.defs[def.0 as usize].data = DefData::Fn(Box::new(fd));
            let mut info = self.s.p.closures[&def].clone();
            for c in &mut info.captures {
                c.ty = super::default_ty(&self.resolve(&c.ty));
            }
            self.s.p.closures.insert(def, info);
        }
    }

    /// Each closure's own body, as a function: the parent's locals (with
    /// final types), where the captures and the closure's parameters are
    /// the parameters, and everything outside the closure is unused. The
    /// closure's variables become unused in the parent's (`parent_locals`).
    /// Inner closures come first, so an outer one leaves theirs out too.
    pub(crate) fn closure_bodies(&mut self, parent_locals: &mut [thir::Local]) -> Vec<thir::Body> {
        let mut out = Vec::new();
        for c in std::mem::take(&mut self.closures) {
            let mut body = c.body;
            self.zonk_expr(&mut body);
            let info = self.s.p.closures[&c.def].clone();
            let fd = self.s.p.fn_data(c.def).clone();
            let mut locals = parent_locals.to_vec();
            for (i, l) in locals.iter_mut().enumerate() {
                if !(c.first..c.end).contains(&(i as u32)) {
                    l.kind = LocalKind::Temp;
                }
            }
            for l in &mut parent_locals[c.first as usize..c.end as usize] {
                l.kind = LocalKind::Temp;
            }
            let mut params = Vec::new();
            for (i, (l, cap)) in c.captured.iter().zip(&info.captures).enumerate() {
                let local = &mut locals[l.0 as usize];
                local.kind = LocalKind::Param { index: i, inout: cap.mode == ParamMode::InOut, bang: false, owned: cap.mode == ParamMode::Take };
                local.mutable = cap.mode == ParamMode::InOut || local.mutable;
                local.depth = 0;
                params.push(*l);
            }
            let n = c.captured.len();
            for (j, l) in c.params.iter().enumerate() {
                let p = &fd.sig.params[n + j];
                let local = &mut locals[l.0 as usize];
                local.kind = LocalKind::Param { index: n + j, inout: p.mode == ParamMode::InOut, bang: false, owned: p.mode == ParamMode::Take };
                local.depth = 0;
                params.push(*l);
            }
            let span = body.span;
            let block = thir::Block { stmts: Vec::new(), tail: Some(Box::new(body)), locals: Vec::new(), span };
            out.push(thir::Body { def: c.def, locals, params, block, span });
        }
        out
    }

    /// The variables from outside a closure (locals before `first`) that
    /// `e` uses, and how.
    fn collect_uses(&self, e: &Expr, first: u32, out: &mut Vec<(LocalId, Use)>) {
        let note = |l: LocalId, u: Use, out: &mut Vec<(LocalId, Use)>| {
            if l.0 >= first {
                return;
            }
            match out.iter_mut().find(|(x, _)| *x == l) {
                Some((_, prev)) => {
                    prev.changed |= u.changed;
                    prev.moved |= u.moved;
                }
                None => out.push((l, u)),
            }
        };
        let changed = Use { changed: true, moved: false };
        let moved = Use { changed: false, moved: true };
        // How `e` itself uses a variable, before looking inside it.
        match &e.kind {
            ExprKind::Local(l) => note(*l, Use::default(), out),
            ExprKind::Read { place, mode: ReadMode::Move } => {
                if let Some(l) = root_local(place) {
                    note(l, moved, out);
                }
            }
            ExprKind::Assign { place, .. } | ExprKind::AssignOp { place, .. } => {
                if let Some(l) = root_local(place) {
                    note(l, changed, out);
                }
            }
            ExprKind::Slice { base, mutable: true, .. } => {
                if let Some(l) = root_local(base) {
                    note(l, changed, out);
                }
            }
            ExprKind::Call { callee, args } => {
                for a in args {
                    if let Arg::InOut(p) = a
                        && let Some(l) = root_local(p)
                    {
                        note(l, changed, out);
                    }
                }
                if let Callee::Value { callee, kind: ClosureKind::FnMut } = callee
                    && let Some(l) = root_local(callee)
                {
                    note(l, changed, out);
                }
            }
            // A closure inside this one captures from it in turn; one that
            // changes a variable changes it here too.
            ExprKind::Closure { captures, .. } => {
                for a in captures {
                    if let Arg::InOut(p) = a
                        && let Some(l) = root_local(p)
                    {
                        note(l, changed, out);
                    }
                }
            }
            _ => {}
        }
        crate::borrowck::visit_children(e, &mut |x| self.collect_uses(x, first, out));
    }

    /// Why a closure does not satisfy an `Fn`, `FnMut` or `FnOnce` bound:
    /// what it does to its captures, or its signature.
    pub(crate) fn fn_bound_diag(&self, t: &Ty, tr: &crate::ty::TraitRef, span: Span) -> Option<Diagnostic> {
        let want = self.s.p.lang_name(tr.trait_id).and_then(ClosureKind::from_lang)?;
        let tn = self.s.p.def(tr.trait_id).name.clone();
        let Ty::Closure(def, args) = t else { return None };
        let info = self.s.p.closures.get(def)?;
        if info.kind > want {
            let (does, names): (&str, Vec<&str>) = if info.kind == ClosureKind::FnOnce {
                ("gives away", info.captures.iter().filter(|c| c.mode == ParamMode::Take).map(|c| c.name.as_str()).collect())
            } else {
                ("changes", info.captures.iter().filter(|c| c.mode == ParamMode::InOut).map(|c| c.name.as_str()).collect())
            };
            let list = names.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ");
            let kind = if info.kind == ClosureKind::FnOnce { "FnOnce" } else { "FnMut" };
            let why = if info.kind == ClosureKind::FnOnce { "so it can be called only once" } else { "so calling it changes something" };
            return Some(
                Diagnostic::error(tyc::UNSATISFIED_BOUND, span, format!("expected a closure that implements `{tn}`, but this one is only `{kind}`"))
                    .with_label(format!("the closure {does} {list}, {why}"))
                    .with_note(format!("a closure that {does} what it captured implements `{kind}`, not `{tn}`")),
            );
        }
        let have = self.s.p.closure_sig(*def, args);
        let want_sig = tr.args.first()?;
        let (h, w) = (self.s.p.ty_str(&have), self.s.p.ty_str(want_sig));
        Some(
            Diagnostic::error(tyc::UNSATISFIED_BOUND, span, format!("this closure has the wrong signature for `{tn}`"))
                .with_label(format!("expected `{w}`, found `{h}`")),
        )
    }

    // ===================================================================== calls

    /// The signature and kind for calling a value of type `t`, if it can
    /// be called: a closure, a function pointer, or a generic parameter
    /// bounded by an `Fn` trait.
    pub(crate) fn callable_sig(&self, t: &Ty) -> Option<CallSig> {
        let t = self.shallow(t);
        match &t {
            Ty::Closure(def, args) => {
                let kind = self.s.p.closures.get(def)?.kind;
                let Ty::FnPtr(ps, r) = self.s.p.closure_sig(*def, args) else { return None };
                Some((ps, *r, kind))
            }
            Ty::FnPtr(ps, r) => Some((ps.clone(), *r.clone(), ClosureKind::Fn)),
            Ty::Param(..) | Ty::Opaque(..) => {
                // The weakest bound that applies: `Fn` before `FnMut` before `FnOnce`.
                let mut best: Option<CallSig> = None;
                for p in self.known_preds(&t) {
                    let Some(kind) = self.s.p.lang_name(p.trait_ref.trait_id).and_then(ClosureKind::from_lang) else { continue };
                    if p.ty != t {
                        continue;
                    }
                    if let Some(Ty::FnPtr(ps, r)) = p.trait_ref.args.first()
                        && best.as_ref().is_none_or(|(_, _, k)| kind < *k)
                    {
                        best = Some((ps.clone(), *r.clone(), kind));
                    }
                }
                best
            }
            _ => None,
        }
    }

    /// `f(args)` where `f` is a value implementing an `Fn` trait.
    pub(crate) fn check_value_call(&mut self, f: &ast::Expr, args: &[ast::Expr], span: Span) -> Expr {
        let place = self.check_place_or_temp(f);
        if let Ty::FnPtr(params, ret) = self.shallow(&place.ty) {
            let callee = self.read(place, ReadMode::Copy);
            let sig: Vec<(Ty, ParamMode)> = params.iter().map(|(m, t)| (t.clone(), *m)).collect();
            let a = self.check_args(&sig, false, args, span);
            return self.mk(ExprKind::Call { callee: Callee::Indirect(Box::new(callee)), args: a }, *ret, span);
        }
        let Some((sig, ret, kind)) = self.callable_sig(&place.ty) else {
            if !self.resolve(&place.ty).is_error() {
                let t = self.ty_str(&place.ty);
                self.err(
                    Diagnostic::error(tyc::NOT_CALLABLE, f.span, format!("expected a function or closure, found `{t}`"))
                        .with_help("only functions, function pointers, closures and generic parameters bounded by `Fn`, `FnMut` or `FnOnce` can be called"),
                );
            }
            for a in args {
                let _ = self.check_expr(a, None);
            }
            return self.error_expr(span);
        };
        let callee = match kind {
            ClosureKind::Fn => place,
            ClosureKind::FnMut => {
                if !self.is_mutable_place(&place) && !place.ty.is_error() {
                    self.immutable_err_for(&place, f.span, "call this `FnMut` closure (it changes what it captured)", false);
                }
                place
            }
            ClosureKind::FnOnce => self.read(place, ReadMode::Move),
        };
        let sig: Vec<(Ty, ParamMode)> = sig.into_iter().map(|(m, t)| (t, m)).collect();
        let a = self.check_args(&sig, false, args, span);
        self.mk(ExprKind::Call { callee: Callee::Value { callee: Box::new(callee), kind }, args: a }, ret, span)
    }
}
