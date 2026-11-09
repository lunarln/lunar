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
        ty::TyDisplay { ty: t, names: &names }.to_string()
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
