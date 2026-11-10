//! Semantic analysis for Lunar: name resolution, type checking, traits, the
//! `Dull` auto trait, unsafe checking, and move/borrow checking. Produces a
//! typed IR ([`thir`]) for code generation.

pub mod borrowck;
mod collect;
pub mod defs;
mod expand;
pub mod layout;
pub mod loader;
mod lower;
mod resolve;
pub mod thir;
pub mod traits;
pub mod ty;
mod typeck;

use defs::*;
use ln_ast as ast;
use ln_diag::{Diagnostic, Diagnostics};
use loader::CrateSource;
use std::collections::HashMap;
use ty::Ty;

/// Compilation target information used by `#[cfg(...)]`.
#[derive(Clone, Debug)]
pub struct Options {
    /// `windows`, `linux`, `macos`, ...
    pub target_os: String,
    /// `windows` or `unix`.
    pub target_family: String,
    /// `x86_64`, `aarch64`, ...
    pub target_arch: String,
    /// Building the test harness (`lunar test`).
    pub test: bool,
    /// Type-check function bodies even after errors in item signatures, and
    /// return the (partial) program despite errors. For editor tooling.
    pub keep_going: bool,
}

impl Options {
    pub fn host(test: bool) -> Self {
        let os = std::env::consts::OS.to_string();
        let family = if cfg!(windows) { "windows" } else { "unix" }.to_string();
        Self { target_os: os, target_family: family, target_arch: std::env::consts::ARCH.to_string(), test, keep_going: false }
    }
}

/// A resolved name occurrence in the source, for editor tooling.
#[derive(Clone, Debug)]
pub struct Ref {
    /// The identifier (or path segment) that was resolved.
    pub span: ln_diag::Span,
    pub target: RefTarget,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefTarget {
    Def(DefId),
    /// Field `field` of variant `variant` of struct/enum `adt`.
    Field {
        adt: DefId,
        variant: usize,
        field: usize,
    },
    /// A local of the body of `body`.
    Local {
        body: DefId,
        local: thir::LocalId,
    },
    /// A generic parameter, by the span of its name where it is declared.
    TyParam(ln_diag::Span),
}

/// The analyzed program handed to code generation.
#[derive(Debug, Default)]
pub struct Program {
    pub defs: Vec<DefInfo>,
    pub bodies: HashMap<DefId, thir::Body>,
    /// Lang items (`#[lang = "..."]`) by name.
    pub lang: HashMap<String, DefId>,
    pub crate_roots: Vec<(String, DefId)>,
    pub impls: Vec<DefId>,
    pub main: Option<DefId>,
    pub tests: Vec<DefId>,
    /// Every resolved name, in resolution order (may contain duplicates).
    pub refs: Vec<Ref>,
    /// For each macro expansion's file, where the macro was invoked.
    pub expansion_sites: HashMap<ln_diag::FileId, ln_diag::Span>,
    /// Closures, by the function definition each one is compiled to.
    pub closures: HashMap<DefId, ClosureInfo>,
    /// `impl Trait` return types, by function and position (`Ty::Opaque`).
    pub opaques: HashMap<(DefId, u32), OpaqueInfo>,
    /// For each function (and closure) with a checked body, which of its
    /// parameters the value it returns may point into, inferred from the
    /// body (see `borrowck::infer_ret_points`).
    pub ret_points: HashMap<DefId, borrowck::RetPoints>,
    /// Native libraries to link, from `#[link(name = "...")]` on `extern`
    /// blocks, in first-seen order.
    pub link_libs: Vec<String>,
}

/// An `impl Trait` in a function's return type.
#[derive(Clone, Debug)]
pub struct OpaqueInfo {
    /// What callers know about it (in terms of the function's generics).
    pub bounds: Vec<ty::TraitRef>,
    /// The type the function's body returns there, once checked.
    pub concrete: Option<Ty>,
    pub span: ln_diag::Span,
}

/// A closure is compiled to a function whose leading parameters are its
/// captures (in this order), followed by its own parameters. Its value is a
/// struct of the captures: a pointer for each borrowed one, the value for
/// each one it took.
#[derive(Clone, Debug)]
pub struct ClosureInfo {
    /// The function the closure is written in; the closure has its generics.
    pub parent: DefId,
    pub captures: Vec<Capture>,
    pub kind: ClosureKind,
}

#[derive(Clone, Debug)]
pub struct Capture {
    pub name: String,
    /// The captured variable's type (in terms of the parent's generics).
    pub ty: Ty,
    /// Kept as a pointer to the variable, rather than moved or copied into
    /// the closure.
    pub by_ref: bool,
    /// How the body uses it, which is the mode of its parameter: read
    /// (`Borrow`), changed (`InOut`) or consumed (`Take`).
    pub mode: defs::ParamMode,
}

/// What calling a closure may do to its captures (`Fn` < `FnMut` < `FnOnce`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ClosureKind {
    Fn,
    FnMut,
    FnOnce,
}

impl ClosureKind {
    pub fn lang(self) -> &'static str {
        match self {
            ClosureKind::Fn => "fn",
            ClosureKind::FnMut => "fn_mut",
            ClosureKind::FnOnce => "fn_once",
        }
    }

    pub fn from_lang(name: &str) -> Option<ClosureKind> {
        Some(match name {
            "fn" => ClosureKind::Fn,
            "fn_mut" => ClosureKind::FnMut,
            "fn_once" => ClosureKind::FnOnce,
            _ => return None,
        })
    }
}

impl Program {
    /// The fields of a closure's value, for closure `def` instantiated
    /// with `args`: a pointer per borrowed capture, the value otherwise.
    pub fn closure_fields(&self, def: DefId, args: &[Ty]) -> Vec<Ty> {
        let Some(c) = self.closures.get(&def) else { return Vec::new() };
        c.captures
            .iter()
            .map(|cap| {
                let t = cap.ty.subst(args);
                if cap.by_ref { Ty::Ptr(Box::new(t), cap.mode == defs::ParamMode::InOut) } else { t }
            })
            .collect()
    }

    /// The signature `fn(params): ret` a closure is called with (without
    /// its captures), for closure `def` instantiated with `args`.
    pub fn closure_sig(&self, def: DefId, args: &[Ty]) -> Ty {
        let fd = self.fn_data(def);
        let n = self.closures.get(&def).map_or(0, |c| c.captures.len());
        let params = fd.sig.params[n..].iter().map(|p| (p.mode, p.ty.subst(args))).collect();
        Ty::FnPtr(params, Box::new(fd.sig.ret.subst(args)))
    }

    /// What is known about an `impl Trait` type: its bounds, as predicates
    /// on it.
    pub fn opaque_preds(&self, def: DefId, idx: u32, args: &[Ty]) -> Vec<defs::Pred> {
        let Some(o) = self.opaques.get(&(def, idx)) else { return Vec::new() };
        let ty = Ty::Opaque(def, idx, args.to_vec());
        o.bounds.iter().map(|b| defs::Pred { ty: ty.clone(), trait_ref: b.subst(args), span: o.span }).collect()
    }

    /// `t` with every `impl Trait` replaced by the type it stands for (for
    /// code generation, after the functions returning them are checked).
    pub fn reveal(&self, t: &Ty) -> Ty {
        let mut depth = 0;
        let mut t = t.clone();
        while depth < 32 && {
            let mut any = false;
            t.walk(&mut |x| any |= matches!(x, Ty::Opaque(..)));
            any
        } {
            t = t.map(&mut |x| match x {
                Ty::Opaque(d, i, args) => match self.opaques.get(&(d, i)).and_then(|o| o.concrete.as_ref()) {
                    Some(c) => c.subst(&args),
                    None => Ty::Error,
                },
                x => x,
            });
            depth += 1;
        }
        t
    }

    /// A trait with its arguments, as written in a bound: `Iterator::<i32>`,
    /// or with the `Fn` sugar, `Fn(i32): bool`.
    pub fn trait_ref_str(&self, tr: &ty::TraitRef) -> String {
        let name = self.def(tr.trait_id).name.clone();
        let fn_like = self.lang_name(tr.trait_id).and_then(ClosureKind::from_lang).is_some();
        match tr.args.as_slice() {
            [] => name,
            [sig @ Ty::FnPtr(..)] if fn_like => format!("{name}{}", self.ty_str(sig).trim_start_matches("fn")),
            args => format!("{name}::<{}>", args.iter().map(|a| self.ty_str(a)).collect::<Vec<_>>().join(", ")),
        }
    }
}

impl Program {
    /// Where `span` is in code the user wrote: spans inside macro
    /// expansions map to the invocation (repeatedly, for nested ones).
    pub fn source_span(&self, mut span: ln_diag::Span) -> ln_diag::Span {
        while let Some(site) = self.expansion_sites.get(&span.file) {
            span = *site;
        }
        span
    }
}

impl Program {
    pub fn def(&self, id: DefId) -> &DefInfo {
        &self.defs[id.0 as usize]
    }

    pub fn fn_data(&self, id: DefId) -> &FnData {
        match &self.def(id).data {
            DefData::Fn(f) => f,
            _ => panic!("not a fn: {}", self.def(id).name),
        }
    }

    pub fn adt(&self, id: DefId) -> &AdtData {
        match &self.def(id).data {
            DefData::Adt(a) => a,
            _ => panic!("not an adt: {}", self.def(id).name),
        }
    }

    pub fn trait_data(&self, id: DefId) -> &TraitData {
        match &self.def(id).data {
            DefData::Trait(t) => t,
            _ => panic!("not a trait"),
        }
    }

    pub fn impl_data(&self, id: DefId) -> &ImplData {
        match &self.def(id).data {
            DefData::Impl(i) => i,
            _ => panic!("not an impl"),
        }
    }

    /// Whether parameter `index` of function `def` is passed as a pointer to
    /// the caller's place: in-out (`&mut T`) and borrowed (`&T`) parameters
    /// of Lunar functions. Intrinsics and `extern` functions take values.
    pub fn param_by_ref(&self, def: DefId, index: usize) -> bool {
        let fd = self.fn_data(def);
        let Some(p) = fd.sig.params.get(index) else { return false };
        if p.inout {
            return true;
        }
        p.mode == ParamMode::Borrow && fd.abi == Abi::Lunar && !self.def(def).attrs.intrinsic
    }

    pub fn lang_item(&self, name: &str) -> Option<DefId> {
        self.lang.get(name).copied()
    }

    /// Fully qualified display path of an item, e.g. `std::io::println`.
    pub fn path_of(&self, id: DefId) -> String {
        let d = self.def(id);
        match d.kind {
            DefKind::Mod => {
                let DefData::Mod(md) = &d.data else { unreachable!() };
                match md.parent {
                    Some(p) if md.is_block => self.path_of(p),
                    Some(p) => format!("{}::{}", self.path_of(p), d.name),
                    None => d.name.clone(),
                }
            }
            DefKind::Variant => format!("{}::{}", self.path_of(d.parent.unwrap()), d.name),
            DefKind::Fn if d.parent.is_some() => {
                let parent = d.parent.unwrap();
                match self.def(parent).kind {
                    DefKind::Impl => {
                        let i = self.impl_data(parent);
                        format!("{}::{}", self.ty_str(&i.self_ty), d.name)
                    }
                    _ => format!("{}::{}", self.path_of(parent), d.name),
                }
            }
            _ => format!("{}::{}", self.path_of(d.module), d.name),
        }
    }

    pub fn ty_str(&self, t: &Ty) -> String {
        let names = |id: DefId| self.def(id).name.clone();
        let opaque = |def: DefId, idx: u32, args: &[Ty]| {
            let bounds: Vec<String> = self.opaque_preds(def, idx, args).iter().map(|p| self.trait_ref_str(&p.trait_ref)).collect();
            format!("impl {}", bounds.join(" + "))
        };
        ty::TyDisplay { ty: t, names: &names, opaque: &opaque }.to_string()
    }

    pub fn is_enum(&self, id: DefId) -> bool {
        self.def(id).kind == DefKind::Enum
    }
}

/// AST node backing a definition (only during analysis).
#[derive(Clone, Copy)]
pub(crate) enum AstRef<'a> {
    None,
    /// The bool marks functions declared in `extern` blocks.
    Fn(&'a ast::FnDecl, bool),
    Struct(&'a ast::StructDecl),
    Enum(&'a ast::EnumDecl),
    #[allow(dead_code)]
    Variant(&'a ast::VariantDecl),
    Trait(&'a ast::TraitDecl),
    Impl(&'a ast::ImplDecl),
    Const(&'a ast::ConstDecl),
    Static(&'a ast::StaticDecl),
    Alias(&'a ast::TypeAlias),
    Macro(&'a ast::MacroDef),
}

pub(crate) struct Sema<'a> {
    pub(crate) p: Program,
    pub(crate) asts: Vec<AstRef<'a>>,
    pub(crate) diags: Diagnostics,
    pub(crate) opts: Options,
    pub(crate) pending_uses: Vec<(DefId, &'a ast::UseTree, bool)>,
    pub(crate) impl_items: HashMap<DefId, Vec<DefId>>,
    pub(crate) block_scopes: HashMap<ast::NodeId, DefId>,
    pub(crate) prelude: Option<DefId>,
    /// Array element / `!` checks deferred until all ADT fields are known.
    pub(crate) deferred: Vec<lower::Deferred>,
    pub(crate) alias_stack: Vec<DefId>,
    /// For each crater root, the craters it may name and under what name.
    pub(crate) crater_deps: HashMap<DefId, Vec<(String, DefId)>>,

    // ----- macros (see `expand`) -----
    /// The sources, plus a file for each expansion (taken from the caller
    /// for the analysis and handed back).
    pub(crate) map: ln_diag::SourceMap,
    /// Syntax contexts of identifiers in expansion files.
    pub(crate) file_ctxs: HashMap<ln_diag::FileId, ln_parser::Ctxs>,
    /// Node ids for expanded syntax.
    pub(crate) ids: ln_parser::IdGen,
    /// One per expansion; index 0 stands for `SyntaxCtx::ROOT`.
    pub(crate) marks: Vec<expand::Mark>,
    pub(crate) macro_rules: HashMap<DefId, Vec<expand::Rule<'a>>>,
    pub(crate) pending_macros: Vec<expand::Pending<'a>>,
    pub(crate) arenas: &'a expand::Arenas,
    pub(crate) ty_expansions: HashMap<(ln_diag::FileId, u32), Option<&'a ast::Ty>>,
}

impl<'a> Sema<'a> {
    pub(crate) fn err(&mut self, d: Diagnostic) {
        // Lints are about code someone wrote by hand; inside a macro's
        // expansion the caller could not act on them.
        if !d.is_error() && self.p.expansion_sites.contains_key(&d.span.file) {
            return;
        }
        self.diags.push(d);
    }
}

/// Analyze a set of crates. The first crate is the user's; `core` and `std`
/// must be among them. Returns the program (if there were no errors) and all
/// diagnostics. Macro expansions are added to `map` as files of their own.
pub fn analyze(crates: &[CrateSource], map: &mut ln_diag::SourceMap, opts: Options) -> (Option<Program>, Vec<Diagnostic>) {
    let arenas = expand::Arenas::default();
    let mut s = Sema {
        map: std::mem::take(map),
        file_ctxs: HashMap::new(),
        // Loaded files number their nodes from 0; expansions start far above.
        ids: ln_parser::IdGen::starting_at(1 << 30),
        marks: vec![expand::Mark { def_module: DefId(0), def_ctx: ast::SyntaxCtx::ROOT, depth: 0 }],
        macro_rules: HashMap::new(),
        pending_macros: Vec::new(),
        arenas: &arenas,
        ty_expansions: HashMap::new(),
        p: Program::default(),
        asts: Vec::new(),
        diags: Diagnostics::new(),
        opts,
        pending_uses: Vec::new(),
        impl_items: HashMap::new(),
        block_scopes: HashMap::new(),
        prelude: None,
        deferred: Vec::new(),
        alias_stack: Vec::new(),
        crater_deps: HashMap::new(),
    };
    let mut user_root = None;
    let mut roots = Vec::new();
    for (i, c) in crates.iter().enumerate() {
        let root = s.collect_crate(c);
        roots.push(root);
        if i == 0 {
            user_root = Some(root);
        }
    }
    for (c, &root) in crates.iter().zip(&roots) {
        let deps = c.deps.iter().filter_map(|(alias, i)| roots.get(*i).map(|r| (alias.clone(), *r))).collect();
        s.crater_deps.insert(root, deps);
    }
    if let Some((_, core)) = s.p.crate_roots.iter().find(|(n, _)| n == "core").cloned()
        && let Some(b) = s.lookup_in(core, "prelude", Ns::Type)
    {
        s.prelude = Some(b.def);
    }
    s.resolve_imports();
    s.expand_items();
    s.lower_all();
    if let Some(root) = user_root
        && let Some(b) = s.lookup_in(root, "main", Ns::Value)
        && s.p.def(b.def).kind == DefKind::Fn
    {
        s.p.main = Some(b.def);
    }
    if !s.diags.has_errors() || s.opts.keep_going {
        s.check_bodies();
    }
    *map = std::mem::take(&mut s.map);
    let diags = s.diags.into_vec();
    let ok = !diags.iter().any(|d| d.is_error());
    (if ok || s.opts.keep_going { Some(s.p) } else { None }, diags)
}
