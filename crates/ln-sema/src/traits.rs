//! Trait solving: impl selection, the `Dull` auto trait, builtin impls for
//! primitives, and type classification (owners, copyability).
//!
//! These queries run on a finished [`Program`] so code generation can use
//! them during monomorphization.

use crate::Program;
use crate::defs::*;
use crate::ty::{TraitRef, Ty};
use std::collections::HashMap;

/// How a trait method call is implemented for a concrete self type.
#[derive(Clone, Debug)]
pub enum MethodImpl {
    /// A real function: `def` instantiated with `args`.
    Fn { def: DefId, args: Vec<Ty> },
    /// A compiler-provided implementation (primitive operators, copies, ...),
    /// identified by the trait's lang name and the method name.
    Builtin { lang: String, method: String },
}

/// Operator traits that primitives implement natively.
pub const OP_TRAITS: &[&str] = &["add", "sub", "mul", "div", "rem", "neg", "not", "bitand", "bitor", "bitxor", "shl", "shr"];

impl Program {
    pub fn lang_name(&self, trait_id: DefId) -> Option<&str> {
        self.def(trait_id).attrs.lang.as_deref()
    }

    pub fn is_lang(&self, trait_id: DefId, name: &str) -> bool {
        self.lang_name(trait_id) == Some(name)
    }

    /// Supertrait closure of a set of predicates.
    pub fn elaborate(&self, env: &[Pred]) -> Vec<Pred> {
        let mut out: Vec<Pred> = Vec::new();
        let mut stack: Vec<Pred> = env.to_vec();
        while let Some(p) = stack.pop() {
            if out.iter().any(|q| q.ty == p.ty && q.trait_ref == p.trait_ref) {
                continue;
            }
            if let DefData::Trait(t) = &self.def(p.trait_ref.trait_id).data {
                // Supertraits are expressed with Self = Param(0) and the trait's params after.
                let mut args = vec![p.ty.clone()];
                args.extend(p.trait_ref.args.iter().cloned());
                for st in &t.supertraits {
                    stack.push(Pred { ty: p.ty.clone(), trait_ref: st.subst(&args), span: p.span });
                }
            }
            out.push(p);
        }
        out
    }

    /// Whether `ty` is Dull: contains no pointers or arrays.
    pub fn is_dull(&self, ty: &Ty, env: &[Pred]) -> bool {
        self.is_dull_rec(ty, env, 0)
    }

    fn is_dull_rec(&self, ty: &Ty, env: &[Pred], depth: u32) -> bool {
        if depth > 64 {
            return true; // recursive types are reported elsewhere
        }
        match ty {
            Ty::Int(_) | Ty::Float(_) | Ty::Bool | Ty::Char | Ty::Never | Ty::Error => true,
            Ty::Tuple(ts) => ts.iter().all(|t| self.is_dull_rec(t, env, depth + 1)),
            Ty::Adt(id, args) => {
                let adt = self.adt(*id);
                adt.variants.iter().all(|v| v.fields.iter().all(|f| self.is_dull_rec(&f.ty.subst(args), env, depth + 1)))
            }
            Ty::Param(..) => {
                let Some(dull) = self.lang_item("dull") else { return false };
                self.elaborate(env).iter().any(|p| p.ty == *ty && p.trait_ref.trait_id == dull)
            }
            // A closure is Dull when it holds only Dull values (no borrows).
            Ty::Closure(def, args) => self.closure_fields(*def, args).iter().all(|t| self.is_dull_rec(t, env, depth + 1)),
            // `impl Trait` is Dull when its bounds say so.
            Ty::Opaque(def, i, args) => {
                let Some(dull) = self.lang_item("dull") else { return false };
                self.elaborate(&self.opaque_preds(*def, *i, args)).iter().any(|p| p.trait_ref.trait_id == dull)
            }
            Ty::Str | Ty::String | Ty::FmtArgs | Ty::Array(..) | Ty::Grow(_) | Ty::Ptr(..) | Ty::FnPtr(..) | Ty::Infer(_) => false,
        }
    }

    /// Whether `ty` is Light: owns no array or string (Dull types, views,
    /// pointers, and anything built only from those). Light types are the
    /// ones copied implicitly, and the only possible array items.
    pub fn is_light(&self, ty: &Ty, env: &[Pred]) -> bool {
        self.is_light_rec(ty, env, 0)
    }

    fn is_light_rec(&self, ty: &Ty, env: &[Pred], depth: u32) -> bool {
        if depth > 64 {
            return true; // recursive types are reported elsewhere
        }
        match ty {
            Ty::Int(_) | Ty::Float(_) | Ty::Bool | Ty::Char | Ty::Never | Ty::Error => true,
            Ty::Str | Ty::FmtArgs | Ty::Ptr(..) | Ty::FnPtr(..) => true,
            Ty::String => false,
            Ty::Array(e, k) => !k.is_owner() && self.is_light_rec(e, env, depth + 1),
            Ty::Grow(t) => self.is_light_rec(t, env, depth + 1),
            Ty::Tuple(ts) => ts.iter().all(|t| self.is_light_rec(t, env, depth + 1)),
            Ty::Adt(id, args) => {
                let adt = self.adt(*id);
                adt.variants.iter().all(|v| v.fields.iter().all(|f| self.is_light_rec(&f.ty.subst(args), env, depth + 1)))
            }
            // `T: Dull` implies `T: Light` (`Dull` is a subtrait).
            Ty::Param(..) => {
                let (Some(light), dull) = (self.lang_item("light"), self.lang_item("dull")) else { return false };
                self.elaborate(env).iter().any(|p| p.ty == *ty && (p.trait_ref.trait_id == light || Some(p.trait_ref.trait_id) == dull))
            }
            Ty::Closure(def, args) => self.closure_fields(*def, args).iter().all(|t| self.is_light_rec(t, env, depth + 1)),
            Ty::Opaque(def, i, args) => {
                let (Some(light), dull) = (self.lang_item("light"), self.lang_item("dull")) else { return false };
                self.elaborate(&self.opaque_preds(*def, *i, args)).iter().any(|p| p.trait_ref.trait_id == light || Some(p.trait_ref.trait_id) == dull)
            }
            Ty::Infer(_) => false,
        }
    }

    /// Whether a value of `ty` owns a data stack (directly or in a field).
    pub fn has_owner(&self, ty: &Ty) -> bool {
        self.has_owner_rec(ty, 0)
    }

    fn has_owner_rec(&self, ty: &Ty, depth: u32) -> bool {
        if depth > 64 {
            return false;
        }
        match ty {
            Ty::String => true,
            Ty::Array(_, k) => k.is_owner(),
            Ty::Tuple(ts) => ts.iter().any(|t| self.has_owner_rec(t, depth + 1)),
            Ty::Adt(id, args) => {
                let adt = self.adt(*id);
                adt.variants.iter().any(|v| v.fields.iter().any(|f| self.has_owner_rec(&f.ty.subst(args), depth + 1)))
            }
            Ty::Grow(t) => self.has_owner_rec(t, depth + 1),
            Ty::Closure(def, args) => self.closure_fields(*def, args).iter().any(|t| self.has_owner_rec(t, depth + 1)),
            _ => false,
        }
    }

    /// Whether a value of `ty` may own a data stack (an array or `String`,
    /// directly or in a field), so that changing it in place could shrink
    /// or free one: a valid target for `+`.
    pub fn could_own(&self, ty: &Ty, env: &[Pred]) -> bool {
        self.could_own_rec(ty, env, 0)
    }

    fn could_own_rec(&self, ty: &Ty, env: &[Pred], depth: u32) -> bool {
        if depth > 64 {
            return false;
        }
        match ty {
            Ty::String => true,
            Ty::Array(_, k) => k.is_owner(),
            // A generic parameter might be instantiated with an owner.
            Ty::Param(..) | Ty::Opaque(..) => !self.is_light(ty, env),
            Ty::Tuple(ts) => ts.iter().any(|t| self.could_own_rec(t, env, depth + 1)),
            Ty::Adt(id, args) => {
                let adt = self.adt(*id);
                adt.variants.iter().any(|v| v.fields.iter().any(|f| self.could_own_rec(&f.ty.subst(args), env, depth + 1)))
            }
            Ty::Closure(def, args) => self.closure_fields(*def, args).iter().any(|t| self.could_own_rec(t, env, depth + 1)),
            _ => false,
        }
    }

    /// Whether values of `ty` are implicitly copied (rather than moved).
    /// Generic parameters are copyable only when bounded by `Light` (or
    /// `Dull`).
    pub fn is_copy(&self, ty: &Ty, env: &[Pred]) -> bool {
        self.is_copy_rec(ty, env, 0)
    }

    fn is_copy_rec(&self, ty: &Ty, env: &[Pred], depth: u32) -> bool {
        if depth > 64 {
            return true;
        }
        match ty {
            Ty::String => false,
            Ty::Array(e, k) => !k.is_owner() && self.is_copy_rec(e, env, depth + 1),
            Ty::Param(..) | Ty::Opaque(..) => self.is_light(ty, env),
            Ty::Tuple(ts) => ts.iter().all(|t| self.is_copy_rec(t, env, depth + 1)),
            Ty::Adt(id, args) => {
                let adt = self.adt(*id);
                adt.variants.iter().all(|v| v.fields.iter().all(|f| self.is_copy_rec(&f.ty.subst(args), env, depth + 1)))
            }
            Ty::Grow(t) => self.is_copy_rec(t, env, depth + 1),
            // A closure that changes a variable it borrows is not copied: two
            // copies would both change it.
            Ty::Closure(def, args) => {
                let changes_borrowed =
                    self.closures.get(def).is_some_and(|c| c.captures.iter().any(|cap| cap.by_ref && cap.mode == crate::defs::ParamMode::InOut));
                !changes_borrowed && self.closure_fields(*def, args).iter().all(|t| self.is_copy_rec(t, env, depth + 1))
            }
            _ => true,
        }
    }

    /// Builtin trait implementations for primitive types.
    pub fn builtin_impl(&self, ty: &Ty, trait_id: DefId, env: &[Pred]) -> bool {
        let Some(lang) = self.lang_name(trait_id) else { return false };
        let int = matches!(ty, Ty::Int(_));
        let signed = matches!(ty, Ty::Int(i) if i.signed());
        let float = matches!(ty, Ty::Float(_));
        match lang {
            "dull" => self.is_dull(ty, env),
            "light" => self.is_light(ty, env),
            "add" | "sub" | "mul" | "div" | "rem" => int || float,
            "neg" => signed || float,
            "not" => int || matches!(ty, Ty::Bool),
            "bitand" | "bitor" | "bitxor" => int || matches!(ty, Ty::Bool),
            "shl" | "shr" => int,
            "eq" => int || float || matches!(ty, Ty::Bool | Ty::Char | Ty::Str) || ty.is_unit(),
            "ord" => int || float || matches!(ty, Ty::Char | Ty::Str),
            "clone" => self.is_copy(ty, env) && !matches!(ty, Ty::Param(..)),
            "default" => int || float || matches!(ty, Ty::Bool | Ty::Char | Ty::Str) || ty.is_unit(),
            _ => false,
        }
    }

    /// One-way matching of an impl's self type `pat` (containing the impl's
    /// params) against `ty`, filling `out`.
    pub fn match_ty(pat: &Ty, ty: &Ty, out: &mut Vec<Option<Ty>>) -> bool {
        match (pat, ty) {
            (Ty::Param(i, _), t) => {
                let i = *i as usize;
                if out.len() <= i {
                    out.resize(i + 1, None);
                }
                match &out[i] {
                    Some(prev) => prev == t,
                    None => {
                        out[i] = Some(t.clone());
                        true
                    }
                }
            }
            (Ty::Tuple(a), Ty::Tuple(b)) => a.len() == b.len() && a.iter().zip(b).all(|(x, y)| Self::match_ty(x, y, out)),
            (Ty::Adt(a, xs), Ty::Adt(b, ys)) => a == b && xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| Self::match_ty(x, y, out)),
            (Ty::Array(a, k1), Ty::Array(b, k2)) => k1 == k2 && Self::match_ty(a, b, out),
            (Ty::Ptr(a, m1), Ty::Ptr(b, m2)) => m1 == m2 && Self::match_ty(a, b, out),
            (Ty::Grow(a), Ty::Grow(b)) => Self::match_ty(a, b, out),
            (Ty::FnPtr(a, r1), Ty::FnPtr(b, r2)) => {
                a.len() == b.len() && a.iter().zip(b).all(|((m1, x), (m2, y))| m1 == m2 && Self::match_ty(x, y, out)) && Self::match_ty(r1, r2, out)
            }
            (a, b) => a == b,
        }
    }

    /// Find a user impl of `tr` for `ty`. Returns the impl and its generic args.
    pub fn find_impl(&self, ty: &Ty, tr: &TraitRef, env: &[Pred]) -> Option<(DefId, Vec<Ty>)> {
        self.find_impl_depth(ty, tr, env, 0)
    }

    fn find_impl_depth(&self, ty: &Ty, tr: &TraitRef, env: &[Pred], depth: u32) -> Option<(DefId, Vec<Ty>)> {
        if depth > 32 {
            return None;
        }
        for &imp in &self.impls {
            let DefData::Impl(data) = &self.def(imp).data else { continue };
            let Some(itr) = &data.trait_ref else { continue };
            if itr.trait_id != tr.trait_id || data.negative {
                continue;
            }
            let mut subst: Vec<Option<Ty>> = vec![None; data.generics.count()];
            if !Self::match_ty(&data.self_ty, ty, &mut subst) {
                continue;
            }
            if !itr.args.iter().zip(&tr.args).all(|(p, t)| Self::match_ty(p, t, &mut subst)) {
                continue;
            }
            let args: Vec<Ty> = subst.into_iter().map(|t| t.unwrap_or(Ty::Error)).collect();
            let ok = data.generics.preds.iter().all(|p| {
                let pty = p.ty.subst(&args);
                let ptr = p.trait_ref.subst(&args);
                self.implements_depth(&pty, &ptr, env, depth + 1)
            });
            if ok {
                return Some((imp, args));
            }
        }
        None
    }

    /// Whether `ty: tr` holds in environment `env`.
    pub fn implements(&self, ty: &Ty, tr: &TraitRef, env: &[Pred]) -> bool {
        self.implements_depth(ty, tr, env, 0)
    }

    fn implements_depth(&self, ty: &Ty, tr: &TraitRef, env: &[Pred], depth: u32) -> bool {
        if ty.is_error() || tr.args.iter().any(Ty::is_error) {
            return true;
        }
        if let Ty::Grow(inner) = ty {
            return self.implements_depth(inner, tr, env, depth);
        }
        if matches!(ty, Ty::Param(..)) || ty.has_params() {
            if self.elaborate(env).iter().any(|p| p.ty == *ty && p.trait_ref == *tr) {
                return true;
            }
        }
        // `impl Trait`: only what its bounds promise.
        if let Ty::Opaque(def, i, args) = ty {
            return self.elaborate(&self.opaque_preds(*def, *i, args)).iter().any(|p| p.trait_ref == *tr);
        }
        // `Fn(A): R`, `FnMut(..)`, `FnOnce(..)`: closures of that kind or a
        // weaker one, and function pointers, with exactly that signature.
        if let Some(want) = self.lang_name(tr.trait_id).and_then(crate::ClosureKind::from_lang) {
            let sig = tr.args.first();
            return match ty {
                Ty::Closure(def, args) => self.closures.get(def).is_some_and(|c| c.kind <= want) && sig.is_none_or(|s| *s == self.closure_sig(*def, args)),
                Ty::FnPtr(..) => sig.is_none_or(|s| s == ty),
                _ => false,
            };
        }
        if self.builtin_impl(ty, tr.trait_id, env) {
            return true;
        }
        if self.is_lang(tr.trait_id, "dull") || self.is_lang(tr.trait_id, "light") {
            return false;
        }
        self.find_impl_depth(ty, tr, env, depth).is_some()
    }

    /// Resolve the implementation of trait method `method` for concrete `self_ty`.
    pub fn resolve_method(&self, tr: &TraitRef, self_ty: &Ty, method: &str, method_args: &[Ty]) -> Option<MethodImpl> {
        let trait_data = self.trait_data(tr.trait_id);
        let lang = self.lang_name(tr.trait_id).map(str::to_string);
        if let Some(lang) = &lang
            && self.builtin_impl(self_ty, tr.trait_id, &[])
            && lang != "dull"
            && lang != "light"
        {
            return Some(MethodImpl::Builtin { lang: lang.clone(), method: method.to_string() });
        }
        if let Some((imp, mut args)) = self.find_impl(self_ty, tr, &[]) {
            if let Some(&m) = self.impl_data(imp).methods.get(method) {
                args.extend(method_args.iter().cloned());
                return Some(MethodImpl::Fn { def: m, args });
            }
        }
        // Default method body from the trait.
        let tm = trait_data.methods.iter().find(|m| m.name == method)?;
        if !tm.has_default {
            return None;
        }
        let mut args = vec![self_ty.clone()];
        args.extend(tr.args.iter().cloned());
        args.extend(method_args.iter().cloned());
        Some(MethodImpl::Fn { def: tm.def, args })
    }
}

impl Program {
    /// Whether two impls of the same trait could both apply to some type
    /// (coherence). Their headers must unify, and no bound of either impl may
    /// be definitely false for the unified types: `impl::<T: Dull> X for T`
    /// and `impl X for String` do not overlap, because `String` is not Dull.
    ///
    /// "Not Dull" only separates impls when it cannot change under the other
    /// impl's feet: the type is pointy because of the language (strings,
    /// arrays, slices, pointers) or because of types defined in the same
    /// crater as that other impl. A type from another crater might lose its
    /// last pointy field in a later version and become Dull.
    ///
    /// Returns `None` if the impls cannot overlap, else notes explaining
    /// why a bound did not separate them.
    pub fn impls_overlap(&self, a: DefId, b: DefId) -> Option<Vec<String>> {
        let (da, db) = (self.impl_data(a), self.impl_data(b));
        let (Some(ta), Some(tb)) = (&da.trait_ref, &db.trait_ref) else { return None };
        if ta.trait_id != tb.trait_id || da.negative || db.negative {
            return None;
        }
        // Each impl's generics become distinct unification variables.
        let na = da.generics.count();
        let va: Vec<Ty> = (0..na).map(|i| Ty::Infer(i as u32)).collect();
        let vb: Vec<Ty> = (0..db.generics.count()).map(|i| Ty::Infer((na + i) as u32)).collect();
        let mut s: HashMap<u32, Ty> = HashMap::new();
        let mut pairs = vec![(da.self_ty.subst(&va), db.self_ty.subst(&vb))];
        pairs.extend(ta.args.iter().zip(&tb.args).map(|(x, y)| (x.subst(&va), y.subst(&vb))));
        if !pairs.iter().all(|(x, y)| unify(x, y, &mut s)) {
            return None;
        }
        let dull = self.lang.get("dull").copied();
        let mut notes = Vec::new();
        for (d, v, other) in [(da, &va, b), (db, &vb, a)] {
            for p in &d.generics.preds {
                let ty = resolve(&p.ty.subst(v), &s);
                let mut tr = p.trait_ref.subst(v);
                tr.args = tr.args.iter().map(|t| resolve(t, &s)).collect();
                let concrete = !has_infer(&ty) && !tr.args.iter().any(has_infer);
                if !concrete || self.implements(&ty, &tr, &[]) {
                    continue;
                }
                if Some(tr.trait_id) == dull {
                    let crater = self.crater_of(other);
                    if self.robustly_pointy(&ty, crater, 0) {
                        return None;
                    }
                    notes.push(format!(
                        "`{}` is not Dull today, but that depends on types from another crater, which might drop their last pointer or array field in a later version",
                        self.ty_str(&ty)
                    ));
                    continue;
                }
                return None;
            }
        }
        Some(notes)
    }

    /// The crater (root module) a definition belongs to.
    pub fn crater_of(&self, d: DefId) -> DefId {
        let m = if self.def(d).kind == DefKind::Mod { d } else { self.def(d).module };
        match &self.def(m).data {
            DefData::Mod(md) => md.krate,
            _ => m,
        }
    }

    /// Whether `ty` is pointy for reasons that `crater` controls: language
    /// types that are always pointy, or fields of types defined in `crater`.
    fn robustly_pointy(&self, ty: &Ty, crater: DefId, depth: u32) -> bool {
        if depth > 32 {
            return false;
        }
        match ty {
            Ty::Str | Ty::String | Ty::FmtArgs | Ty::Ptr(..) | Ty::FnPtr(..) | Ty::Array(..) => true,
            Ty::Grow(t) => self.robustly_pointy(t, crater, depth + 1),
            Ty::Tuple(ts) => ts.iter().any(|t| self.robustly_pointy(t, crater, depth + 1)),
            Ty::Adt(d, args) if self.crater_of(*d) == crater => {
                let DefData::Adt(adt) = &self.def(*d).data else { return false };
                adt.variants.iter().any(|v| v.fields.iter().any(|f| self.robustly_pointy(&f.ty.subst(args), crater, depth + 1)))
            }
            _ => false,
        }
    }
}

fn shallow(t: &Ty, s: &HashMap<u32, Ty>) -> Ty {
    let mut t = t.clone();
    while let Ty::Infer(v) = t {
        match s.get(&v) {
            Some(x) => t = x.clone(),
            None => break,
        }
    }
    t
}

fn resolve(t: &Ty, s: &HashMap<u32, Ty>) -> Ty {
    let t = shallow(t, s);
    let r = |x: &Ty| resolve(x, s);
    match &t {
        Ty::Tuple(ts) => Ty::Tuple(ts.iter().map(r).collect()),
        Ty::Adt(d, args) => Ty::Adt(*d, args.iter().map(r).collect()),
        Ty::Array(e, k) => Ty::Array(Box::new(r(e)), *k),
        Ty::Grow(e) => Ty::Grow(Box::new(r(e))),
        Ty::Ptr(e, m) => Ty::Ptr(Box::new(r(e)), *m),
        Ty::FnPtr(ps, ret) => Ty::FnPtr(ps.iter().map(|(m, t)| (*m, r(t))).collect(), Box::new(r(ret))),
        _ => t,
    }
}

fn has_infer(t: &Ty) -> bool {
    let mut any = false;
    t.walk(&mut |x| any |= matches!(x, Ty::Infer(_)));
    any
}

/// Syntactic unification with occurs check, binding `Ty::Infer` variables.
fn unify(a: &Ty, b: &Ty, s: &mut HashMap<u32, Ty>) -> bool {
    let (a, b) = (shallow(a, s), shallow(b, s));
    if a == b {
        return true;
    }
    match (&a, &b) {
        (Ty::Infer(v), t) | (t, Ty::Infer(v)) => {
            let t = resolve(t, s);
            let mut occurs = false;
            t.walk(&mut |x| occurs |= *x == Ty::Infer(*v));
            if occurs {
                return false;
            }
            s.insert(*v, t);
            true
        }
        (Ty::Error, _) | (_, Ty::Error) => true,
        (Ty::Tuple(x), Ty::Tuple(y)) => x.len() == y.len() && x.iter().zip(y).all(|(x, y)| unify(x, y, s)),
        (Ty::Adt(d1, x), Ty::Adt(d2, y)) => d1 == d2 && x.len() == y.len() && x.iter().zip(y).all(|(x, y)| unify(x, y, s)),
        (Ty::Array(x, k1), Ty::Array(y, k2)) => k1 == k2 && unify(x, y, s),
        (Ty::Grow(x), Ty::Grow(y)) => unify(x, y, s),
        (Ty::Ptr(x, m1), Ty::Ptr(y, m2)) => m1 == m2 && unify(x, y, s),
        (Ty::FnPtr(p1, r1), Ty::FnPtr(p2, r2)) => {
            p1.len() == p2.len() && p1.iter().zip(p2).all(|((m1, x), (m2, y))| m1 == m2 && unify(x, y, s)) && unify(r1, r2, s)
        }
        _ => false,
    }
}
