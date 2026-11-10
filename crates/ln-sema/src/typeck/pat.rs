//! Patterns and match exhaustiveness.

use super::FnCtx;
use super::call::ValueRes;
use crate::defs::*;
use crate::thir::{self, Arm, LocalId, LocalKind, Pat, PatKind, ReadMode};
use crate::ty::Ty;
use ln_ast as ast;
use ln_diag::{Diagnostic, Span, codes::ty as tyc};

impl<'s, 'a> FnCtx<'s, 'a> {
    /// Struct fields are private to the struct's module (and its children)
    /// unless `pub`; enum variant fields are always visible.
    pub(crate) fn check_field_privacy(&mut self, adt: DefId, field: usize, span: Span) {
        let data = self.s.p.adt(adt);
        if data.is_enum {
            return;
        }
        let f = &data.variants[0].fields[field];
        let owner_mod = self.s.p.def(adt).module;
        if f.is_pub || self.s.is_ancestor_or_self(owner_mod, self.module()) {
            return;
        }
        let (fname, fspan, sname) = (f.name.clone(), f.span, self.s.p.def(adt).name.clone());
        self.err(
            Diagnostic::error(ln_diag::codes::name::PRIVATE, span, format!("field `{fname}` of `{sname}` is private"))
                .with_secondary(fspan, "declared here")
                .with_help("mark the field `pub`, or match the rest with `..`"),
        );
    }

    /// Check a pattern against `ty`, declaring its bindings in the current scope.
    pub(crate) fn check_pat(&mut self, p: &ast::Pat, ty: &Ty, _is_let: bool) -> Pat {
        let span = p.span;
        let sty = self.shallow(ty);
        let sty = sty.strip_grow().clone();
        let kind = match &p.kind {
            ast::PatKind::MacCall(m) => {
                let module = self.module();
                return match self.s.expand_pat(module, m) {
                    Some(x) => self.check_pat(x, ty, _is_let),
                    None => Pat { kind: PatKind::Wild, ty: ty.clone(), span },
                };
            }
            ast::PatKind::Wild | ast::PatKind::Rest => PatKind::Wild,
            ast::PatKind::Binding { is_mut, name } => self.binding(name, *is_mut, ty, span),
            ast::PatKind::Lit(l, neg) => self.lit_pat(l, *neg, ty, span),
            ast::PatKind::Range { lo, hi, inclusive } => {
                let lo_v = self.lit_pat(&lo.0, lo.1, ty, span);
                let hi_v = self.lit_pat(&hi.0, hi.1, ty, span);
                match (lo_v, hi_v) {
                    (PatKind::Lit(a), PatKind::Lit(b)) => {
                        if !matches!(self.shallow(ty), Ty::Int(_) | Ty::Char | Ty::Error) && !self.is_int_var(ty) {
                            self.err(Diagnostic::error(tyc::BAD_PATTERN, span, "range patterns only work on integers and `char`"));
                        }
                        PatKind::Range { lo: a, hi: b, inclusive: *inclusive }
                    }
                    _ => PatKind::Wild,
                }
            }
            ast::PatKind::Tuple(ps) => {
                let has_rest = ps.iter().position(|x| matches!(x.kind, ast::PatKind::Rest));
                let elems: Vec<Ty> = match &sty {
                    Ty::Tuple(ts) => ts.clone(),
                    t if self.is_unbound_var(t) && has_rest.is_none() => {
                        let ts: Vec<Ty> = ps.iter().map(|x| self.new_var(x.span)).collect();
                        let _ = self.unify(t, &Ty::Tuple(ts.clone()));
                        ts
                    }
                    Ty::Error => vec![Ty::Error; ps.len()],
                    t => {
                        self.err(Diagnostic::error(tyc::BAD_PATTERN, span, format!("expected `{}`, found a tuple pattern", self.ty_str(t))));
                        return Pat { kind: PatKind::Wild, ty: Ty::Error, span };
                    }
                };
                let out = self.tuple_like(ps, &elems, has_rest, span);
                PatKind::Tuple(out.into_iter().map(|(_, p)| p).collect())
            }
            ast::PatKind::TupleStruct { path, pats } => match self.resolve_value_path(path, path.span) {
                Some(ValueRes::Ctor { adt, variant, args }) => {
                    let adt_ty = Ty::Adt(adt, args.clone());
                    if !self.unify(&adt_ty, ty) {
                        self.mismatch(ty, &adt_ty, span);
                    }
                    let v = self.s.p.adt(adt).variants[variant].clone();
                    if v.ctor != CtorKind::Tuple {
                        self.err(Diagnostic::error(tyc::BAD_PATTERN, span, format!("`{}` is not a tuple variant or struct", v.name)));
                        return Pat { kind: PatKind::Wild, ty: ty.clone(), span };
                    }
                    let ftys: Vec<Ty> = v.fields.iter().map(|f| f.ty.subst(&args)).collect();
                    let has_rest = pats.iter().position(|x| matches!(x.kind, ast::PatKind::Rest));
                    // Positions written out (not covered by `..`) must be visible.
                    let after = has_rest.map_or(0, |r| pats.len() - r - 1);
                    let before = has_rest.unwrap_or(pats.len());
                    for i in (0..v.fields.len()).filter(|i| *i < before || *i + after >= v.fields.len()) {
                        self.check_field_privacy(adt, i, span);
                    }
                    let fields = self.tuple_like(pats, &ftys, has_rest, span);
                    PatKind::Adt { def: adt, variant, fields }
                }
                Some(_) => {
                    self.err(Diagnostic::error(tyc::BAD_PATTERN, path.span, format!("`{}` is not a tuple variant or struct", path.to_string_lossy())));
                    PatKind::Wild
                }
                None => PatKind::Wild,
            },
            ast::PatKind::Struct { path, fields, rest } => {
                let Some((adt, variant, args)) = self.resolve_struct_path(path, span) else {
                    return Pat { kind: PatKind::Wild, ty: ty.clone(), span };
                };
                let adt_ty = Ty::Adt(adt, args.clone());
                if !self.unify(&adt_ty, ty) {
                    self.mismatch(ty, &adt_ty, span);
                }
                let v = self.s.p.adt(adt).variants[variant].clone();
                let mut out = Vec::new();
                for f in fields {
                    let Some(idx) = v.fields.iter().position(|x| x.name == f.name.name) else {
                        self.err(Diagnostic::error(tyc::NO_FIELD, f.name.span, format!("`{}` has no field named `{}`", v.name, f.name.name)));
                        continue;
                    };
                    self.check_field_privacy(adt, idx, f.name.span);
                    self.s.p.refs.push(crate::Ref { span: f.name.span, target: crate::RefTarget::Field { adt, variant, field: idx } });
                    let ft = v.fields[idx].ty.subst(&args);
                    out.push((idx, self.check_pat(&f.pat, &ft, false)));
                }
                if !rest {
                    let missing: Vec<&str> =
                        v.fields.iter().enumerate().filter(|(i, _)| !out.iter().any(|(j, _)| j == i)).map(|(_, f)| f.name.as_str()).collect();
                    if !missing.is_empty() {
                        self.err(
                            Diagnostic::error(tyc::BAD_PATTERN, span, format!("pattern does not mention field(s) {}", missing.join(", ")))
                                .with_help("add the fields or ignore the rest with `..`"),
                        );
                    }
                }
                PatKind::Adt { def: adt, variant, fields: out }
            }
            ast::PatKind::Path(path) => {
                // Unit variants, unit structs and constants; unknown single names bind.
                if path.is_single() && self.silent_value_kind(path).is_none() {
                    return Pat { kind: self.binding(&path.segments[0].ident, false, ty, span), ty: ty.clone(), span };
                }
                match self.resolve_value_path(path, path.span) {
                    Some(ValueRes::Ctor { adt, variant, args }) => {
                        let adt_ty = Ty::Adt(adt, args);
                        if !self.unify(&adt_ty, ty) {
                            self.mismatch(ty, &adt_ty, span);
                        }
                        let v = &self.s.p.adt(adt).variants[variant];
                        if v.ctor != CtorKind::Unit {
                            let n = v.name.clone();
                            self.err(Diagnostic::error(tyc::BAD_PATTERN, span, format!("`{n}` has fields; match them with `{n}(..)` or `{n} {{ .. }}`")));
                        }
                        PatKind::Adt { def: adt, variant, fields: vec![] }
                    }
                    Some(ValueRes::Const(d)) => {
                        let DefData::Const(c) = &self.s.p.def(d).data else { unreachable!() };
                        let (cty, val) = (c.ty.clone(), c.value.clone());
                        if !self.unify(&cty, ty) {
                            self.mismatch(ty, &cty, span);
                        }
                        match val {
                            Some(v) => PatKind::Lit(v),
                            None => PatKind::Wild,
                        }
                    }
                    Some(_) => {
                        self.err(Diagnostic::error(tyc::BAD_PATTERN, path.span, "expected a unit variant, unit struct or constant"));
                        PatKind::Wild
                    }
                    None => PatKind::Wild,
                }
            }
            ast::PatKind::Or(alts) => {
                // The first alternative declares the variables; the others
                // bind the same ones (see `binding`).
                let start = self.or_binds.len();
                self.or_frames.push(OrFrame { start, alt: 0 });
                let mut ps = Vec::new();
                for (i, a) in alts.iter().enumerate() {
                    self.or_frames.last_mut().unwrap().alt = i;
                    ps.push(self.check_pat(a, ty, false));
                }
                self.or_frames.pop();
                self.check_or_binds(alts, start);
                // The first alternative's variables are the pattern's, for a
                // `|` pattern around this one.
                let first: Vec<OrBind> = self.or_binds[start..].iter().filter(|b| b.alt == 0).cloned().collect();
                self.or_binds.truncate(start);
                if let Some(f) = self.or_frames.last() {
                    let alt = f.alt;
                    self.or_binds.extend(first.into_iter().map(|b| OrBind { alt, ..b }));
                }
                PatKind::Or(ps)
            }
        };
        Pat { kind, ty: ty.clone(), span }
    }

    /// Whether a single-segment path names a constructor or constant (no errors).
    fn silent_value_kind(&self, path: &ast::Path) -> Option<DefKind> {
        match self.s.lookup_ident(self.module(), &path.segments[0].ident, Ns::Value) {
            Some(crate::resolve::ModRes::Def(d)) => {
                let k = self.s.p.def(d).kind;
                matches!(k, DefKind::Variant | DefKind::Struct | DefKind::Const).then_some(k)
            }
            _ => None,
        }
    }

    fn binding(&mut self, name: &ast::Ident, is_mut: bool, ty: &Ty, span: Span) -> PatKind {
        let l = match self.or_binding(name) {
            Some(l) => {
                self.rebind(l, name, is_mut, ty, span);
                l
            }
            None => {
                let depth = self.depth();
                let l = self.new_local(&name.name, ty.clone(), is_mut, LocalKind::Let, span, depth);
                self.bind_name(&name.name, name.ctx, l);
                l
            }
        };
        if let Some(f) = self.or_frames.last() {
            let alt = f.alt;
            self.or_binds.push(OrBind { alt, name: name.name.clone(), ctx: name.ctx, local: l, span });
        }
        let r = self.resolve(ty);
        let mode = if !r.has_infer() && self.s.p.is_copy(&r, &self.env) { ReadMode::Copy } else { ReadMode::Move };
        PatKind::Binding { local: l, mode, sub: None }
    }

    /// In a later alternative of a `|` pattern, the variable that the first
    /// alternative bound under this name (of the innermost such pattern).
    fn or_binding(&self, name: &ast::Ident) -> Option<LocalId> {
        for (k, f) in self.or_frames.iter().enumerate().rev() {
            if f.alt == 0 {
                continue;
            }
            let end = self.or_frames.get(k + 1).map_or(self.or_binds.len(), |n| n.start);
            if let Some(b) = self.or_binds[f.start..end].iter().find(|b| b.alt == 0 && b.name == name.name && b.ctx == name.ctx) {
                return Some(b.local);
            }
        }
        None
    }

    /// Bind `l` again, in a later alternative of a `|` pattern: it must have
    /// the same type and mutability there.
    fn rebind(&mut self, l: LocalId, name: &ast::Ident, is_mut: bool, ty: &Ty, span: Span) {
        self.record_local(span, l);
        let (lty, mutable, first) = {
            let x = &self.locals[l.0 as usize];
            (x.ty.clone(), x.mutable, x.span)
        };
        if mutable != is_mut {
            let n = &name.name;
            self.err(
                Diagnostic::error(ln_diag::codes::name::INCONSISTENT_BINDING, span, format!("variable `{n}` is bound inconsistently across `|` patterns"))
                    .with_label(if is_mut { "bound with `mut` here" } else { "bound without `mut` here" })
                    .with_secondary(first, if mutable { "first bound with `mut` here" } else { "first bound without `mut` here" })
                    .with_help("write `mut` in every alternative or in none"),
            );
        }
        if !self.unify(&lty, ty) {
            let (re, rf) = (self.resolve(&lty), self.resolve(ty));
            if !re.has_error() && !rf.has_error() {
                let (e, f) = (self.s.p.ty_str(&re), self.s.p.ty_str(&rf));
                self.err(
                    Diagnostic::error(tyc::MISMATCH, span, format!("mismatched types: expected `{e}`, found `{f}`"))
                        .with_label(format!("expected `{e}`"))
                        .with_secondary(first, format!("first bound here with type `{e}`"))
                        .with_note("a variable bound by a `|` pattern has the same type in every alternative"),
                );
            }
        }
    }

    /// Every alternative of a `|` pattern binds the same variables: report
    /// each variable that an alternative leaves out (`start`: where the
    /// pattern's bindings begin in `or_binds`).
    fn check_or_binds(&mut self, alts: &[ast::Pat], start: usize) {
        let binds = self.or_binds[start..].to_vec();
        for (i, b) in binds.iter().enumerate() {
            // Each variable once, where it is first bound.
            if binds[..i].iter().any(|x| x.name == b.name && x.ctx == b.ctx) {
                continue;
            }
            for (k, a) in alts.iter().enumerate() {
                if binds.iter().any(|x| x.alt == k && x.name == b.name && x.ctx == b.ctx) {
                    continue;
                }
                let n = &b.name;
                self.err(
                    Diagnostic::error(ln_diag::codes::name::UNBOUND_IN_ALTERNATIVE, a.span, format!("variable `{n}` is not bound in all patterns"))
                        .with_label(format!("this alternative does not bind `{n}`"))
                        .with_secondary(b.span, format!("`{n}` is bound here"))
                        .with_help(format!("bind `{n}` in every alternative, or match them in separate arms")),
                );
            }
        }
    }

    /// Whether `pat` matches every value of `ty`: it is irrefutable, or its
    /// `|` alternatives cover the type together (`Ok(v) | Err(v)`).
    pub(crate) fn pat_covers(&mut self, pat: &Pat, ty: &Ty) -> bool {
        if pat.is_irrefutable(&|d| self.s.p.is_enum(d)) {
            return true;
        }
        let ty = self.resolve(ty);
        if ty.has_error() {
            return false;
        }
        let rows: Vec<Vec<Lite>> = expand(&lite(pat)).into_iter().map(|p| vec![p]).collect();
        self.missing(&rows, &[ty]).is_none()
    }

    fn lit_pat(&mut self, l: &ast::Lit, neg: bool, ty: &Ty, span: Span) -> PatKind {
        let e = self.check_lit(l, Some(ty), span);
        if !self.unify(&e.ty, ty) {
            self.mismatch(ty, &e.ty, span);
        }
        match e.kind {
            thir::ExprKind::Lit(ConstVal::Int(i)) => {
                let v = if neg { -i } else { i };
                if let Some(last) = self.lit_checks.last_mut() {
                    last.1 = v;
                }
                PatKind::Lit(ConstVal::Int(v))
            }
            thir::ExprKind::Lit(ConstVal::Float(f)) => PatKind::Lit(ConstVal::Float(if neg { -f } else { f })),
            thir::ExprKind::Lit(v) => PatKind::Lit(v),
            _ => PatKind::Wild,
        }
    }

    /// Positional sub-patterns with an optional `..`.
    fn tuple_like(&mut self, ps: &[ast::Pat], tys: &[Ty], rest: Option<usize>, span: Span) -> Vec<(usize, Pat)> {
        let mut out = Vec::new();
        match rest {
            None => {
                if ps.len() != tys.len() {
                    self.err(Diagnostic::error(tyc::BAD_PATTERN, span, format!("expected {} field(s) in pattern, found {}", tys.len(), ps.len())));
                }
                for (i, (p, t)) in ps.iter().zip(tys).enumerate() {
                    out.push((i, self.check_pat(p, t, false)));
                }
            }
            Some(r) => {
                let before = &ps[..r];
                let after = &ps[r + 1..];
                if before.len() + after.len() > tys.len() {
                    self.err(Diagnostic::error(tyc::BAD_PATTERN, span, "too many fields in pattern"));
                    return out;
                }
                for (i, p) in before.iter().enumerate() {
                    out.push((i, self.check_pat(p, &tys[i], false)));
                }
                let start = tys.len() - after.len();
                for (k, p) in after.iter().enumerate() {
                    out.push((start + k, self.check_pat(p, &tys[start + k], false)));
                }
                // Fill the elided positions with wildcards (tuples need every slot).
                for i in before.len()..start {
                    out.push((i, Pat { kind: PatKind::Wild, ty: tys[i].clone(), span }));
                }
                out.sort_by_key(|(i, _)| *i);
            }
        }
        out
    }

    // ===================================================================== exhaustiveness

    pub(crate) fn check_exhaustive(&mut self, scrut_ty: &Ty, arms: &[Arm], span: Span) {
        let ty = self.resolve(scrut_ty);
        if ty.has_error() {
            return;
        }
        let rows: Vec<Vec<Lite>> = arms.iter().filter(|a| a.guard.is_none()).flat_map(|a| expand(&lite(&a.pat))).map(|p| vec![p]).collect();
        if let Some(w) = self.missing(&rows, &[ty.clone()]) {
            let w = w.into_iter().next().unwrap_or_else(|| "_".into());
            self.err(
                Diagnostic::error(tyc::NON_EXHAUSTIVE, span, format!("non-exhaustive patterns: `{w}` not covered"))
                    .with_help(format!("add an arm for `{w}`, or a wildcard arm `_ => ...`")),
            );
        }
    }

    /// Returns a witness (one string per column) of a value not matched by `rows`.
    fn missing(&mut self, rows: &[Vec<Lite>], tys: &[Ty]) -> Option<Vec<String>> {
        if tys.is_empty() {
            return if rows.is_empty() { Some(vec![]) } else { None };
        }
        let head_ty = self.resolve(&tys[0]);
        let ctors = self.ctors_of(&head_ty);
        let rest_tys = &tys[1..];
        match ctors {
            Some(all) => {
                let used: Vec<usize> = rows.iter().filter_map(|r| if let Lite::Ctor(c, _) = &r[0] { Some(*c) } else { None }).collect();
                let complete = all.iter().all(|(c, _, _)| used.contains(c));
                if complete || rows.iter().all(|r| matches!(r[0], Lite::Ctor(..))) {
                    for (c, name, field_tys) in &all {
                        let spec: Vec<Vec<Lite>> = rows
                            .iter()
                            .filter_map(|r| match &r[0] {
                                // Fields the pattern does not mention (`{ .. }`, trailing
                                // `..`) are wildcards.
                                Lite::Ctor(rc, subs) if rc == c => Some(
                                    subs.iter()
                                        .cloned()
                                        .chain(std::iter::repeat_n(Lite::Wild, field_tys.len().saturating_sub(subs.len())))
                                        .chain(r[1..].iter().cloned())
                                        .collect(),
                                ),
                                Lite::Wild => Some(std::iter::repeat_n(Lite::Wild, field_tys.len()).chain(r[1..].iter().cloned()).collect()),
                                _ => None,
                            })
                            .collect();
                        let mut sub_tys = field_tys.clone();
                        sub_tys.extend(rest_tys.iter().cloned());
                        if let Some(w) = self.missing(&spec, &sub_tys) {
                            let (fields, rest) = w.split_at(field_tys.len());
                            let head = if field_tys.is_empty() {
                                name.clone()
                            } else if name.is_empty() {
                                format!("({})", fields.join(", "))
                            } else {
                                format!("{name}({})", fields.join(", "))
                            };
                            let mut out = vec![head];
                            out.extend(rest.iter().cloned());
                            return Some(out);
                        }
                    }
                    None
                } else {
                    let default: Vec<Vec<Lite>> = rows.iter().filter(|r| matches!(r[0], Lite::Wild)).map(|r| r[1..].to_vec()).collect();
                    let w = self.missing(&default, rest_tys)?;
                    let missing_ctor = all
                        .iter()
                        .find(|(c, _, _)| !used.contains(c))
                        .map(|(_, n, f)| if f.is_empty() { n.clone() } else { format!("{n}(..)") })
                        .unwrap_or_else(|| "_".into());
                    let mut out = vec![missing_ctor];
                    out.extend(w);
                    Some(out)
                }
            }
            None => {
                // Infinite types: only wildcards cover.
                let default: Vec<Vec<Lite>> = rows.iter().filter(|r| matches!(r[0], Lite::Wild)).map(|r| r[1..].to_vec()).collect();
                let w = self.missing(&default, rest_tys)?;
                let mut out = vec!["_".to_string()];
                out.extend(w);
                Some(out)
            }
        }
    }

    /// Constructors of a type: (id, display name, field types). `None` for
    /// types with infinitely many values.
    fn ctors_of(&self, t: &Ty) -> Option<Vec<(usize, String, Vec<Ty>)>> {
        match t {
            Ty::Bool => Some(vec![(0, "false".into(), vec![]), (1, "true".into(), vec![])]),
            Ty::Tuple(ts) => Some(vec![(0, String::new(), ts.clone())]),
            Ty::Adt(d, args) => {
                let adt = self.s.p.adt(*d);
                Some(
                    adt.variants
                        .iter()
                        .enumerate()
                        .map(|(i, v)| {
                            let name = if adt.is_enum { v.name.clone() } else { v.name.clone() };
                            (i, name, v.fields.iter().map(|f| f.ty.subst(args)).collect())
                        })
                        .collect(),
                )
            }
            _ => None,
        }
    }
}

/// A `|` pattern being checked.
pub(crate) struct OrFrame {
    /// Where its bindings begin in `FnCtx::or_binds`.
    start: usize,
    /// The alternative being checked.
    alt: usize,
}

/// A variable bound by an alternative of a `|` pattern.
#[derive(Clone)]
pub(crate) struct OrBind {
    alt: usize,
    name: String,
    ctx: ast::SyntaxCtx,
    local: LocalId,
    /// The binding (`x`, `mut x`).
    span: Span,
}

/// Simplified pattern for exhaustiveness checking.
#[derive(Clone, Debug)]
enum Lite {
    Wild,
    Ctor(usize, Vec<Lite>),
    /// A literal or range: never covers a type fully.
    Lit,
    Or(Vec<Lite>),
}

fn lite(p: &Pat) -> Lite {
    match &p.kind {
        PatKind::Wild => Lite::Wild,
        PatKind::Binding { sub: None, .. } => Lite::Wild,
        PatKind::Binding { sub: Some(s), .. } => lite(s),
        PatKind::Lit(ConstVal::Bool(b)) => Lite::Ctor(*b as usize, vec![]),
        PatKind::Lit(_) | PatKind::Range { .. } => Lite::Lit,
        PatKind::Tuple(ps) => Lite::Ctor(0, ps.iter().map(lite).collect()),
        PatKind::Adt { variant, fields, def: _ } => {
            // Fields not mentioned are wildcards; order by index.
            let n = fields.iter().map(|(i, _)| i + 1).max().unwrap_or(0);
            let mut subs = vec![Lite::Wild; n];
            for (i, f) in fields {
                subs[*i] = lite(f);
            }
            Lite::Ctor(*variant, subs)
        }
        PatKind::Or(ps) => Lite::Or(ps.iter().map(lite).collect()),
    }
}

/// Expand or-patterns at the top level into separate rows.
fn expand(p: &Lite) -> Vec<Lite> {
    match p {
        Lite::Or(ps) => ps.iter().flat_map(expand).collect(),
        Lite::Ctor(c, subs) => {
            // Expand nested or-patterns (cartesian product).
            let mut acc: Vec<Vec<Lite>> = vec![vec![]];
            for s in subs {
                let alts = expand(s);
                acc = acc
                    .into_iter()
                    .flat_map(|prefix| {
                        alts.iter().map(move |a| {
                            let mut v = prefix.clone();
                            v.push(a.clone());
                            v
                        })
                    })
                    .collect();
            }
            acc.into_iter().map(|v| Lite::Ctor(*c, v)).collect()
        }
        other => vec![other.clone()],
    }
}

#[allow(dead_code)]
fn _unused(_: Span) {}
