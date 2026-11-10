//! Phase 2: lower item signatures (generics, bounds, fields, function
//! signatures, impl headers, constants) from the AST, then validate them.

use crate::defs::*;
use crate::resolve::ModRes;
use crate::ty::{ArrKind, IntTy, TraitRef, Ty};
use crate::{AstRef, Sema};
use ln_ast as ast;
use ln_diag::{Diagnostic, Span, codes::lint, codes::name, codes::syntax, codes::trait_, codes::ty as tyc};

/// A check that needs complete ADT information.
/// Largest Dull value (in bytes) for which a borrow is reported as needless.
const NEEDLESS_BORROW_MAX: u64 = 16;

pub(crate) struct Deferred {
    pub kind: DeferKind,
    pub ty: Ty,
    pub env: Vec<Pred>,
    pub span: Span,
    /// Span of the generic parameter list, for "add Dull to T's bounds" help.
    pub generics_span: Span,
}

pub(crate) enum DeferKind {
    ArrayElem,
    Grow,
    /// `x: &T` where `T` might be Dull (a pointless borrow).
    NeedlessBorrow,
}

/// Where a type appears.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TyPos {
    /// Directly a function parameter type: `!T` and `FmtArgs` allowed.
    Param,
    Other,
}

/// Context for lowering types inside one item.
#[derive(Clone)]
pub(crate) struct LowerCx {
    pub module: DefId,
    pub generic_names: Vec<String>,
    /// Where each of `generic_names` is declared (dummy for a trait's `Self`).
    pub generic_spans: Vec<Span>,
    pub self_ty: Option<Ty>,
    pub env: Vec<Pred>,
    pub allow_infer: bool,
    pub generics_span: Span,
    /// Lowering this function's return type, where `impl Trait` is allowed.
    pub opaque_owner: Option<DefId>,
}

impl LowerCx {
    pub fn new(module: DefId) -> Self {
        Self {
            module,
            generic_names: Vec::new(),
            generic_spans: Vec::new(),
            self_ty: None,
            env: Vec::new(),
            allow_infer: false,
            generics_span: Span::DUMMY,
            opaque_owner: None,
        }
    }
}

impl<'a> Sema<'a> {
    pub(crate) fn ast(&self, id: DefId) -> AstRef<'a> {
        self.asts[id.0 as usize]
    }

    fn def_count(&self) -> usize {
        self.p.defs.len()
    }

    /// Generic parameter names visible in an item, parents first.
    pub(crate) fn generic_names_of(&self, id: DefId) -> Vec<String> {
        self.generic_params_of(id).into_iter().map(|(n, _)| n).collect()
    }

    /// Generic parameters in scope for an item (parents' first) and where
    /// each is declared. A trait's implicit `Self` has a dummy span.
    pub(crate) fn generic_params_of(&self, id: DefId) -> Vec<(String, Span)> {
        let d = &self.p.defs[id.0 as usize];
        let of = |g: &ast::Generics| g.params.iter().map(|p| (p.name.name.clone(), p.name.span)).collect::<Vec<_>>();
        let own = match self.ast(id) {
            AstRef::Fn(f, _) => of(&f.generics),
            AstRef::Struct(s) => of(&s.generics),
            AstRef::Enum(e) => of(&e.generics),
            AstRef::Trait(t) => {
                let mut v = vec![("Self".to_string(), Span::DUMMY)];
                v.extend(of(&t.generics));
                v
            }
            AstRef::Impl(i) => of(&i.generics),
            AstRef::Alias(a) => of(&a.generics),
            _ => Vec::new(),
        };
        match (d.kind, d.parent) {
            (DefKind::Fn, Some(parent)) => {
                let mut v = self.generic_params_of(parent);
                v.extend(own);
                v
            }
            _ => own,
        }
    }

    /// Predicates in scope for an item (own and parents').
    pub(crate) fn env_of(&self, id: DefId) -> Vec<Pred> {
        let d = &self.p.defs[id.0 as usize];
        let mut out = Vec::new();
        if let (DefKind::Fn, Some(parent)) = (d.kind, d.parent) {
            out.extend(self.env_of(parent));
            if self.p.defs[parent.0 as usize].kind == DefKind::Trait {
                // Inside a trait, `Self: Trait` holds.
                let t = self.p.trait_data(parent);
                let args: Vec<Ty> = (0..t.generics.names.len()).map(|i| Ty::Param(i as u32 + 1, t.generics.names[i].clone())).collect();
                out.push(Pred { ty: Ty::Param(0, "Self".into()), trait_ref: TraitRef { trait_id: parent, args }, span: d.span });
            }
        }
        if let Some(g) = self.generics(id) {
            out.extend(g.preds.iter().cloned());
        }
        out
    }

    pub(crate) fn generics(&self, id: DefId) -> Option<&Generics> {
        match &self.p.defs[id.0 as usize].data {
            DefData::Fn(f) => Some(&f.generics),
            DefData::Adt(a) => Some(&a.generics),
            DefData::Trait(t) => Some(&t.generics),
            DefData::Impl(i) => Some(&i.generics),
            DefData::Alias(a) => Some(&a.generics),
            _ => None,
        }
    }

    /// The self type inside an impl or trait (or for a method's parent).
    pub(crate) fn self_ty_of(&self, id: DefId) -> Option<Ty> {
        let d = &self.p.defs[id.0 as usize];
        match d.kind {
            DefKind::Impl => match &d.data {
                DefData::Impl(i) => Some(i.self_ty.clone()),
                _ => None,
            },
            DefKind::Trait => Some(Ty::Param(0, "Self".into())),
            DefKind::Fn => d.parent.and_then(|p| self.self_ty_of(p)),
            DefKind::Struct | DefKind::Enum => {
                let names = self.generic_names_of(id);
                Some(Ty::Adt(id, names.iter().enumerate().map(|(i, n)| Ty::Param(i as u32, n.clone())).collect()))
            }
            _ => None,
        }
    }

    pub(crate) fn lower_cx(&self, id: DefId) -> LowerCx {
        let d = &self.p.defs[id.0 as usize];
        let gspan = match self.ast(id) {
            AstRef::Fn(f, _) => {
                if f.generics.params.is_empty() {
                    f.name.span.shrink_to_hi()
                } else {
                    f.generics.span
                }
            }
            AstRef::Struct(s) => s.generics.span,
            AstRef::Enum(e) => e.generics.span,
            AstRef::Impl(i) => i.generics.span,
            _ => d.span,
        };
        let params = self.generic_params_of(id);
        LowerCx {
            module: d.module,
            generic_names: params.iter().map(|(n, _)| n.clone()).collect(),
            generic_spans: params.iter().map(|(_, s)| *s).collect(),
            self_ty: self.self_ty_of(id),
            env: self.env_of(id),
            allow_infer: false,
            generics_span: gspan,
            opaque_owner: None,
        }
    }

    // ===================================================================== driver

    pub(crate) fn lower_all(&mut self) {
        let n = self.def_count();
        // 1. Generic parameter names.
        for i in 0..n {
            let id = DefId(i as u32);
            let (own, parent_count, span) = match self.ast(id) {
                AstRef::Fn(f, _) => {
                    let pc = self.p.defs[i].parent.map_or(0, |p| self.generic_names_of(p).len());
                    (f.generics.params.iter().map(|p| p.name.name.clone()).collect::<Vec<_>>(), pc, f.generics.span)
                }
                AstRef::Struct(s) => (s.generics.params.iter().map(|p| p.name.name.clone()).collect(), 0, s.generics.span),
                AstRef::Enum(e) => (e.generics.params.iter().map(|p| p.name.name.clone()).collect(), 0, e.generics.span),
                AstRef::Trait(t) => (t.generics.params.iter().map(|p| p.name.name.clone()).collect(), 1, t.generics.span),
                AstRef::Impl(im) => (im.generics.params.iter().map(|p| p.name.name.clone()).collect(), 0, im.generics.span),
                AstRef::Alias(a) => (a.generics.params.iter().map(|p| p.name.name.clone()).collect(), 0, a.generics.span),
                _ => continue,
            };
            let g = Generics { parent_count: parent_count as u32, names: own, preds: Vec::new(), span };
            self.check_dup_generics(id);
            let kind = self.p.defs[i].kind;
            match &mut self.p.defs[i].data {
                DefData::Adt(a) => a.generics = g,
                DefData::Trait(t) => t.generics = g,
                d @ DefData::None => {
                    *d = match kind {
                        DefKind::Fn => DefData::Fn(Box::new(FnData {
                            generics: g,
                            sig: FnSig { params: Vec::new(), ret: Ty::unit(), ret_span: Span::DUMMY },
                            is_unsafe: false,
                            abi: Abi::Lunar,
                            variadic: false,
                            has_body: false,
                            owner: None,
                            has_self: false,
                        })),
                        DefKind::Struct => DefData::Adt(Box::new(AdtData { generics: g, variants: Vec::new(), is_enum: false })),
                        DefKind::Impl => {
                            DefData::Impl(Box::new(ImplData { generics: g, trait_ref: None, self_ty: Ty::Error, methods: Default::default(), negative: false }))
                        }
                        DefKind::TypeAlias => DefData::Alias(AliasData { generics: g, ty: None }),
                        _ => DefData::None,
                    }
                }
                _ => {}
            }
        }
        for lang in ["dull", "light"] {
            if let Some(d) = self.p.lang_item(lang)
                && let DefData::Trait(t) = &mut self.p.defs[d.0 as usize].data
            {
                t.auto = true;
            }
        }
        // 2. Bounds and supertraits.
        for i in 0..n {
            self.lower_preds(DefId(i as u32));
        }
        // 3. ADT fields.
        for i in 0..n {
            let id = DefId(i as u32);
            match self.ast(id) {
                AstRef::Struct(s) => self.lower_struct(id, s),
                AstRef::Enum(e) => self.lower_enum(id, e),
                _ => {}
            }
        }
        // 4. Impl headers first (method signatures may mention `Self`).
        for i in 0..n {
            if let AstRef::Impl(im) = self.ast(DefId(i as u32)) {
                self.lower_impl_header(DefId(i as u32), im);
            }
        }
        for i in 0..n {
            let id = DefId(i as u32);
            match self.ast(id) {
                AstRef::Fn(f, ext) => self.lower_fn(id, f, ext),
                AstRef::Alias(_) => {
                    let _ = self.alias_ty(id);
                }
                _ => {}
            }
        }
        for i in 0..n {
            let id = DefId(i as u32);
            match self.ast(id) {
                AstRef::Const(c) => self.lower_const(id, c),
                AstRef::Static(s) => self.lower_static(id, s),
                _ => {}
            }
        }
        // 5. Deferred checks.
        self.run_deferred();
        self.check_infinite_size();
        // 6. Impl validation.
        for i in 0..n {
            if let AstRef::Impl(im) = self.ast(DefId(i as u32)) {
                self.check_impl(DefId(i as u32), im);
            }
        }
        self.check_overlapping_impls();
        self.check_main();
    }

    fn check_dup_generics(&mut self, id: DefId) {
        let params: &[ast::GenericParam] = match self.ast(id) {
            AstRef::Fn(f, _) => &f.generics.params,
            AstRef::Struct(s) => &s.generics.params,
            AstRef::Enum(e) => &e.generics.params,
            AstRef::Trait(t) => &t.generics.params,
            AstRef::Impl(i) => &i.generics.params,
            _ => return,
        };
        let mut seen: Vec<&str> = Vec::new();
        let mut errs = Vec::new();
        for p in params {
            if seen.contains(&p.name.name.as_str()) || p.name.name == "Self" {
                errs.push(Diagnostic::error(name::DUPLICATE, p.name.span, format!("generic parameter `{}` is declared twice or shadows `Self`", p.name.name)));
            }
            seen.push(&p.name.name);
        }
        for e in errs {
            self.err(e);
        }
    }

    fn lower_preds(&mut self, id: DefId) {
        let (params, supers): (&[ast::GenericParam], &[ast::Path]) = match self.ast(id) {
            AstRef::Fn(f, _) => (&f.generics.params, &[]),
            AstRef::Struct(s) => (&s.generics.params, &[]),
            AstRef::Enum(e) => (&e.generics.params, &[]),
            AstRef::Trait(t) => (&t.generics.params, &t.supertraits),
            AstRef::Impl(i) => (&i.generics.params, &[]),
            _ => return,
        };
        let cx = self.lower_cx(id);
        let offset = match &self.p.defs[id.0 as usize].data {
            DefData::Fn(f) => f.generics.parent_count,
            DefData::Trait(_) => 1,
            _ => 0,
        };
        let mut preds = Vec::new();
        for p in params {
            self.p.refs.push(crate::Ref { span: p.name.span, target: crate::RefTarget::TyParam(p.name.span) });
        }
        for (i, p) in params.iter().enumerate() {
            for b in &p.bounds {
                if let Some(tr) = self.lower_trait_ref(&cx, b) {
                    let idx = offset + i as u32;
                    preds.push(Pred { ty: Ty::Param(idx, p.name.name.clone()), trait_ref: tr, span: b.span });
                }
            }
        }
        let mut sts = Vec::new();
        for s in supers {
            if let Some(tr) = self.lower_trait_ref(&cx, s) {
                sts.push(tr);
            }
        }
        match &mut self.p.defs[id.0 as usize].data {
            DefData::Fn(f) => f.generics.preds = preds,
            DefData::Adt(a) => a.generics.preds = preds,
            DefData::Trait(t) => {
                t.generics.preds = preds;
                t.supertraits = sts;
            }
            DefData::Impl(i) => i.generics.preds = preds,
            _ => {}
        }
    }

    /// `impl A + B` in a function's return type: a new opaque type with
    /// those bounds, standing for whatever the function's body returns.
    fn lower_impl_trait(&mut self, cx: &LowerCx, bounds: &[ast::Path], span: Span) -> Ty {
        let Some(owner) = cx.opaque_owner else {
            self.err(
                Diagnostic::error(tyc::IMPL_TRAIT_POSITION, span, "`impl Trait` is only allowed as the return type of a function")
                    .with_help("to accept any type with these bounds, add a generic parameter: `fn f::<T: Trait>(x: T)`"),
            );
            return Ty::Error;
        };
        let mut inner = cx.clone();
        inner.opaque_owner = None;
        let trait_refs: Vec<TraitRef> = bounds.iter().filter_map(|b| self.lower_trait_ref(&inner, b)).collect();
        let idx = self.p.opaques.keys().filter(|(d, _)| *d == owner).count() as u32;
        self.p.opaques.insert((owner, idx), crate::OpaqueInfo { bounds: trait_refs, concrete: None, span });
        let args = cx.generic_names.iter().enumerate().map(|(i, n)| Ty::Param(i as u32, n.clone())).collect();
        Ty::Opaque(owner, idx, args)
    }

    pub(crate) fn lower_trait_ref(&mut self, cx: &LowerCx, path: &ast::Path) -> Option<TraitRef> {
        let idents: Vec<ast::Ident> = path.segments.iter().map(|s| s.ident.clone()).collect();
        let res = self.resolve_prefix(cx.module, &idents, Ns::Type);
        let Some((ModRes::Def(def), used)) = res else {
            if res.is_none() && idents.len() == 1 {
                let d = self.unresolved_diag(cx.module, &idents[0], Ns::Type);
                self.err(d);
            }
            return None;
        };
        if used != idents.len() || self.p.defs[def.0 as usize].kind != DefKind::Trait {
            let d = &self.p.defs[def.0 as usize];
            self.err(Diagnostic::error(name::NOT_A_TRAIT, path.span, format!("expected a trait, found {} `{}`", d.kind.describe(), d.name)));
            return None;
        }
        let args: Vec<Ty> = match &path.segments[used - 1].args {
            Some(a) => a.iter().map(|t| self.lower_ty(cx, t, TyPos::Other)).collect(),
            None => Vec::new(),
        };
        let want = match &self.p.defs[def.0 as usize].data {
            DefData::Trait(t) => t.generics.names.len(),
            _ => 0,
        };
        if args.len() != want {
            self.err(Diagnostic::error(
                tyc::GENERIC_COUNT,
                path.span,
                format!("trait `{}` takes {want} generic argument(s) but {} were supplied", self.p.defs[def.0 as usize].name, args.len()),
            ));
            return None;
        }
        Some(TraitRef { trait_id: def, args })
    }

    fn lower_fields(&mut self, cx: &LowerCx, data: &ast::VariantData) -> (CtorKind, Vec<FieldDef>) {
        match data {
            ast::VariantData::Unit => (CtorKind::Unit, Vec::new()),
            ast::VariantData::Tuple(fs) => (
                CtorKind::Tuple,
                fs.iter()
                    .enumerate()
                    .map(|(i, f)| FieldDef { name: i.to_string(), ty: self.lower_ty(cx, &f.ty, TyPos::Other), is_pub: f.is_pub, span: f.ty.span })
                    .collect(),
            ),
            ast::VariantData::Named(fs) => {
                let mut out: Vec<FieldDef> = Vec::new();
                for f in fs {
                    if out.iter().any(|x| x.name == f.name.name) {
                        self.err(Diagnostic::error(name::DUPLICATE, f.name.span, format!("field `{}` is declared twice", f.name.name)));
                    }
                    let ty = self.lower_ty(cx, &f.ty, TyPos::Other);
                    out.push(FieldDef { name: f.name.name.clone(), ty, is_pub: f.is_pub, span: f.name.span });
                }
                (CtorKind::Named, out)
            }
        }
    }

    fn lower_struct(&mut self, id: DefId, s: &ast::StructDecl) {
        let cx = self.lower_cx(id);
        let (ctor, fields) = self.lower_fields(&cx, &s.kind);
        if let DefData::Adt(a) = &mut self.p.defs[id.0 as usize].data {
            a.variants = vec![VariantDef { name: s.name.name.clone(), def: id, ctor, fields, span: s.name.span }];
        }
    }

    fn lower_enum(&mut self, id: DefId, e: &ast::EnumDecl) {
        let cx = self.lower_cx(id);
        let mut lowered = Vec::new();
        for v in &e.variants {
            lowered.push(self.lower_fields(&cx, &v.data));
        }
        if let DefData::Adt(a) = &mut self.p.defs[id.0 as usize].data {
            for (v, (ctor, fields)) in a.variants.iter_mut().zip(lowered) {
                v.ctor = ctor;
                v.fields = fields;
            }
        }
    }

    fn lower_impl_header(&mut self, id: DefId, im: &ast::ImplDecl) {
        let mut cx = self.lower_cx(id);
        cx.self_ty = None;
        let self_ty = self.lower_ty(&cx, &im.self_ty, TyPos::Other);
        cx.self_ty = Some(self_ty.clone());
        let trait_ref = im.trait_ref.as_ref().and_then(|p| self.lower_trait_ref(&cx, p));
        let mut methods = std::collections::HashMap::new();
        let items = self.impl_items.get(&id).cloned().unwrap_or_default();
        for m in items {
            let name = self.p.defs[m.0 as usize].name.clone();
            if methods.insert(name.clone(), m).is_some() {
                let sp = self.p.defs[m.0 as usize].span;
                self.err(Diagnostic::error(name::DUPLICATE, sp, format!("method `{name}` is defined twice in this impl")));
            }
        }
        if let DefData::Impl(i) = &mut self.p.defs[id.0 as usize].data {
            i.self_ty = self_ty;
            i.trait_ref = trait_ref;
            i.negative = im.negative;
            i.methods = methods;
        }
    }

    fn lower_fn(&mut self, id: DefId, f: &ast::FnDecl, is_extern: bool) {
        let cx = self.lower_cx(id);
        let parent = self.p.defs[id.0 as usize].parent;
        let mut params = Vec::new();
        let mut has_self = false;
        for (i, p) in f.params.iter().enumerate() {
            let (pname, is_self) = match &p.kind {
                ast::ParamKind::SelfParam => {
                    if i != 0 || parent.is_none() {
                        self.err(Diagnostic::error(name::BAD_SELF, p.span, "`self` is only allowed as the first parameter of a method"));
                    }
                    has_self = true;
                    ("self".to_string(), true)
                }
                ast::ParamKind::Named(n) => (n.name.clone(), false),
                ast::ParamKind::Wild => ("_".to_string(), false),
            };
            let ty = match &p.ty {
                Some(t) => self.lower_ty(&cx, t, TyPos::Param),
                None => {
                    let t = cx.self_ty.clone().unwrap_or(Ty::Error);
                    if p.self_grow { Ty::Grow(Box::new(t)) } else { t }
                }
            };
            if is_extern {
                self.check_ffi_ty(&ty, p.span);
            }
            if matches!(ty, Ty::Grow(_)) && p.mode != ast::ParamMode::InOut {
                let fix = if is_self { "&mut +self".to_string() } else { format!("{pname}: &mut {}", self.p.ty_str(&ty)) };
                self.err(
                    Diagnostic::error(tyc::GROW_WITHOUT_MUT, p.span, format!("`+` parameter `{}` is not `&mut`", if is_self { "self" } else { &pname }))
                        .with_label("a parameter that is not in-out cannot change the caller's value at all")
                        .with_note("`+` promises that an in-out argument only grows")
                        .with_help(format!("declare it `{fix}`, or drop the `+`")),
                );
            }
            // `needless_borrow` deliberately skips:
            // * `&self`: methods conventionally borrow their receiver whatever
            //   its type, so a type's methods read alike and the receiver can
            //   later grow an owned field without changing every signature;
            // * trait methods and their impls, which must keep the trait's
            //   modes even when the implementing type is Dull;
            // * extern functions, whose signatures follow C.
            let in_trait_item = parent.is_some_and(|pd| match &self.p.defs[pd.0 as usize].data {
                DefData::Impl(d) => d.trait_ref.is_some(),
                DefData::Trait(_) => true,
                _ => false,
            });
            if p.mode == ast::ParamMode::Borrow && !is_self && !is_extern && !in_trait_item {
                self.deferred.push(Deferred {
                    kind: DeferKind::NeedlessBorrow,
                    ty: ty.clone(),
                    env: cx.env.clone(),
                    span: p.span,
                    generics_span: f.generics.span,
                });
            }
            let ctx = match &p.kind {
                ast::ParamKind::Named(n) => n.ctx,
                _ => ast::SyntaxCtx::ROOT,
            };
            params.push(Param { name: pname, ctx, ty, mode: p.mode, inout: p.mode == ast::ParamMode::InOut, mutable: p.is_mut, is_self, kept: p.kept, span: p.span });
        }
        // `impl Trait` is allowed in the return type of functions with a
        // body that are not part of a trait (a trait's implementations
        // could not agree on one type).
        let in_trait_item = parent.is_some_and(|pd| match &self.p.defs[pd.0 as usize].data {
            DefData::Impl(d) => d.trait_ref.is_some(),
            DefData::Trait(_) => true,
            _ => false,
        });
        let mut rcx = cx.clone();
        if f.body.is_some() && !in_trait_item {
            rcx.opaque_owner = Some(id);
        }
        let (ret, ret_span) = match &f.ret {
            Some(t) => (self.lower_ty(&rcx, t, TyPos::Other), t.span),
            None => (Ty::unit(), f.sig_span.shrink_to_hi()),
        };
        if is_extern {
            self.check_ffi_ty(&ret, ret_span);
        }
        let attrs = self.p.defs[id.0 as usize].attrs.clone();
        if f.body.is_none() && !is_extern && !attrs.intrinsic {
            let in_trait = parent.is_some_and(|p| self.p.defs[p.0 as usize].kind == DefKind::Trait);
            if !in_trait {
                self.err(
                    Diagnostic::error(syntax::MISSING_BODY, f.sig_span, format!("function `{}` has no body", f.name.name))
                        .with_label("expected `{ ... }` after this signature")
                        .with_help("add a body: `{ ... }`; only trait methods and functions in `extern` blocks are declared without one"),
                );
            }
        }
        if f.variadic && !is_extern {
            self.err(
                Diagnostic::error(tyc::VARIADIC_NOT_EXTERN, f.sig_span, format!("function `{}` is variadic but not an extern function", f.name.name))
                    .with_note("only functions declared in an `extern \"C\" { ... }` block can take `...`")
                    .with_help("take the extra arguments as a slice instead, e.g. `args: [i32; &]`"),
            );
        }
        if let DefData::Fn(fd) = &mut self.p.defs[id.0 as usize].data {
            fd.sig = FnSig { params, ret, ret_span };
            fd.is_unsafe = f.is_unsafe || is_extern;
            fd.abi = if is_extern { Abi::C } else { Abi::Lunar };
            fd.variadic = f.variadic;
            fd.has_body = f.body.is_some();
            fd.owner = parent;
            fd.has_self = has_self;
        }
    }

    fn check_ffi_ty(&mut self, ty: &Ty, span: Span) {
        let ok = matches!(ty, Ty::Int(_) | Ty::Float(_) | Ty::Ptr(..) | Ty::Bool | Ty::Never | Ty::FnPtr(..)) || ty.is_unit();
        if !ok && !ty.is_error() {
            let s = self.p.ty_str(ty);
            self.err(
                Diagnostic::error(tyc::FFI_TYPE, span, format!("type `{s}` cannot cross the C ABI"))
                    .with_note("extern functions may only use integers, floats, bool, raw pointers and fn pointers"),
            );
        }
    }

    fn lower_const(&mut self, id: DefId, c: &ast::ConstDecl) {
        let cx = LowerCx::new(self.p.defs[id.0 as usize].module);
        let ty = self.lower_ty(&cx, &c.ty, TyPos::Other);
        let value = self.eval_const(cx.module, &c.value, &ty);
        self.p.defs[id.0 as usize].data = DefData::Const(ConstData { ty, value });
    }

    fn lower_static(&mut self, id: DefId, s: &ast::StaticDecl) {
        let cx = LowerCx::new(self.p.defs[id.0 as usize].module);
        let ty = self.lower_ty(&cx, &s.ty, TyPos::Other);
        // `FmtArgs` is already rejected outside parameters by `lower_ty`.
        if self.p.has_owner(&ty) {
            let t = self.p.ty_str(&ty);
            let what = if matches!(ty, Ty::String | Ty::Array(..)) { "is an owner" } else { "contains an owning array or `String`" };
            self.err(
                Diagnostic::error(tyc::CONST_EVAL, s.ty.span, format!("a static cannot own data, but `{t}` {what}"))
                    .with_note("statics are not on any data stack, so they cannot hold an owning array, a `String`, or a type containing one"),
            );
        }
        let value = self.eval_const(cx.module, &s.value, &ty);
        self.p.defs[id.0 as usize].data = DefData::Static(StaticData { ty, mutable: s.is_mut, value });
    }

    // ===================================================================== types

    pub(crate) fn lower_ty(&mut self, cx: &LowerCx, t: &ast::Ty, pos: TyPos) -> Ty {
        let ty = match &t.kind {
            ast::TyKind::Path(p) => self.lower_path_ty(cx, p),
            ast::TyKind::MacCall(m) => {
                return match self.expand_ty(cx.module, m) {
                    Some(t) => self.lower_ty(cx, t, pos),
                    None => Ty::Error,
                };
            }
            ast::TyKind::Infer => {
                if !cx.allow_infer {
                    self.err(Diagnostic::error(tyc::ANNOTATIONS_NEEDED, t.span, "`_` is not allowed in item signatures"));
                    return Ty::Error;
                }
                return Ty::Infer(u32::MAX);
            }
            ast::TyKind::Never => Ty::Never,
            ast::TyKind::ImplTrait(bounds) => return self.lower_impl_trait(cx, bounds, t.span),
            ast::TyKind::Tuple(ts) => Ty::Tuple(ts.iter().map(|t| self.lower_ty(cx, t, TyPos::Other)).collect()),
            ast::TyKind::Ptr { is_mut, inner } => Ty::Ptr(Box::new(self.lower_ty(cx, inner, TyPos::Other)), *is_mut),
            ast::TyKind::Fn { params, ret } => {
                let ps = params.iter().map(|(m, t)| (*m, self.lower_ty(cx, t, TyPos::Param))).collect();
                let r = ret.as_ref().map_or(Ty::unit(), |r| self.lower_ty(cx, r, TyPos::Other));
                Ty::FnPtr(ps, Box::new(r))
            }
            ast::TyKind::Grow(inner) => {
                let inner_ty = self.lower_ty(cx, inner, TyPos::Other);
                if pos != TyPos::Param {
                    self.err(
                        Diagnostic::error(tyc::GROW_OUTSIDE_PARAM, t.span, "`+` types are only valid as function parameter types")
                            .with_note("`&mut +T` marks a parameter that only grows; it has no meaning elsewhere"),
                    );
                    return inner_ty;
                }
                self.deferred.push(Deferred {
                    kind: DeferKind::Grow,
                    ty: inner_ty.clone(),
                    env: cx.env.clone(),
                    span: t.span,
                    generics_span: cx.generics_span,
                });
                Ty::Grow(Box::new(inner_ty))
            }
            ast::TyKind::DynArray(elem) => {
                let e = self.lower_ty(cx, elem, TyPos::Other);
                self.defer_elem(cx, &e, elem.span);
                Ty::Array(Box::new(e), ArrKind::Dyn)
            }
            ast::TyKind::Array(elem, size) => {
                let e = self.lower_ty(cx, elem, TyPos::Other);
                self.defer_elem(cx, &e, elem.span);
                let kind = match size {
                    ast::ArraySize::Fixed(expr) => match self.eval_const(cx.module, expr, &Ty::usize()) {
                        Some(ConstVal::Int(n)) if n >= 0 => ArrKind::Fixed(n as u64),
                        Some(_) => {
                            self.err(Diagnostic::error(tyc::ARRAY_LEN, expr.span, "array length must be a non-negative integer"));
                            ArrKind::Fixed(0)
                        }
                        None => ArrKind::Fixed(0),
                    },
                    ast::ArraySize::Inferred => {
                        if !cx.allow_infer {
                            self.err(Diagnostic::error(tyc::ANNOTATIONS_NEEDED, t.span, "inferred array lengths `[T; _]` are not allowed in item signatures"));
                            ArrKind::Fixed(0)
                        } else {
                            ArrKind::FixedVar(u32::MAX)
                        }
                    }
                    ast::ArraySize::Grow => ArrKind::Grow,
                    ast::ArraySize::GrowShrink => ArrKind::Shrink,
                    ast::ArraySize::Slice => ArrKind::Slice,
                    ast::ArraySize::SliceMut => ArrKind::SliceMut,
                };
                Ty::Array(Box::new(e), kind)
            }
        };
        if matches!(ty, Ty::FmtArgs) && pos != TyPos::Param {
            self.err(
                Diagnostic::error(tyc::FMTARGS_OUTSIDE_PARAM, t.span, "`FmtArgs` is only valid as a function parameter type")
                    .with_note("a string literal converts to `FmtArgs` when passed to such a parameter"),
            );
        }
        ty
    }

    fn defer_elem(&mut self, cx: &LowerCx, elem: &Ty, span: Span) {
        self.deferred.push(Deferred { kind: DeferKind::ArrayElem, ty: elem.clone(), env: cx.env.clone(), span, generics_span: cx.generics_span });
    }

    fn assoc_types_unsupported(span: Span) -> Diagnostic {
        Diagnostic::error(name::NOT_A_TYPE, span, "associated types are not supported")
            .with_note("traits in Lunar have methods and associated functions, but no associated types")
            .with_help("make the type a generic parameter of the trait instead, as in `Iterator::<T>`")
    }

    fn lower_path_ty(&mut self, cx: &LowerCx, p: &ast::Path) -> Ty {
        // Generic parameters and `Self`.
        if p.segments.len() == 1 {
            let seg = &p.segments[0];
            if seg.ident.name == "Self" {
                return match &cx.self_ty {
                    Some(t) => t.clone(),
                    None => {
                        self.err(Diagnostic::error(name::UNRESOLVED, seg.ident.span, "`Self` is only available inside impls and traits"));
                        Ty::Error
                    }
                };
            }
            if let Some(i) = cx.generic_names.iter().rposition(|n| *n == seg.ident.name) {
                if seg.args.is_some() {
                    self.err(Diagnostic::error(tyc::GENERIC_COUNT, p.span, "generic parameters cannot take generic arguments"));
                }
                if let Some(&decl) = cx.generic_spans.get(i)
                    && !decl.is_dummy()
                {
                    self.p.refs.push(crate::Ref { span: seg.ident.span, target: crate::RefTarget::TyParam(decl) });
                }
                return Ty::Param(i as u32, seg.ident.name.clone());
            }
        }
        // `Self::Item`, `T::Item`: a path into a type, not a module.
        if p.segments.len() > 1 {
            let first = &p.segments[0].ident.name;
            if first == "Self" && cx.self_ty.is_none() {
                self.err(Diagnostic::error(name::UNRESOLVED, p.segments[0].ident.span, "`Self` is only available inside impls and traits"));
                return Ty::Error;
            }
            if first == "Self" || cx.generic_names.iter().any(|n| n == first) {
                self.err(Self::assoc_types_unsupported(p.span));
                return Ty::Error;
            }
        }
        let idents: Vec<ast::Ident> = p.segments.iter().map(|s| s.ident.clone()).collect();
        let res = self.resolve_prefix(cx.module, &idents, Ns::Type);
        let (res, used) = match res {
            Some(r) => r,
            None => {
                if idents.len() == 1 {
                    let d = self.unresolved_diag(cx.module, &idents[0], Ns::Type);
                    self.err(d);
                }
                return Ty::Error;
            }
        };
        if used != idents.len() {
            let d = match res {
                ModRes::Def(def) if !matches!(self.p.defs[def.0 as usize].kind, DefKind::Struct | DefKind::Enum | DefKind::TypeAlias | DefKind::Trait) => {
                    let d = &self.p.defs[def.0 as usize];
                    let prefix = idents[..used].iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join("::");
                    Diagnostic::error(name::NOT_A_TYPE, idents[used - 1].span, format!("expected a type or module, found {} `{prefix}`", d.kind.describe()))
                }
                _ => Self::assoc_types_unsupported(p.span),
            };
            self.err(d);
            return Ty::Error;
        }
        let seg_args = &p.segments[used - 1].args;
        match res {
            ModRes::Prim(t) => {
                if seg_args.is_some() {
                    self.err(Diagnostic::error(tyc::GENERIC_COUNT, p.span, "primitive types take no generic arguments"));
                }
                t
            }
            ModRes::Def(def) => {
                let kind = self.p.defs[def.0 as usize].kind;
                match kind {
                    DefKind::Struct | DefKind::Enum | DefKind::TypeAlias => {
                        let want = self.generic_names_of(def).len();
                        let args: Vec<Ty> = match seg_args {
                            Some(a) => a.iter().map(|t| self.lower_ty(cx, t, TyPos::Other)).collect(),
                            None if want > 0 && cx.allow_infer => vec![Ty::Infer(u32::MAX); want],
                            None => Vec::new(),
                        };
                        if args.len() != want {
                            let name = self.p.defs[def.0 as usize].name.clone();
                            self.err(
                                Diagnostic::error(
                                    tyc::GENERIC_COUNT,
                                    p.span,
                                    format!("`{name}` takes {want} generic argument(s) but {} were supplied", args.len()),
                                )
                                .with_help(format!("write `{name}::<...>`")),
                            );
                            return Ty::Error;
                        }
                        if kind == DefKind::TypeAlias {
                            return match self.alias_ty(def) {
                                Some(t) => t.subst(&args),
                                None => Ty::Error,
                            };
                        }
                        Ty::Adt(def, args)
                    }
                    _ => {
                        let d = &self.p.defs[def.0 as usize];
                        let msg = format!("expected a type, found {} `{}`", d.kind.describe(), d.name);
                        self.err(Diagnostic::error(name::NOT_A_TYPE, p.span, msg));
                        Ty::Error
                    }
                }
            }
        }
    }

    fn alias_ty(&mut self, id: DefId) -> Option<Ty> {
        if let DefData::Alias(a) = &self.p.defs[id.0 as usize].data
            && let Some(t) = &a.ty
        {
            return Some(t.clone());
        }
        let AstRef::Alias(a) = self.ast(id) else { return None };
        if self.alias_stack.contains(&id) {
            self.err(Diagnostic::error(tyc::INFINITE_SIZE, a.name.span, format!("type alias `{}` refers to itself", a.name.name)));
            return None;
        }
        self.alias_stack.push(id);
        let cx = self.lower_cx(id);
        let t = self.lower_ty(&cx, &a.ty, TyPos::Other);
        self.alias_stack.pop();
        if let DefData::Alias(ad) = &mut self.p.defs[id.0 as usize].data {
            ad.ty = Some(t.clone());
        }
        Some(t)
    }

    // ===================================================================== checks

    fn run_deferred(&mut self) {
        for d in std::mem::take(&mut self.deferred) {
            if d.ty.is_error() {
                continue;
            }
            match d.kind {
                DeferKind::ArrayElem => {
                    if !self.p.is_light(&d.ty, &d.env) {
                        let diag = self.pointy_diag(&d.ty, d.span, d.generics_span);
                        self.err(diag);
                    }
                }
                DeferKind::Grow => {
                    if !self.p.could_own(&d.ty, &d.env) {
                        let s = self.p.ty_str(&d.ty);
                        self.err(
                            Diagnostic::error(tyc::GROW_NO_OWNER, d.span, format!("`+{s}` is meaningless: `{s}` owns no array or string"))
                                .with_note("`+` promises an argument is never shrunk or replaced; only owners (`[T]`, `[T; ..+]`, `[T; -..+]`, `String`, or types containing them) can be"),
                        );
                    }
                }
                DeferKind::NeedlessBorrow => {
                    // Borrowing passes an address, which pays off for large
                    // values; for a small Dull value a copy is just as cheap.
                    let inner = d.ty.strip_grow();
                    let small = self.p.layout_of(inner).is_some_and(|l| l.size <= NEEDLESS_BORROW_MAX);
                    if small && self.p.is_dull(inner, &d.env) {
                        let s = self.p.ty_str(inner);
                        self.err(
                            Diagnostic::warning(
                                lint::NEEDLESS_BORROW,
                                d.span,
                                format!("borrowing `{s}` is pointless: it is small and Dull, so copying it is just as cheap"),
                            )
                            .with_help(format!("take it by value: `: {s}`")),
                        );
                    }
                }
            }
        }
    }

    /// The `err(type::array_owner_items)` diagnostic.
    pub(crate) fn pointy_diag(&self, elem: &Ty, span: Span, generics_span: Span) -> Diagnostic {
        let s = self.p.ty_str(elem);
        let mut d = Diagnostic::error(tyc::ARRAY_OWNER_ITEMS, span, "Arrays and slices cannot hold items that own arrays or strings.")
            .with_label(format!("`{s}` is not known to be `Light`"));
        match elem {
            Ty::Param(_, n) => {
                d = d.with_help(format!("add Light to {n}'s bounds: `{n}: Light`"));
                if !generics_span.is_dummy() {
                    d = d.with_secondary(generics_span, format!("`{n}` is declared here"));
                }
            }
            _ => {
                d = d.with_note(format!(
                    "`{s}` owns an array or string. Items may be views (`str`, slices) or Dull values, \
                     but not owners: store the owners elsewhere (an arena, or one big array) and keep views or handles here"
                ));
            }
        }
        d
    }

    fn check_infinite_size(&mut self) {
        for i in 0..self.p.defs.len() {
            let id = DefId(i as u32);
            if !matches!(self.p.defs[i].kind, DefKind::Struct | DefKind::Enum) {
                continue;
            }
            if self.adt_contains(id, id, &mut Vec::new()) {
                let d = &self.p.defs[i];
                let msg = format!("recursive type `{}` has infinite size", d.name);
                let sp = d.span;
                self.err(Diagnostic::error(tyc::INFINITE_SIZE, sp, msg).with_help("store the recursive part in an array-backed pool and refer to it by index"));
            }
        }
    }

    /// Whether ADT `cur` stores a `root` inside itself, by value.
    fn adt_contains(&self, root: DefId, cur: DefId, seen: &mut Vec<DefId>) -> bool {
        self.contains_by_value(root, &Ty::Adt(cur, Vec::new()), seen, true)
    }

    /// Whether a value of type `ty` stores a `root` inside itself. Only what
    /// is stored inline counts: fields (with the type's arguments
    /// substituted, so `Id::<Expr>` holding a `u32` does not contain an
    /// `Expr`), tuple elements and fixed-size array items. Owners, slices
    /// and pointers break the cycle.
    fn contains_by_value(&self, root: DefId, ty: &Ty, seen: &mut Vec<DefId>, top: bool) -> bool {
        match ty {
            Ty::Adt(d, args) => {
                if *d == root && !top {
                    return true;
                }
                if seen.contains(d) {
                    return false;
                }
                seen.push(*d);
                let DefData::Adt(a) = &self.p.defs[d.0 as usize].data else { return false };
                a.variants.iter().flat_map(|v| &v.fields).any(|f| self.contains_by_value(root, &f.ty.subst(args), seen, false))
            }
            Ty::Tuple(ts) => ts.iter().any(|t| self.contains_by_value(root, t, seen, false)),
            Ty::Array(e, k) if !(k.is_owner() || k.is_slice()) => self.contains_by_value(root, e, seen, false),
            Ty::Grow(t) => self.contains_by_value(root, t, seen, false),
            _ => false,
        }
    }

    /// Two impls of the same trait that can apply to the same type would make
    /// method resolution ambiguous: `err(trait::conflicting_impl)`.
    fn check_overlapping_impls(&mut self) {
        let impls: Vec<DefId> =
            self.p.impls.iter().copied().filter(|i| matches!(&self.p.def(*i).data, DefData::Impl(d) if d.trait_ref.is_some() && !d.negative)).collect();
        for (k, &b) in impls.iter().enumerate() {
            for &a in &impls[..k] {
                let Some(notes) = self.p.impls_overlap(a, b) else { continue };
                let header = |s: &Self, id: DefId| match s.ast(id) {
                    AstRef::Impl(im) => im.trait_ref.as_ref().map_or(im.self_ty.span, |t| t.span.to(im.self_ty.span)),
                    _ => s.p.def(id).span,
                };
                // Report at the impl outside core/std when there is one.
                let is_std = |s: &Self, id: DefId| {
                    let k = s.p.crater_of(id);
                    s.p.crate_roots.iter().any(|(n, r)| *r == k && (n == "core" || n == "std"))
                };
                let (here, first) = if is_std(self, b) && !is_std(self, a) { (a, b) } else { (b, a) };
                let (sh, sf) = (header(self, here), header(self, first));
                let td = self.p.impl_data(here);
                let tname = self.p.def(td.trait_ref.as_ref().unwrap().trait_id).name.clone();
                let tys = self.p.ty_str(&td.self_ty);
                let mut d = Diagnostic::error(trait_::CONFLICTING_IMPL, sh, format!("conflicting implementations of trait `{tname}` for `{tys}`"))
                    .with_label("conflicting implementation")
                    .with_secondary(sf, "other implementation here")
                    .with_note("two impls of a trait may not apply to the same type, or calls would be ambiguous");
                for n in notes {
                    d = d.with_note(n);
                }
                self.err(d);
                break;
            }
        }
    }

    fn check_impl(&mut self, id: DefId, im: &ast::ImplDecl) {
        let DefData::Impl(data) = &self.p.defs[id.0 as usize].data else { return };
        let data = data.clone();
        let krate = self.mod_data(self.p.defs[id.0 as usize].module).krate;
        let is_std = self.p.crate_roots.iter().any(|(n, r)| *r == krate && (n == "core" || n == "std"));
        let Some(tr) = &data.trait_ref else {
            // Inherent impl.
            let ok = match &data.self_ty {
                Ty::Adt(d, _) => self.mod_data(self.p.defs[d.0 as usize].module).krate == krate,
                Ty::Error => true,
                _ => is_std,
            };
            if !ok {
                let s = self.p.ty_str(&data.self_ty);
                self.err(
                    Diagnostic::error(
                        trait_::INHERENT_FOREIGN,
                        im.self_ty.span,
                        format!("cannot define inherent methods on `{s}` outside the crater that defines it"),
                    )
                    .with_help("define a trait with these methods and implement it instead"),
                );
            }
            return;
        };
        let tname = self.p.defs[tr.trait_id.0 as usize].name.clone();
        let tr_span = im.trait_ref.as_ref().unwrap().span;
        if self.p.is_lang(tr.trait_id, "dull") {
            let msg = if data.negative {
                "`Dull` cannot be negated: whether a type is Dull is decided by the compiler"
            } else {
                "`Dull` cannot be implemented manually: the compiler implements it"
            };
            self.err(
                Diagnostic::error(trait_::DULL_IMPL, tr_span, msg)
                    .with_note("the compiler implements `core::ptr::Dull` automatically for every type without pointers, slices, strings or arrays"),
            );
            return;
        }
        if self.p.is_lang(tr.trait_id, "light") {
            let msg = if data.negative {
                "`Light` cannot be negated: whether a type is Light is decided by the compiler"
            } else {
                "`Light` cannot be implemented manually: the compiler implements it"
            };
            self.err(
                Diagnostic::error(trait_::DULL_IMPL, tr_span, msg)
                    .with_note("the compiler implements `core::ptr::Light` automatically for every type that owns no array or string"),
            );
            return;
        }
        if data.negative {
            self.err(
                Diagnostic::error(trait_::NEGATIVE_IMPL, tr_span, format!("`{tname}` is not an auto trait, so it cannot have a negative impl"))
                    .with_note("negative impls are only meaningful for auto traits, and Lunar's only auto trait is `Dull`, which cannot be negated either")
                    .with_help(format!("a type without an `impl {tname}` already does not implement `{tname}`; remove this impl")),
            );
            return;
        }
        let td = self.p.trait_data(tr.trait_id).clone();
        let impl_count = data.generics.count();
        for m in &td.methods {
            match data.methods.get(&m.name) {
                None if !m.has_default => {
                    self.err(
                        Diagnostic::error(trait_::MISSING_ITEM, im.self_ty.span, format!("missing method `{}` in implementation of `{tname}`", m.name))
                            .with_secondary(self.p.defs[m.def.0 as usize].span, "required by the trait here"),
                    );
                }
                None => {}
                Some(&imp_m) => self.compare_sigs(m.def, imp_m, &data.self_ty, tr, impl_count),
            }
        }
        for (name, &m) in &data.methods {
            if !td.methods.iter().any(|tm| tm.name == *name) {
                let sp = self.p.defs[m.0 as usize].span;
                self.err(Diagnostic::error(trait_::EXTRA_ITEM, sp, format!("method `{name}` is not a member of trait `{tname}`")));
            }
        }
        // Supertraits must also be implemented.
        let mut args = vec![data.self_ty.clone()];
        args.extend(tr.args.iter().cloned());
        let env = data.generics.preds.clone();
        for st in &td.supertraits {
            let st = st.subst(&args);
            if !self.p.implements(&data.self_ty, &st, &env) {
                let s = self.p.ty_str(&data.self_ty);
                let sn = self.p.defs[st.trait_id.0 as usize].name.clone();
                self.err(Diagnostic::error(tyc::UNSATISFIED_BOUND, im.self_ty.span, format!("`{s}` implements `{tname}` but not its supertrait `{sn}`")));
            }
        }
    }

    fn compare_sigs(&mut self, trait_m: DefId, impl_m: DefId, self_ty: &Ty, tr: &TraitRef, impl_count: usize) {
        let tf = self.p.fn_data(trait_m).clone();
        let imf = self.p.fn_data(impl_m).clone();
        let sp = self.p.defs[impl_m.0 as usize].span;
        if tf.generics.own_count() != imf.generics.own_count() {
            self.err(Diagnostic::error(trait_::SIGNATURE, sp, "method has a different number of generic parameters than the trait declares"));
            return;
        }
        // Map trait-method params [Self, trait params.., method params..] to the impl numbering.
        let mut args = vec![self_ty.clone()];
        args.extend(tr.args.iter().cloned());
        for (j, n) in imf.generics.names.iter().enumerate() {
            args.push(Ty::Param((impl_count + j) as u32, n.clone()));
        }
        if tf.sig.params.len() != imf.sig.params.len() {
            self.err(
                Diagnostic::error(
                    trait_::SIGNATURE,
                    sp,
                    format!("method takes {} parameter(s) but the trait declares {}", imf.sig.params.len(), tf.sig.params.len()),
                )
                .with_secondary(self.p.defs[trait_m.0 as usize].span, "trait method declared here"),
            );
            return;
        }
        for (tp, ip) in tf.sig.params.iter().zip(&imf.sig.params) {
            let want = tp.ty.subst(&args);
            if want != ip.ty || tp.mode != ip.mode {
                let (w, g) = (self.p.ty_str(&want), self.p.ty_str(&ip.ty));
                let prefix = |m: ast::ParamMode| match m {
                    ast::ParamMode::Take => "",
                    ast::ParamMode::Borrow => "&",
                    ast::ParamMode::InOut => "&mut ",
                };
                let (wm, gm) = (prefix(tp.mode), prefix(ip.mode));
                self.err(
                    Diagnostic::error(trait_::SIGNATURE, ip.span, format!("parameter type mismatch with trait: expected `{wm}{w}`, found `{gm}{g}`"))
                        .with_secondary(tp.span, "trait parameter declared here"),
                );
            } else if ip.kept && !tp.kept {
                // Callers through the trait would not know the view is kept.
                self.err(
                    Diagnostic::error(trait_::SIGNATURE, ip.span, format!("parameter `{}` is `kept`, but the trait's is not", ip.name))
                        .with_secondary(tp.span, "trait parameter declared here")
                        .with_help("mark the trait's parameter `kept` too, or do not store views from this argument"),
                );
            }
        }
        let want_ret = tf.sig.ret.subst(&args);
        if want_ret != imf.sig.ret {
            let (w, g) = (self.p.ty_str(&want_ret), self.p.ty_str(&imf.sig.ret));
            self.err(Diagnostic::error(trait_::SIGNATURE, imf.sig.ret_span, format!("return type mismatch with trait: expected `{w}`, found `{g}`")));
        }
    }

    fn check_main(&mut self) {
        let Some((_, root)) = self.p.crate_roots.first().cloned() else { return };
        let Some(b) = self.lookup_in(root, "main", Ns::Value) else { return };
        let DefData::Fn(f) = &self.p.defs[b.def.0 as usize].data else { return };
        if !f.sig.params.is_empty() || !(f.sig.ret.is_unit() || f.sig.ret == Ty::Int(IntTy::I32)) || f.generics.count() > 0 {
            self.err(Diagnostic::error(tyc::BAD_MAIN, b.span, "`main` must take no parameters and return `()` or `i32`"));
        }
    }

    // ===================================================================== const evaluation

    /// Evaluate a constant expression of (expected) type `ty`.
    pub(crate) fn eval_const(&mut self, module: DefId, e: &ast::Expr, ty: &Ty) -> Option<ConstVal> {
        match self.eval_const_inner(module, e, ty) {
            Ok(v) => Some(v),
            Err(msg) => {
                self.err(Diagnostic::error(tyc::CONST_EVAL, e.span, msg));
                None
            }
        }
    }

    fn eval_const_inner(&mut self, module: DefId, e: &ast::Expr, ty: &Ty) -> Result<ConstVal, String> {
        use ast::ExprKind as K;
        Ok(match &e.kind {
            K::Lit(l) => match l {
                ast::Lit::Int { value, .. } => {
                    if let Ty::Float(_) = ty {
                        ConstVal::Float(*value as f64)
                    } else {
                        ConstVal::Int(*value as i128)
                    }
                }
                ast::Lit::Float { value, .. } => ConstVal::Float(*value),
                ast::Lit::Bool(b) => ConstVal::Bool(*b),
                ast::Lit::Char(c) => ConstVal::Char(*c),
                ast::Lit::Str { value, .. } => ConstVal::Str(value.clone()),
            },
            K::Paren(x) => self.eval_const_inner(module, x, ty)?,
            K::Unary(op, x) => {
                let v = self.eval_const_inner(module, x, ty)?;
                match (op, v) {
                    (ast::UnOp::Neg, ConstVal::Int(i)) => ConstVal::Int(-i),
                    (ast::UnOp::Neg, ConstVal::Float(f)) => ConstVal::Float(-f),
                    (ast::UnOp::Not, ConstVal::Bool(b)) => ConstVal::Bool(!b),
                    (ast::UnOp::Not, ConstVal::Int(i)) => ConstVal::Int(!i),
                    _ => return Err("unsupported operation in constant".into()),
                }
            }
            K::Binary(op, a, b) => {
                let x = self.eval_const_inner(module, a, ty)?;
                let y = self.eval_const_inner(module, b, ty)?;
                use ast::BinOp::*;
                match (x, y) {
                    (ConstVal::Int(x), ConstVal::Int(y)) => match op {
                        Add => ConstVal::Int(x.wrapping_add(y)),
                        Sub => ConstVal::Int(x.wrapping_sub(y)),
                        Mul => ConstVal::Int(x.wrapping_mul(y)),
                        Div if y != 0 => ConstVal::Int(x / y),
                        Rem if y != 0 => ConstVal::Int(x % y),
                        Div | Rem => return Err("division by zero in constant".into()),
                        Shl => ConstVal::Int(x << (y as u32 & 127)),
                        Shr => ConstVal::Int(x >> (y as u32 & 127)),
                        BitAnd => ConstVal::Int(x & y),
                        BitOr => ConstVal::Int(x | y),
                        BitXor => ConstVal::Int(x ^ y),
                        Eq => ConstVal::Bool(x == y),
                        Ne => ConstVal::Bool(x != y),
                        Lt => ConstVal::Bool(x < y),
                        Le => ConstVal::Bool(x <= y),
                        Gt => ConstVal::Bool(x > y),
                        Ge => ConstVal::Bool(x >= y),
                        And | Or => return Err("`&&`/`||` need booleans".into()),
                    },
                    (ConstVal::Float(x), ConstVal::Float(y)) => match op {
                        Add => ConstVal::Float(x + y),
                        Sub => ConstVal::Float(x - y),
                        Mul => ConstVal::Float(x * y),
                        Div => ConstVal::Float(x / y),
                        _ => return Err("unsupported float operation in constant".into()),
                    },
                    (ConstVal::Bool(x), ConstVal::Bool(y)) => match op {
                        And => ConstVal::Bool(x && y),
                        Or => ConstVal::Bool(x || y),
                        Eq => ConstVal::Bool(x == y),
                        Ne => ConstVal::Bool(x != y),
                        _ => return Err("unsupported boolean operation in constant".into()),
                    },
                    _ => return Err("mismatched operand types in constant".into()),
                }
            }
            K::Cast(x, _) => {
                let v = self.eval_const_inner(module, x, &Ty::Error)?;
                match (v, ty) {
                    (ConstVal::Int(i), Ty::Float(_)) => ConstVal::Float(i as f64),
                    (ConstVal::Float(f), Ty::Int(_)) => ConstVal::Int(f as i128),
                    (v, _) => v,
                }
            }
            K::Path(p) => {
                let idents: Vec<ast::Ident> = p.segments.iter().map(|s| s.ident.clone()).collect();
                match self.resolve_prefix(module, &idents, Ns::Value) {
                    Some((ModRes::Def(d), n)) if n == idents.len() => match self.ast(d) {
                        AstRef::Const(c) => {
                            if let DefData::Const(cd) = &self.p.defs[d.0 as usize].data
                                && let Some(v) = &cd.value
                            {
                                return Ok(v.clone());
                            }
                            let m = self.p.defs[d.0 as usize].module;
                            let cx = LowerCx::new(m);
                            let cty = self.lower_ty(&cx, &c.ty, TyPos::Other);
                            self.eval_const_inner(m, &c.value, &cty)?
                        }
                        _ => return Err(format!("`{}` is not a constant", p.to_string_lossy())),
                    },
                    _ => return Err(format!("cannot find constant `{}`", p.to_string_lossy())),
                }
            }
            K::Array(es) => {
                let et = match ty {
                    Ty::Array(e, _) => (**e).clone(),
                    _ => Ty::Error,
                };
                ConstVal::Array(es.iter().map(|x| self.eval_const_inner(module, x, &et)).collect::<Result<_, _>>()?)
            }
            K::ArrayRepeat(v, n) => {
                let et = match ty {
                    Ty::Array(e, _) => (**e).clone(),
                    _ => Ty::Error,
                };
                let v = self.eval_const_inner(module, v, &et)?;
                let ConstVal::Int(n) = self.eval_const_inner(module, n, &Ty::usize())? else {
                    return Err("array repeat count must be an integer".into());
                };
                ConstVal::Array(vec![v; n.max(0) as usize])
            }
            K::Tuple(es) => {
                let tys: Vec<Ty> = match ty {
                    Ty::Tuple(ts) => ts.clone(),
                    _ => vec![Ty::Error; es.len()],
                };
                if es.is_empty() {
                    ConstVal::Unit
                } else {
                    ConstVal::Tuple(es.iter().zip(tys).map(|(x, t)| self.eval_const_inner(module, x, &t)).collect::<Result<_, _>>()?)
                }
            }
            _ => return Err("this expression is not supported in constants".into()),
        })
    }
}
