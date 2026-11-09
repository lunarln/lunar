//! Phase 3: type check function bodies and lower them to THIR.

mod call;
mod closure;
mod expr;
mod pat;

use crate::AstRef;
use crate::Sema;
use crate::defs::*;
use crate::lower::LowerCx;
use crate::thir::{self, Coercion, Expr, ExprKind, Local, LocalId, LocalKind, LoopId, ReadMode};
use crate::ty::{ArrKind, FloatTy, IntTy, TraitRef, Ty};
use ln_ast as ast;
use ln_diag::{Diagnostic, Span, codes::ty as tyc, codes::unsafe_};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VarKind {
    General,
    Int,
    Float,
}

#[derive(Clone, Debug)]
enum Var {
    Unbound(VarKind, Span),
    Bound(Ty),
}

#[derive(Clone, Debug)]
enum LenVar {
    Unbound,
    Bound(u64),
    Link(u32),
}

/// A lexical scope inside a function body.
struct Scope {
    names: Vec<(String, ast::SyntaxCtx, LocalId)>,
    /// Anonymous module for items declared in this block.
    module: Option<DefId>,
}

struct LoopCtx {
    id: LoopId,
    /// Type of `break value` (only for `loop`).
    break_ty: Option<Ty>,
    is_loop: bool,
    saw_break: bool,
}

pub(crate) struct FnCtx<'s, 'a> {
    s: &'s mut Sema<'a>,
    /// The function whose body is being checked.
    def: DefId,
    cx: LowerCx,
    env: Vec<Pred>,
    locals: Vec<Local>,
    scopes: Vec<Scope>,
    vars: Vec<Var>,
    lens: Vec<LenVar>,
    ret_ty: Ty,
    loops: Vec<LoopCtx>,
    next_loop: u32,
    unsafe_depth: u32,
    in_unsafe_fn: bool,
    obligations: Vec<(Ty, TraitRef, Span)>,
    /// Array element types to check for Dull once inference is done.
    dull_checks: Vec<(Ty, Span)>,
    /// Integer literals to range-check once their type is known.
    lit_checks: Vec<(Ty, i128, Span)>,
    reported_annotations: bool,
    /// Closures written in this function, finished with it.
    closures: Vec<closure::PendingClosure>,
    /// This function's `impl Trait` return types: the variable standing
    /// for each inside the body (the type it returns there).
    opaque_vars: Vec<(u32, Ty, Span)>,
}

impl<'a> Sema<'a> {
    pub(crate) fn check_bodies(&mut self) {
        let n = self.p.defs.len();
        for i in 0..n {
            let id = DefId(i as u32);
            if let AstRef::Fn(f, false) = self.ast(id)
                && let Some(body) = &f.body
            {
                if self.p.defs[i].attrs.test {
                    let fd = self.p.fn_data(id);
                    if !fd.sig.params.is_empty() || !fd.sig.ret.is_unit() || fd.generics.count() > 0 {
                        self.err(Diagnostic::error(tyc::BAD_TEST, f.sig_span, "test functions must take no parameters, have no generics and return `()`"));
                        continue;
                    }
                }
                let errs_before = self.diags.error_count();
                let (b, closures) = FnCtx::check_fn(self, id, f, body);
                let krate = self.mod_data(self.p.defs[i].module).krate;
                let is_std = self.p.crate_roots.iter().any(|(n, r)| *r == krate && (n == "core" || n == "std"));
                // Closures first: their bodies are part of this function's.
                for c in closures {
                    self.finish_body(c, errs_before, is_std);
                }
                self.finish_body(b, errs_before, is_std);
            }
        }
    }

    /// Borrow check and lint a checked body (if type checking its function
    /// found no errors), then keep it.
    fn finish_body(&mut self, b: thir::Body, errs_before: usize, is_std: bool) {
        if self.diags.error_count() == errs_before {
            crate::borrowck::check(&self.p, &b, &mut self.diags);
            // Lints on a function with borrow errors would only add
            // noise (and may be wrong about code that will change).
            if !is_std && self.diags.error_count() == errs_before {
                let mut lints = ln_diag::Diagnostics::new();
                crate::borrowck::lint(&self.p, &b, &mut lints);
                // Through `err`, which drops lints inside expansions.
                for d in lints.into_vec() {
                    self.err(d);
                }
            }
        }
        self.p.bodies.insert(b.def, b);
    }
}

impl<'s, 'a> FnCtx<'s, 'a> {
    /// The function's body, and the bodies of the closures written in it.
    fn check_fn(s: &'s mut Sema<'a>, def: DefId, f: &'a ast::FnDecl, body: &'a ast::Block) -> (thir::Body, Vec<thir::Body>) {
        let mut cx = s.lower_cx(def);
        cx.allow_infer = true;
        let fd = s.p.fn_data(def).clone();
        let env = s.p.elaborate(&cx.env);
        let mut fcx = FnCtx {
            s,
            def,
            cx,
            env,
            locals: Vec::new(),
            scopes: vec![Scope { names: Vec::new(), module: None }],
            vars: Vec::new(),
            lens: Vec::new(),
            ret_ty: fd.sig.ret.clone(),
            loops: Vec::new(),
            next_loop: 0,
            unsafe_depth: 0,
            in_unsafe_fn: fd.is_unsafe,
            obligations: Vec::new(),
            dull_checks: Vec::new(),
            lit_checks: Vec::new(),
            reported_annotations: false,
            closures: Vec::new(),
            opaque_vars: Vec::new(),
        };
        // Inside the body, each `impl Trait` it returns is the type it
        // actually returns: a variable to infer.
        let mut opened = Vec::new();
        fcx.ret_ty = fd.sig.ret.map(&mut |t| match t {
            Ty::Opaque(d, i, _) if d == def => {
                let span = fcx.s.p.opaques.get(&(d, i)).map_or(f.sig_span, |o| o.span);
                let v = fcx.new_var(span);
                opened.push((i, v.clone(), span));
                v
            }
            t => t,
        });
        fcx.opaque_vars = opened;
        let mut params = Vec::new();
        for (i, p) in fd.sig.params.iter().enumerate() {
            let bang = matches!(p.ty, Ty::Bang(_));
            let owned = p.mode == crate::defs::ParamMode::Take;
            let mutable = p.inout || (owned && p.mutable);
            let id = fcx.new_local(&p.name, p.ty.clone(), mutable, LocalKind::Param { index: i, inout: p.inout, bang, owned }, p.span, 0);
            if p.name != "_" {
                fcx.bind_name(&p.name, p.ctx, id);
            }
            params.push(id);
        }
        let ret = fcx.ret_ty.clone();
        let block = fcx.check_block_expr(body, Some(&ret), true);
        let diverges_or_ok = fcx.coercible(&block.ty, &ret);
        let block = if !diverges_or_ok && body.tail.is_none() && !ret.is_unit() {
            let r = fcx.ty_str(&ret);
            fcx.err(
                Diagnostic::error(
                    tyc::RETURN_MISSING,
                    f.sig_span,
                    format!("function `{}` must return `{r}` but its body can finish without returning", f.name.name),
                )
                .with_secondary(body.span.shrink_to_hi(), "body ends here")
                .with_help("add a `return` statement or a tail expression"),
            );
            block
        } else {
            let sp = body.tail.as_ref().map_or(body.span, |t| t.span);
            fcx.coerce(block, &ret, sp)
        };
        fcx.finish();
        let mut b = match block.kind {
            ExprKind::Block(b) => b,
            _ => thir::Block { stmts: Vec::new(), tail: Some(Box::new(block)), locals: Vec::new(), span: body.span },
        };
        fcx.zonk_block(&mut b);
        fcx.resolve_closures();
        let mut locals = std::mem::take(&mut fcx.locals);
        for l in &mut locals {
            l.ty = default_ty(&fcx.resolve(&l.ty));
        }
        let closures = fcx.closure_bodies(&mut locals);
        (thir::Body { def, locals, params, block: b, span: body.span }, closures)
    }

    // ===================================================================== diagnostics

    pub(crate) fn err(&mut self, d: Diagnostic) {
        self.s.err(d);
    }

    pub(crate) fn ty_str(&self, t: &Ty) -> String {
        let r = self.resolve(t);
        self.s.p.ty_str(&r)
    }

    pub(crate) fn mismatch(&mut self, expected: &Ty, found: &Ty, span: Span) {
        let (re, rf) = (self.resolve(expected), self.resolve(found));
        if re.has_error() || rf.has_error() {
            return;
        }
        let (e, f) = (self.s.p.ty_str(&re), self.s.p.ty_str(&rf));
        let mut d = Diagnostic::error(tyc::MISMATCH, span, format!("mismatched types: expected `{e}`, found `{f}`")).with_label(format!("expected `{e}`"));
        match (&re, &rf) {
            (Ty::Array(_, k), Ty::Array(_, k2)) if k.is_slice() && !k2.is_slice() => {
                d = d.with_help("take a slice of the array with `[..]`");
            }
            (Ty::Str, Ty::String) => d = d.with_help("use `.as_str()` to view a `String` as `str`"),
            (Ty::String, Ty::Str) => d = d.with_help("use `String::from(...)` to build an owned `String`"),
            (Ty::Array(_, a), Ty::Array(_, b)) if a.is_owner() && b.is_owner() => {
                d = d.with_note("array kinds only convert implicitly when passed to a parameter");
            }
            (Ty::Closure(..), Ty::Closure(..)) => {
                d = d.with_note("every closure has a type of its own, even when two are written alike");
                if self.opaque_vars.iter().any(|(_, v, _)| self.resolve(v) == re) {
                    d.code = tyc::OPAQUE_MISMATCH;
                    d = d.with_help("a function returning `impl Trait` returns one type: return a single closure and decide inside it");
                }
            }
            _ => {}
        }
        self.err(d);
    }

    // ===================================================================== locals & scopes

    pub(crate) fn depth(&self) -> u32 {
        self.scopes.len() as u32 - 1
    }

    pub(crate) fn new_local(&mut self, name: &str, ty: Ty, mutable: bool, kind: LocalKind, span: Span, depth: u32) -> LocalId {
        let id = LocalId(self.locals.len() as u32);
        self.locals.push(Local { name: name.to_string(), ty, mutable, kind, span, depth });
        if kind != LocalKind::Temp {
            self.record_local(span, id);
        }
        id
    }

    /// Remember that `span` names local `l` (editor tooling).
    pub(crate) fn record_local(&mut self, span: Span, l: LocalId) {
        self.s.p.refs.push(crate::Ref { span, target: crate::RefTarget::Local { body: self.def, local: l } });
    }

    pub(crate) fn record_def(&mut self, span: Span, d: DefId) {
        self.s.record(span, d);
    }

    /// Bind a local. Hygiene: a binding only answers to names with the same
    /// syntax context, so a macro's `let t` and the caller's `t` never meet.
    /// `self` is the exception: it is the receiver wherever it is written.
    pub(crate) fn bind_name(&mut self, name: &str, ctx: ast::SyntaxCtx, id: LocalId) {
        let ctx = if name == "self" { ast::SyntaxCtx::ROOT } else { ctx };
        self.scopes.last_mut().unwrap().names.push((name.to_string(), ctx, id));
    }

    pub(crate) fn local_kind(&self, id: LocalId) -> thir::LocalKind {
        self.locals[id.0 as usize].kind
    }

    pub(crate) fn lookup_local(&self, name: &str, ctx: ast::SyntaxCtx) -> Option<LocalId> {
        let ctx = if name == "self" { ast::SyntaxCtx::ROOT } else { ctx };
        for s in self.scopes.iter().rev() {
            if let Some((_, _, id)) = s.names.iter().rev().find(|(n, c, _)| n == name && *c == ctx) {
                return Some(*id);
            }
        }
        None
    }

    pub(crate) fn push_scope(&mut self, module: Option<DefId>) {
        self.scopes.push(Scope { names: Vec::new(), module });
    }

    pub(crate) fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    /// The module used for item lookups at this point (innermost block scope).
    pub(crate) fn module(&self) -> DefId {
        for s in self.scopes.iter().rev() {
            if let Some(m) = s.module {
                return m;
            }
        }
        self.cx.module
    }

    pub(crate) fn lower_cx(&self) -> LowerCx {
        let mut cx = self.cx.clone();
        cx.module = self.module();
        cx
    }

    pub(crate) fn new_loop(&mut self, is_loop: bool) -> LoopId {
        let id = LoopId(self.next_loop);
        self.next_loop += 1;
        self.loops.push(LoopCtx { id, break_ty: None, is_loop, saw_break: false });
        id
    }

    // ===================================================================== inference

    pub(crate) fn new_var(&mut self, span: Span) -> Ty {
        self.vars.push(Var::Unbound(VarKind::General, span));
        Ty::Infer(self.vars.len() as u32 - 1)
    }

    fn new_kind_var(&mut self, kind: VarKind, span: Span) -> Ty {
        self.vars.push(Var::Unbound(kind, span));
        Ty::Infer(self.vars.len() as u32 - 1)
    }

    pub(crate) fn int_var(&mut self, span: Span) -> Ty {
        self.new_kind_var(VarKind::Int, span)
    }

    pub(crate) fn float_var(&mut self, span: Span) -> Ty {
        self.new_kind_var(VarKind::Float, span)
    }

    pub(crate) fn new_len(&mut self) -> u32 {
        self.lens.push(LenVar::Unbound);
        self.lens.len() as u32 - 1
    }

    fn len_root(&self, mut v: u32) -> u32 {
        while let LenVar::Link(n) = self.lens[v as usize] {
            v = n;
        }
        v
    }

    /// Replace placeholder inference markers produced by type lowering.
    pub(crate) fn instantiate(&mut self, t: &Ty, span: Span) -> Ty {
        t.map(&mut |t| match t {
            Ty::Infer(u32::MAX) => self.new_var(span),
            Ty::Array(e, ArrKind::FixedVar(u32::MAX)) => {
                let v = self.new_len();
                Ty::Array(e, ArrKind::FixedVar(v))
            }
            t => t,
        })
    }

    /// Resolve the outermost inference variable.
    pub(crate) fn shallow(&self, t: &Ty) -> Ty {
        let mut t = t.clone();
        loop {
            match &t {
                Ty::Infer(v) => match &self.vars[*v as usize] {
                    Var::Bound(b) => t = b.clone(),
                    Var::Unbound(..) => return t,
                },
                Ty::Array(e, ArrKind::FixedVar(v)) => {
                    let r = self.len_root(*v);
                    return match self.lens[r as usize] {
                        LenVar::Bound(n) => Ty::Array(e.clone(), ArrKind::Fixed(n)),
                        _ => Ty::Array(e.clone(), ArrKind::FixedVar(r)),
                    };
                }
                _ => return t,
            }
        }
    }

    /// Fully resolve all inference variables that have been solved.
    pub(crate) fn resolve(&self, t: &Ty) -> Ty {
        t.map(&mut |t| match &t {
            Ty::Infer(_) | Ty::Array(_, ArrKind::FixedVar(_)) => {
                let s = self.shallow(&t);
                if s == t { t } else { self.resolve(&s) }
            }
            _ => t,
        })
    }

    fn var_kind(&self, t: &Ty) -> Option<VarKind> {
        match self.shallow(t) {
            Ty::Infer(v) => match &self.vars[v as usize] {
                Var::Unbound(k, _) => Some(*k),
                _ => None,
            },
            _ => None,
        }
    }

    pub(crate) fn is_int_var(&self, t: &Ty) -> bool {
        self.var_kind(t) == Some(VarKind::Int)
    }

    pub(crate) fn is_float_var(&self, t: &Ty) -> bool {
        self.var_kind(t) == Some(VarKind::Float)
    }

    pub(crate) fn is_unbound_var(&self, t: &Ty) -> bool {
        self.var_kind(t).is_some()
    }

    fn occurs(&self, v: u32, t: &Ty) -> bool {
        let r = self.resolve(t);
        let mut found = false;
        r.walk(&mut |x| found |= matches!(x, Ty::Infer(y) if *y == v));
        found
    }

    /// Unify two types; returns false on mismatch (no error reported).
    pub(crate) fn unify(&mut self, a: &Ty, b: &Ty) -> bool {
        let a = self.shallow(a);
        let b = self.shallow(b);
        match (&a, &b) {
            (Ty::Error, _) | (_, Ty::Error) => true,
            (Ty::Infer(x), Ty::Infer(y)) if x == y => true,
            (Ty::Infer(x), Ty::Infer(y)) => {
                let (kx, ky) = (self.var_kind(&a).unwrap(), self.var_kind(&b).unwrap());
                let merged = match (kx, ky) {
                    (VarKind::General, k) | (k, VarKind::General) => k,
                    (k1, k2) if k1 == k2 => k1,
                    _ => return false,
                };
                if let Var::Unbound(k, _) = &mut self.vars[*y as usize] {
                    *k = merged;
                }
                self.vars[*x as usize] = Var::Bound(b.clone());
                true
            }
            (Ty::Infer(x), t) | (t, Ty::Infer(x)) => {
                let Var::Unbound(kind, _) = self.vars[*x as usize] else { unreachable!() };
                let ok = match kind {
                    VarKind::General => true,
                    VarKind::Int => matches!(t, Ty::Int(_)),
                    VarKind::Float => matches!(t, Ty::Float(_)),
                };
                if !ok || self.occurs(*x, t) {
                    return false;
                }
                self.vars[*x as usize] = Var::Bound(t.clone());
                true
            }
            (Ty::Tuple(xs), Ty::Tuple(ys)) => xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| self.unify(x, y)),
            (Ty::Adt(d1, xs), Ty::Adt(d2, ys)) => d1 == d2 && xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| self.unify(x, y)),
            (Ty::Array(e1, k1), Ty::Array(e2, k2)) => self.unify_len(*k1, *k2) && self.unify(e1, e2),
            (Ty::Ptr(x, m1), Ty::Ptr(y, m2)) => m1 == m2 && self.unify(x, y),
            (Ty::Bang(x), Ty::Bang(y)) => self.unify(x, y),
            (Ty::FnPtr(p1, r1), Ty::FnPtr(p2, r2)) => {
                p1.len() == p2.len() && p1.iter().zip(p2).all(|((m1, x), (m2, y))| m1 == m2 && self.unify(x, y)) && self.unify(r1, r2)
            }
            (x, y) => x == y,
        }
    }

    fn unify_len(&mut self, a: ArrKind, b: ArrKind) -> bool {
        match (a, b) {
            (ArrKind::FixedVar(x), ArrKind::FixedVar(y)) => {
                let (rx, ry) = (self.len_root(x), self.len_root(y));
                if rx != ry {
                    match (self.lens[rx as usize].clone(), self.lens[ry as usize].clone()) {
                        (LenVar::Bound(n), LenVar::Bound(m)) => return n == m,
                        (LenVar::Bound(_), _) => self.lens[ry as usize] = LenVar::Link(rx),
                        _ => self.lens[rx as usize] = LenVar::Link(ry),
                    }
                }
                true
            }
            (ArrKind::FixedVar(x), ArrKind::Fixed(n)) | (ArrKind::Fixed(n), ArrKind::FixedVar(x)) => {
                let r = self.len_root(x);
                match self.lens[r as usize] {
                    LenVar::Bound(m) => m == n,
                    _ => {
                        self.lens[r as usize] = LenVar::Bound(n);
                        true
                    }
                }
            }
            (a, b) => a == b,
        }
    }

    /// Default integer/float variables (used before method lookups).
    pub(crate) fn default_var(&mut self, t: &Ty) -> Ty {
        let s = self.shallow(t);
        match self.var_kind(&s) {
            Some(VarKind::Int) => {
                self.unify(&s, &Ty::Int(IntTy::I32));
            }
            Some(VarKind::Float) => {
                self.unify(&s, &Ty::Float(FloatTy::F64));
            }
            _ => {}
        }
        self.shallow(t)
    }

    // ===================================================================== coercion

    /// Whether `from` can be coerced to `to` (without committing inference).
    pub(crate) fn coercible(&mut self, from: &Ty, to: &Ty) -> bool {
        let f = self.shallow(from);
        if matches!(f, Ty::Never | Ty::Error) {
            return true;
        }
        let snapshot = (self.vars.clone(), self.lens.clone());
        let ok = self.unify(from, to);
        (self.vars, self.lens) = snapshot;
        ok
    }

    /// Coerce `e` to type `to`, reporting a mismatch at `span` on failure.
    pub(crate) fn coerce(&mut self, e: Expr, to: &Ty, span: Span) -> Expr {
        match self.try_coerce(e, to) {
            Ok(e) => e,
            Err(e) => {
                self.mismatch(to, &e.ty, span);
                Expr { ty: Ty::Error, ..e }
            }
        }
    }

    pub(crate) fn try_coerce(&mut self, e: Expr, to: &Ty) -> Result<Expr, Expr> {
        let from = self.shallow(&e.ty);
        let target = self.shallow(to);
        let span = e.span;
        if matches!(from, Ty::Never) {
            if matches!(target, Ty::Never) {
                return Ok(e);
            }
            return Ok(Expr { kind: ExprKind::Coerce { kind: Coercion::NeverToAny, expr: Box::new(e) }, ty: to.clone(), span });
        }
        match (&from, &target) {
            (Ty::Array(a, ArrKind::SliceMut), Ty::Array(b, ArrKind::Slice)) => {
                let (a, b) = ((**a).clone(), (**b).clone());
                if self.unify(&a, &b) {
                    return Ok(Expr { kind: ExprKind::Coerce { kind: Coercion::Weaken, expr: Box::new(e) }, ty: to.clone(), span });
                }
            }
            (Ty::Ptr(a, true), Ty::Ptr(b, false)) => {
                let (a, b) = ((**a).clone(), (**b).clone());
                if self.unify(&a, &b) {
                    return Ok(Expr { kind: ExprKind::Coerce { kind: Coercion::Weaken, expr: Box::new(e) }, ty: to.clone(), span });
                }
            }
            _ => {}
        }
        if self.unify(&e.ty, to) { Ok(e) } else { Err(e) }
    }

    /// Coercion at a parameter-passing site: additionally allows viewing an
    /// array (place) as a slice and a `String` as `str`.
    pub(crate) fn coerce_arg(&mut self, e: Expr, to: &Ty, span: Span) -> Expr {
        let from = self.shallow(&e.ty);
        let target = self.shallow(to);
        match (&from, &target) {
            (Ty::Array(a, k), Ty::Array(b, tk)) if (k.is_owner() || k.is_fixed()) && tk.is_slice() => {
                let mutable = *tk == ArrKind::SliceMut;
                let (a, b) = ((**a).clone(), (**b).clone());
                if self.unify(&a, &b) {
                    let place = self.expr_as_place(e);
                    if mutable && !self.is_mutable_place(&place) {
                        self.immutable_err(&place, span, "borrow this array as `[T; &mut]`");
                    }
                    return Expr { kind: ExprKind::Coerce { kind: Coercion::ToSlice { mutable }, expr: Box::new(place) }, ty: to.clone(), span };
                }
            }
            (Ty::String, Ty::Str) => {
                let place = self.expr_as_place(e);
                return Expr { kind: ExprKind::Coerce { kind: Coercion::StringToStr, expr: Box::new(place) }, ty: Ty::Str, span };
            }
            _ => {}
        }
        self.coerce(e, to, span)
    }

    /// Turn a value-producing expression that reads a place back into the
    /// place (or materialize it into a temporary).
    pub(crate) fn expr_as_place(&mut self, e: Expr) -> Expr {
        match e.kind {
            ExprKind::Read { place, .. } => *place,
            _ if e.is_place() => e,
            _ => {
                let ty = e.ty.clone();
                let span = e.span;
                Expr { kind: ExprKind::Temp(Box::new(e)), ty, span }
            }
        }
    }

    /// Read a place as a value: copies for copyable types, otherwise `mode`.
    pub(crate) fn read(&mut self, place: Expr, mode: ReadMode) -> Expr {
        let ty = place.ty.clone();
        let span = place.span;
        let resolved = self.resolve(&ty);
        let copy = if resolved.has_infer() { !self.could_be_owner(&resolved) } else { self.s.p.is_copy(resolved.strip_bang(), &self.env) };
        let mode = if copy { ReadMode::Copy } else { mode };
        let ty = match ty {
            Ty::Bang(t) => *t,
            t => t,
        };
        Expr { kind: ExprKind::Read { place: Box::new(place), mode }, ty, span }
    }

    /// Conservative: an unresolved variable might become an owner.
    fn could_be_owner(&self, t: &Ty) -> bool {
        let mut any = false;
        t.walk(&mut |x| {
            any |= matches!(x, Ty::Infer(v) if matches!(self.vars[*v as usize], Var::Unbound(VarKind::General, _)))
                || matches!(x, Ty::String | Ty::Param(..))
                || matches!(x, Ty::Array(_, k) if k.is_owner())
        });
        any
    }

    // ===================================================================== places

    pub(crate) fn is_mutable_place(&self, e: &Expr) -> bool {
        match &e.kind {
            ExprKind::Local(id) => self.locals[id.0 as usize].mutable,
            ExprKind::Static(d) => matches!(&self.s.p.def(*d).data, DefData::Static(s) if s.mutable),
            ExprKind::Field { base, .. } => self.is_mutable_place(base),
            ExprKind::Index { base, .. } => match self.shallow(&base.ty) {
                Ty::Array(_, ArrKind::SliceMut) => true,
                Ty::Array(_, ArrKind::Slice) => false,
                _ => self.is_mutable_place(base),
            },
            ExprKind::Deref(p) => matches!(self.shallow(&p.ty), Ty::Ptr(_, true)),
            ExprKind::Temp(_) => true,
            _ => false,
        }
    }

    pub(crate) fn root_local(&self, e: &Expr) -> Option<LocalId> {
        match &e.kind {
            ExprKind::Local(id) => Some(*id),
            ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => self.root_local(base),
            _ => None,
        }
    }

    pub(crate) fn immutable_err(&mut self, place: &Expr, span: Span, what: &str) {
        self.immutable_err_for(place, span, what, false);
    }

    /// `shrink`: the place is about to be shrunk, so the in-place fix for a
    /// parameter is `&mut !T` rather than `&mut T`.
    pub(crate) fn immutable_err_for(&mut self, place: &Expr, span: Span, what: &str, shrink: bool) {
        let mut d = Diagnostic::error(tyc::IMMUTABLE, span, format!("cannot {what}: it is not mutable"));
        let container = match &place.kind {
            ExprKind::Index { base, .. } => self.shallow(&base.ty),
            _ => Ty::Error,
        };
        if let Ty::Array(_, ArrKind::Slice) = container {
            d = d.with_note("elements of a `[T; &]` slice are read-only; use a `[T; &mut]` slice");
        } else if let Some(l) = self.root_local(place) {
            let local = &self.locals[l.0 as usize];
            let n = local.name.clone();
            let sp = local.span;
            d = match local.kind {
                LocalKind::Param { owned: true, .. } => {
                    d.with_help(format!("declare it `mut {n}` to change this function's own copy, or `{n}: &mut ...` to change the caller's"))
                }
                LocalKind::Param { .. } => {
                    let ty = self.ty_str(local.ty.strip_bang());
                    let bang = if shrink { "!" } else { "" };
                    d.with_help(format!("to change the caller's value in place: `{n}: &mut {bang}{ty}`"))
                        .with_help(format!("or take ownership and change your own: `mut {n}: {ty}`"))
                }
                _ => d.with_help(format!("declare it with `let mut {n}`")),
            };
            d = d.with_secondary(sp, format!("`{n}` declared here"));
        } else if let ExprKind::Deref(_) = place.kind {
            d = d.with_note("writing through a pointer requires a `*mut T`");
        }
        self.err(d);
    }

    // ===================================================================== unsafe

    pub(crate) fn require_unsafe(&mut self, span: Span, what: &str) {
        if self.unsafe_depth > 0 {
            return;
        }
        if self.in_unsafe_fn {
            self.err(
                Diagnostic::error(unsafe_::IN_UNSAFE_FN, span, format!("{what} requires an `unsafe` block, even inside an `unsafe fn`"))
                    .with_note("`unsafe fn` only means the function is unsafe to call; its body gets no implicit unsafe context")
                    .with_help("wrap the operation in `unsafe { ... }`"),
            );
        } else {
            self.err(
                Diagnostic::error(unsafe_::OUTSIDE_UNSAFE, span, format!("{what} is unsafe and requires an `unsafe` block"))
                    .with_help("wrap the operation in `unsafe { ... }`"),
            );
        }
    }

    // ===================================================================== obligations

    /// The bounds known to hold here: the function's (with supertraits),
    /// and for an `impl Trait` type, what it promises.
    pub(crate) fn known_preds(&self, ty: &Ty) -> Vec<Pred> {
        let mut out = self.env.clone();
        if let Ty::Opaque(d, i, args) = ty {
            out.extend(self.s.p.elaborate(&self.s.p.opaque_preds(*d, *i, args)));
        }
        out
    }

    pub(crate) fn oblige(&mut self, ty: Ty, tr: TraitRef, span: Span) {
        self.obligations.push((ty, tr, span));
    }

    /// Learn types from trait bounds: where a bound with unknown types can
    /// only be met one way (by one impl, or one bound in scope), take that
    /// way's types. `I: Iterator::<(K, V)>` with `I` known gives `K` and
    /// `V`; a returned `Ids::<_>` promised as `impl Iterator::<Id::<T>>`
    /// gives its parameter.
    pub(crate) fn infer_from_bounds(&mut self) {
        let mut goals: Vec<(Ty, TraitRef)> = self.obligations.iter().map(|(t, tr, _)| (t.clone(), tr.clone())).collect();
        for (i, v, _) in &self.opaque_vars {
            if let Some(o) = self.s.p.opaques.get(&(self.def, *i)) {
                goals.extend(o.bounds.iter().map(|b| (v.clone(), b.clone())));
            }
        }
        for _ in 0..4 {
            let mut progress = false;
            for (ty, tr) in &goals {
                let t = self.resolve(ty);
                let tr = TraitRef { trait_id: tr.trait_id, args: tr.args.iter().map(|a| self.resolve(a)).collect() };
                if !t.has_infer() && !tr.args.iter().any(Ty::has_infer) {
                    continue;
                }
                progress |= self.meet_unique_way(&t, &tr);
            }
            if !progress {
                break;
            }
        }
    }

    /// If exactly one bound in scope or impl can make `t: tr` hold, unify
    /// with it; returns whether that taught anything.
    fn meet_unique_way(&mut self, t: &Ty, tr: &TraitRef) -> bool {
        // Each way: the self type and trait arguments it has.
        let mut ways: Vec<(Ty, Vec<Ty>)> = Vec::new();
        let sm = self.shallow(t);
        if !matches!(sm, Ty::Infer(_)) {
            for p in self.known_preds(&sm) {
                if p.trait_ref.trait_id == tr.trait_id {
                    ways.push((p.ty.clone(), p.trait_ref.args.clone()));
                }
            }
        }
        let snapshot = (self.vars.clone(), self.lens.clone());
        let mut fits = Vec::new();
        for (wty, wargs) in ways {
            let ok = self.unify(&wty, t) && wargs.len() == tr.args.len() && wargs.iter().zip(&tr.args).all(|(a, b)| self.unify(a, b));
            (self.vars, self.lens) = snapshot.clone();
            if ok {
                fits.push((wty, wargs));
            }
        }
        let mut impl_fit: Option<DefId> = None;
        let mut impl_count = 0;
        for imp in self.s.p.impls.clone() {
            let DefData::Impl(d) = &self.s.p.def(imp).data else { continue };
            let Some(itr) = &d.trait_ref else { continue };
            if itr.trait_id != tr.trait_id || d.negative {
                continue;
            }
            let (self_ty, itr, n) = (d.self_ty.clone(), itr.clone(), d.generics.count());
            let fresh: Vec<Ty> = (0..n).map(|_| self.new_var(Span::DUMMY)).collect();
            let ok = self.unify(&self_ty.subst(&fresh), t)
                && itr.args.len() == tr.args.len()
                && itr.args.iter().zip(&tr.args).all(|(a, b)| self.unify(&a.subst(&fresh), b));
            (self.vars, self.lens) = snapshot.clone();
            if ok {
                impl_count += 1;
                impl_fit = Some(imp);
            }
        }
        if fits.len() + impl_count != 1 {
            return false;
        }
        let before = (self.resolve(t), tr.args.iter().map(|a| self.resolve(a)).collect::<Vec<_>>());
        if let Some((wty, wargs)) = fits.pop() {
            self.unify(&wty, t);
            for (a, b) in wargs.iter().zip(&tr.args) {
                self.unify(a, b);
            }
        } else if let Some(imp) = impl_fit {
            let d = self.s.p.impl_data(imp).clone();
            let itr = d.trait_ref.clone().unwrap();
            let fresh: Vec<Ty> = (0..d.generics.count()).map(|_| self.new_var(Span::DUMMY)).collect();
            self.unify(&d.self_ty.subst(&fresh), t);
            for (a, b) in itr.args.iter().zip(&tr.args) {
                self.unify(&a.subst(&fresh), b);
            }
        }
        let after = (self.resolve(t), tr.args.iter().map(|a| self.resolve(a)).collect::<Vec<_>>());
        before != after
    }

    /// Resolve remaining inference, check deferred obligations and literals.
    fn finish(&mut self) {
        self.infer_from_bounds();
        for i in 0..self.vars.len() {
            if let Var::Unbound(k, _) = self.vars[i] {
                match k {
                    VarKind::Int => self.vars[i] = Var::Bound(Ty::Int(IntTy::I32)),
                    VarKind::Float => self.vars[i] = Var::Bound(Ty::Float(FloatTy::F64)),
                    VarKind::General => {}
                }
            }
        }
        // Whether a closure implements an `Fn` trait depends on its
        // signature, so record it before checking obligations.
        self.resolve_closures();
        for (ty, tr, span) in std::mem::take(&mut self.obligations) {
            let t = self.resolve(&ty);
            let tr = TraitRef { trait_id: tr.trait_id, args: tr.args.iter().map(|a| self.resolve(a)).collect() };
            if t.has_infer() || tr.args.iter().any(Ty::has_infer) {
                self.annotations_needed(span);
                continue;
            }
            if !self.s.p.implements(&t, &tr, &self.env) {
                if let Some(d) = self.fn_bound_diag(&t, &tr, span) {
                    self.err(d);
                    continue;
                }
                let ts = self.s.p.ty_str(&t);
                let tn = self.s.p.def(tr.trait_id).name.clone();
                let mut d = Diagnostic::error(tyc::UNSATISFIED_BOUND, span, format!("the trait bound `{ts}: {tn}` is not satisfied"))
                    .with_label(format!("`{ts}` does not implement `{tn}`"));
                if let Ty::Param(_, n) = &t {
                    d = d.with_help(format!("add {tn} to {n}'s bounds: `{n}: {tn}`"));
                } else if self.s.p.is_lang(tr.trait_id, "dull") {
                    d = d.with_note(format!("`{ts}` is pointy: it is or contains a pointer, slice, string or array"));
                }
                self.err(d);
            }
        }
        self.finish_opaques();
        for (ty, span) in std::mem::take(&mut self.dull_checks) {
            let t = self.resolve(&ty);
            if t.has_infer() || t.has_error() {
                continue;
            }
            if !self.s.p.is_dull(&t, &self.env) {
                let d = self.s.pointy_diag(&t, span, self.cx.generics_span);
                self.err(d);
            }
        }
        for (ty, v, span) in std::mem::take(&mut self.lit_checks) {
            if let Ty::Int(it) = self.resolve(&ty) {
                let ok = if v < 0 { v.unsigned_abs() <= it.min_magnitude() } else { (v as u128) <= it.max() };
                if !ok {
                    let lo = if it.signed() { format!("-{}", it.min_magnitude()) } else { "0".into() };
                    self.err(Diagnostic::error(tyc::LITERAL_RANGE, span, format!("literal `{v}` does not fit in `{}`", it.name())).with_note(format!(
                        "`{}` ranges from {lo} to {}",
                        it.name(),
                        it.max()
                    )));
                }
            }
        }
        for i in 0..self.vars.len() {
            if let Var::Unbound(VarKind::General, span) = self.vars[i] {
                self.annotations_needed(span);
                self.vars[i] = Var::Bound(Ty::Error);
            }
        }
        for i in 0..self.lens.len() {
            if matches!(self.lens[i], LenVar::Unbound) {
                self.lens[i] = LenVar::Bound(0);
            }
        }
    }

    /// Record the type each `impl Trait` of this function stands for, and
    /// check that it has the promised bounds.
    fn finish_opaques(&mut self) {
        for (i, var, span) in std::mem::take(&mut self.opaque_vars) {
            let c = self.resolve(&var);
            if c.has_infer() {
                // Only a body that never returns leaves it open.
                if matches!(self.shallow(&var), Ty::Infer(_)) {
                    self.unify(&var, &Ty::unit());
                } else {
                    self.annotations_needed(span);
                    continue;
                }
            }
            let c = default_ty(&self.resolve(&var));
            let bounds = self.s.p.opaques.get(&(self.def, i)).map(|o| o.bounds.clone()).unwrap_or_default();
            for b in &bounds {
                if c.has_error() || self.s.p.implements(&c, b, &self.env) {
                    continue;
                }
                let (cs, bs) = (self.s.p.ty_str(&c), self.s.p.trait_ref_str(b));
                // For a closure, say why at the closure.
                let at = match &c {
                    Ty::Closure(cd, _) => self.s.p.def(*cd).span,
                    _ => span,
                };
                let mut d = match self.fn_bound_diag(&c, b, at) {
                    Some(fd) => fd.with_secondary(span, format!("the function promises `{bs}`")),
                    None => Diagnostic::error(tyc::OPAQUE_MISMATCH, span, format!("this function returns `{cs}`, which does not implement `{bs}`"))
                        .with_label(format!("promises `{bs}`")),
                };
                d.code = tyc::OPAQUE_MISMATCH;
                self.err(d);
            }
            if let Some(o) = self.s.p.opaques.get_mut(&(self.def, i)) {
                o.concrete = Some(c);
            }
        }
    }

    pub(crate) fn annotations_needed(&mut self, span: Span) {
        if self.reported_annotations {
            return;
        }
        self.reported_annotations = true;
        self.err(Diagnostic::error(tyc::ANNOTATIONS_NEEDED, span, "type annotations needed").with_help("add a type annotation, e.g. `let x: i32 = ...`"));
    }

    // ===================================================================== zonking

    fn zonk(&self, t: &mut Ty) {
        *t = default_ty(&self.resolve(t));
    }

    fn zonk_block(&self, b: &mut thir::Block) {
        for s in &mut b.stmts {
            match s {
                thir::Stmt::Let { pat, init, .. } => {
                    self.zonk_pat(pat);
                    if let Some(e) = init {
                        self.zonk_expr(e);
                    }
                }
                thir::Stmt::Expr(e) => self.zonk_expr(e),
            }
        }
        if let Some(t) = &mut b.tail {
            self.zonk_expr(t);
        }
    }

    fn zonk_pat(&self, p: &mut thir::Pat) {
        self.zonk(&mut p.ty);
        match &mut p.kind {
            thir::PatKind::Binding { sub: Some(s), .. } => self.zonk_pat(s),
            thir::PatKind::Tuple(ps) | thir::PatKind::Or(ps) => ps.iter_mut().for_each(|p| self.zonk_pat(p)),
            thir::PatKind::Adt { fields, .. } => fields.iter_mut().for_each(|(_, p)| self.zonk_pat(p)),
            _ => {}
        }
    }

    fn zonk_expr(&self, e: &mut Expr) {
        self.zonk(&mut e.ty);
        use ExprKind as K;
        match &mut e.kind {
            K::Local(_) | K::Static(_) | K::Lit(_) | K::Continue { .. } | K::Error => {}
            K::Field { base, .. } => self.zonk_expr(base),
            K::Index { base, index } => {
                self.zonk_expr(base);
                self.zonk_expr(index);
            }
            K::Deref(x) | K::Temp(x) | K::Unary(_, x) | K::Cast(x) => self.zonk_expr(x),
            K::Read { place, .. } => self.zonk_expr(place),
            K::FnRef { args, .. } => args.iter_mut().for_each(|t| self.zonk(t)),
            K::Call { callee, args } => {
                match callee {
                    thir::Callee::Fn { args, .. } | thir::Callee::Intrinsic { args, .. } => args.iter_mut().for_each(|t| self.zonk(t)),
                    thir::Callee::Trait { trait_ref, self_ty, method_args, .. } => {
                        trait_ref.args.iter_mut().for_each(|t| self.zonk(t));
                        self.zonk(self_ty);
                        method_args.iter_mut().for_each(|t| self.zonk(t));
                    }
                    thir::Callee::Indirect(f) | thir::Callee::Value { callee: f, .. } => self.zonk_expr(f),
                }
                for a in args {
                    match a {
                        thir::Arg::Value(x) | thir::Arg::InOut(x) => self.zonk_expr(x),
                    }
                }
            }
            K::Binary(_, a, b) | K::Logical { lhs: a, rhs: b, .. } => {
                self.zonk_expr(a);
                self.zonk_expr(b);
            }
            K::Assign { place, value } | K::AssignOp { place, value, .. } => {
                self.zonk_expr(place);
                self.zonk_expr(value);
            }
            K::Tuple(es) | K::Array(es) => es.iter_mut().for_each(|x| self.zonk_expr(x)),
            K::Adt { args, fields, .. } => {
                args.iter_mut().for_each(|t| self.zonk(t));
                fields.iter_mut().for_each(|(_, x)| self.zonk_expr(x));
            }
            K::ArrayRepeat { value, count } => {
                self.zonk_expr(value);
                self.zonk_expr(count);
            }
            K::Slice { base, start, end, .. } => {
                self.zonk_expr(base);
                if let Some(x) = start {
                    self.zonk_expr(x);
                }
                if let Some(x) = end {
                    self.zonk_expr(x);
                }
            }
            K::Block(b) | K::Loop { body: b, .. } => self.zonk_block(b),
            K::If { cond, then, else_ } => {
                self.zonk_expr(cond);
                self.zonk_expr(then);
                if let Some(x) = else_ {
                    self.zonk_expr(x);
                }
            }
            K::Match { scrut, arms } => {
                self.zonk_expr(scrut);
                for a in arms {
                    self.zonk_pat(&mut a.pat);
                    if let Some(g) = &mut a.guard {
                        self.zonk_expr(g);
                    }
                    self.zonk_expr(&mut a.body);
                }
            }
            K::While { cond, body, .. } => {
                self.zonk_expr(cond);
                self.zonk_block(body);
            }
            K::ForRange { start, end, body, .. } => {
                self.zonk_expr(start);
                self.zonk_expr(end);
                self.zonk_block(body);
            }
            K::ForArray { pat, array, body, .. } => {
                self.zonk_pat(pat);
                self.zonk_expr(array);
                self.zonk_block(body);
            }
            K::Break { value, .. } | K::Return(value) => {
                if let Some(x) = value {
                    self.zonk_expr(x);
                }
            }
            K::Coerce { expr, .. } => self.zonk_expr(expr),
            K::FmtArgs(pieces) => {
                for p in pieces {
                    if let thir::FmtPiece::Arg(x, _) = p {
                        self.zonk_expr(x);
                    }
                }
            }
            K::Closure { captures, .. } => {
                for a in captures {
                    match a {
                        thir::Arg::Value(x) | thir::Arg::InOut(x) => self.zonk_expr(x),
                    }
                }
            }
        }
    }
}

/// Replace any leftover inference variables with defaults (after errors).
fn default_ty(t: &Ty) -> Ty {
    t.map(&mut |t| match t {
        Ty::Infer(_) => Ty::Error,
        Ty::Array(e, ArrKind::FixedVar(_)) => Ty::Array(e, ArrKind::Fixed(0)),
        t => t,
    })
}

pub(crate) fn lang_trait_ref(p: &crate::Program, name: &str) -> Option<TraitRef> {
    p.lang_item(name).map(|id| TraitRef { trait_id: id, args: Vec::new() })
}
