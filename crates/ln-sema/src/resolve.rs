//! Module-level name resolution: `use` imports and item paths.

use crate::Sema;
use crate::defs::*;
use crate::ty::{FloatTy, IntTy, Ty};
use ln_ast as ast;
use ln_diag::{Diagnostic, Span, codes::name};

/// The name an item a macro expansion defines is bound under, unless it is
/// `#[export]`ed: `#` cannot appear in an identifier, so only names from the
/// same expansion (same context) find it.
pub(crate) fn hidden_name(name: &str, ctx: ast::SyntaxCtx) -> String {
    format!("{name}#{}", ctx.0)
}

/// What a module-level path resolves to.
#[derive(Clone, Debug)]
pub(crate) enum ModRes {
    Def(DefId),
    Prim(Ty),
}

pub(crate) fn prim_type(name: &str) -> Option<Ty> {
    Some(match name {
        "bool" => Ty::Bool,
        "char" => Ty::Char,
        "str" => Ty::Str,
        "String" => Ty::String,
        "FmtArgs" => Ty::FmtArgs,
        "f32" => Ty::Float(FloatTy::F32),
        "f64" => Ty::Float(FloatTy::F64),
        _ => Ty::Int(IntTy::from_name(name)?),
    })
}

impl<'a> Sema<'a> {
    /// The nearest enclosing non-block module.
    pub(crate) fn normal_module(&self, mut m: DefId) -> DefId {
        while self.mod_data(m).is_block {
            m = self.mod_data(m).parent.unwrap();
        }
        m
    }

    /// Whether code in module `from` may see private items of module `owner`.
    pub(crate) fn is_ancestor_or_self(&self, owner: DefId, from: DefId) -> bool {
        let mut cur = Some(from);
        while let Some(m) = cur {
            if m == owner {
                return true;
            }
            cur = self.mod_data(m).parent;
        }
        false
    }

    /// Look `name` up directly in module (or enum) `m`, including globs.
    pub(crate) fn lookup_in(&self, m: DefId, name: &str, ns: Ns) -> Option<Binding> {
        self.lookup_in_rec(m, name, ns, &mut Vec::new())
    }

    fn lookup_in_rec(&self, m: DefId, name: &str, ns: Ns, seen: &mut Vec<DefId>) -> Option<Binding> {
        if seen.contains(&m) {
            return None;
        }
        seen.push(m);
        let d = &self.p.defs[m.0 as usize];
        match &d.data {
            DefData::Mod(md) => {
                if let Some(b) = md.ns(ns).get(name) {
                    return Some(*b);
                }
                for &(g, is_pub) in &md.globs {
                    if let Some(b) = self.lookup_in_rec(g, name, ns, seen) {
                        if b.is_pub || self.p.defs[g.0 as usize].kind == DefKind::Enum {
                            return Some(Binding { def: b.def, is_pub, span: b.span });
                        }
                    }
                }
                None
            }
            DefData::Adt(adt) if adt.is_enum => adt.variants.iter().find(|v| v.name == name).map(|v| Binding { def: v.def, is_pub: true, span: v.span }),
            _ => None,
        }
    }

    /// Unqualified lookup from inside module `from`: block scopes, the
    /// module itself, then the prelude, crate names and primitive types.
    pub(crate) fn lookup_unqualified(&self, from: DefId, name: &str, ns: Ns) -> Option<ModRes> {
        let mut m = from;
        loop {
            if let Some(b) = self.lookup_in(m, name, ns) {
                return Some(ModRes::Def(b.def));
            }
            let md = self.mod_data(m);
            if md.is_block {
                m = md.parent.unwrap();
                continue;
            }
            break;
        }
        if let Some(prelude) = self.prelude
            && let Some(b) = self.lookup_in(prelude, name, ns)
        {
            return Some(ModRes::Def(b.def));
        }
        if ns == Ns::Type {
            // Craters by name: this crater's declared dependencies, then
            // `core` and `std`, which every crater can see.
            let krate = self.mod_data(from).krate;
            if let Some(deps) = self.crater_deps.get(&krate)
                && let Some((_, root)) = deps.iter().find(|(n, _)| n == name)
            {
                return Some(ModRes::Def(*root));
            }
            if (name == "core" || name == "std")
                && let Some((_, root)) = self.p.crate_roots.iter().find(|(n, _)| n == name)
            {
                return Some(ModRes::Def(*root));
            }
            if let Some(t) = prim_type(name) {
                return Some(ModRes::Prim(t));
            }
        }
        None
    }

    /// Unqualified lookup of an identifier, with hygiene (docs/design/macros.md
    /// section 4). A name written by hand resolves from `from`. A name a
    /// macro's body wrote resolves first among the items that same
    /// expansion defined (bound under a hidden name, so nothing else sees
    /// them), then where the macro is defined.
    pub(crate) fn lookup_ident(&self, from: DefId, id: &ast::Ident, ns: Ns) -> Option<ModRes> {
        if id.ctx.is_root() {
            return self.lookup_unqualified(from, &id.name, ns);
        }
        if let Some(r) = self.lookup_unqualified(from, &hidden_name(&id.name, id.ctx), ns) {
            return Some(r);
        }
        let mark = &self.marks[id.ctx.0 as usize];
        let at_def = ast::Ident { name: id.name.clone(), span: id.span, ctx: mark.def_ctx };
        self.lookup_ident(mark.def_module, &at_def, ns)
    }

    /// The module `crater`, `self` and `super` in a path are relative to:
    /// for a name a macro's body wrote, the module the macro is defined in.
    fn path_base(&self, from: DefId, first: &ast::Ident) -> DefId {
        let mut from = from;
        let mut ctx = first.ctx;
        while !ctx.is_root() {
            let mark = &self.marks[ctx.0 as usize];
            from = mark.def_module;
            ctx = mark.def_ctx;
        }
        from
    }

    /// Resolve the longest module-level prefix of `segs`. Returns the result
    /// and how many segments were consumed; remaining segments are
    /// type-relative (associated items). Reports errors.
    pub(crate) fn resolve_prefix(&mut self, from: DefId, segs: &[ast::Ident], last_ns: Ns) -> Option<(ModRes, usize)> {
        let first = &segs[0];
        let first_ns = if segs.len() == 1 { last_ns } else { Ns::Type };
        let base = self.path_base(from, first);
        let mut cur = match first.name.as_str() {
            "crater" => DefId(self.mod_data(base).krate.0),
            "self" if segs.len() > 1 => self.normal_module(base),
            "super" => {
                let m = self.normal_module(base);
                match self.mod_data(m).parent {
                    Some(p) => p,
                    None => {
                        self.err(Diagnostic::error(name::UNRESOLVED, first.span, "`super` used in a crater root"));
                        return None;
                    }
                }
            }
            _ => match self.lookup_ident(from, first, first_ns) {
                Some(ModRes::Def(d)) => d,
                Some(ModRes::Prim(t)) => return Some((ModRes::Prim(t), 1)),
                None => {
                    // A single segment might name a value while we looked for a
                    // type or vice versa; let the caller decide.
                    if segs.len() == 1 {
                        return None;
                    }
                    self.err(self.unresolved_diag(from, first, Ns::Type));
                    return None;
                }
            },
        };
        self.record(first.span, cur);
        let mut i = 1;
        while i < segs.len() {
            let kind = self.p.defs[cur.0 as usize].kind;
            if !matches!(kind, DefKind::Mod | DefKind::Enum) {
                break;
            }
            let seg = &segs[i];
            let ns = if i + 1 == segs.len() { last_ns } else { Ns::Type };
            let other = match ns {
                Ns::Type => Ns::Value,
                Ns::Value | Ns::Macro => Ns::Type,
            };
            // Enum variants live in both namespaces. In a module, a name
            // found only in the other namespace is still returned, so the
            // caller can say "expected a value, found module `std::io`"
            // instead of claiming that `io` does not exist.
            let found = self.lookup_in(cur, &seg.name, ns).or_else(|| self.lookup_in(cur, &seg.name, other));
            let Some(b) = found else {
                if kind == DefKind::Enum {
                    // Possibly an associated function on the enum.
                    break;
                }
                let what = &self.p.defs[cur.0 as usize].name;
                self.err(Diagnostic::error(name::UNRESOLVED, seg.span, format!("cannot find `{}` in `{what}`", seg.name)));
                return None;
            };
            if kind == DefKind::Mod && !b.is_pub && !self.is_ancestor_or_self(cur, from) {
                let d = &self.p.defs[b.def.0 as usize];
                self.err(
                    Diagnostic::error(name::PRIVATE, seg.span, format!("{} `{}` is private", d.kind.describe(), seg.name))
                        .with_secondary(d.span, "defined here")
                        .with_help("mark it `pub` to use it from other modules"),
                );
            }
            cur = b.def;
            self.record(seg.span, cur);
            i += 1;
        }
        Some((ModRes::Def(cur), i))
    }

    /// Remember that the identifier at `span` names `def` (editor tooling).
    pub(crate) fn record(&mut self, span: Span, def: DefId) {
        self.p.refs.push(crate::Ref { span, target: crate::RefTarget::Def(def) });
    }

    pub(crate) fn unresolved_diag(&self, from: DefId, id: &ast::Ident, ns: Ns) -> Diagnostic {
        let what = match ns {
            Ns::Type => "type or module",
            Ns::Value => "value",
            Ns::Macro => "macro",
        };
        let mut d = Diagnostic::error(name::UNRESOLVED, id.span, format!("cannot find {what} `{}` in this scope", id.name));
        if id.name == "crate" {
            // Habit from Rust.
            return d.with_help("Lunar calls the root of the current program or library a crater: write `crater::`");
        }
        if let Some(s) = self.suggest(from, &id.name, ns) {
            d = d.with_help(format!("a similar name exists: `{s}`"));
        }
        d
    }

    /// Edit-distance based suggestion from the module scope: the closest
    /// name, and of equally close names the first in alphabetical order
    /// (the scope is a hash map, whose order changes from run to run).
    fn suggest(&self, from: DefId, name: &str, ns: Ns) -> Option<String> {
        let md = self.mod_data(self.normal_module(from));
        let mut best: Option<(usize, &String)> = None;
        let prelude = self.prelude.map(|p| self.mod_data(p));
        let cands = md.ns(ns).keys().chain(prelude.into_iter().flat_map(|p| p.ns(ns).keys()));
        // Names hidden by macro hygiene (`name#3`) are never suggested.
        for c in cands.filter(|c| !c.contains('#')) {
            let dist = edit_distance(name, c);
            if dist <= (name.len() / 3).max(1) && best.is_none_or(|(b, s)| (dist, c) < (b, s)) {
                best = Some((dist, c));
            }
        }
        best.map(|(_, s)| s.clone())
    }

    // ----- imports -----

    pub(crate) fn resolve_imports(&mut self) {
        let mut pending: Vec<(DefId, Vec<ast::Ident>, &'a ast::UseTree, bool)> = Vec::new();
        for (m, tree, is_pub) in std::mem::take(&mut self.pending_uses) {
            flatten_use(m, Vec::new(), tree, is_pub, &mut pending);
        }
        // Iterate to a fixpoint: imports may depend on other imports.
        loop {
            let mut progress = false;
            let mut still = Vec::new();
            for (m, path, tree, is_pub) in pending {
                if self.try_import(m, &path, tree, is_pub) {
                    progress = true;
                } else {
                    still.push((m, path, tree, is_pub));
                }
            }
            pending = still;
            if !progress || pending.is_empty() {
                break;
            }
        }
        for (m, path, tree, _) in pending {
            // Report through the normal resolver for a good message.
            let segs: Vec<ast::Ident> = path.clone();
            if segs.is_empty() {
                continue;
            }
            match self.resolve_prefix(m, &segs, Ns::Type) {
                Some(_) => {
                    let last = segs.last().unwrap();
                    self.err(Diagnostic::error(name::UNRESOLVED, last.span, format!("unresolved import `{}`", join(&segs))));
                }
                None if segs.len() == 1 => {
                    self.err(Diagnostic::error(name::UNRESOLVED, tree.span, format!("unresolved import `{}`", join(&segs))));
                }
                None => {}
            }
        }
    }

    /// Try to perform one flattened import; false if a prefix is not yet resolvable.
    fn try_import(&mut self, m: DefId, path: &[ast::Ident], tree: &ast::UseTree, is_pub: bool) -> bool {
        let glob = matches!(tree.kind, ast::UseKind::Glob);
        if path.is_empty() {
            return true;
        }
        // Silent resolution attempt, in every namespace the name exists in.
        let target_ty = self.silent_resolve(m, path, Ns::Type);
        if glob {
            let Some(target_ty) = target_ty else { return false };
            let md = self.mod_data_mut(m);
            if !md.globs.iter().any(|(g, _)| *g == target_ty) {
                md.globs.push((target_ty, is_pub));
            }
            return true;
        }
        let v = self.silent_resolve(m, path, Ns::Value);
        let mac = self.silent_resolve(m, path, Ns::Macro);
        if target_ty.is_none() && v.is_none() && mac.is_none() {
            return false;
        }
        self.add_import(m, path, tree, is_pub, [target_ty, v, mac]);
        true
    }

    fn add_import(&mut self, m: DefId, path: &[ast::Ident], tree: &ast::UseTree, is_pub: bool, defs: [Option<DefId>; 3]) {
        let last = path.last().unwrap();
        let (local_name, span) = match &tree.kind {
            ast::UseKind::Simple(Some(rename)) => (rename.name.clone(), rename.span),
            _ => (last.name.clone(), last.span),
        };
        for (ns, def) in [Ns::Type, Ns::Value, Ns::Macro].into_iter().zip(defs) {
            let Some(def) = def else { continue };
            let md = self.mod_data(m);
            if let Some(prev) = md.ns(ns).get(&local_name) {
                if prev.def == def {
                    continue;
                }
                let ps = prev.span;
                self.err(
                    Diagnostic::error(name::DUPLICATE, span, format!("`{local_name}` is imported or defined multiple times"))
                        .with_secondary(ps, "previously bound here"),
                );
                continue;
            }
            self.mod_data_mut(m).ns_mut(ns).insert(local_name.clone(), Binding { def, is_pub, span });
        }
    }

    /// Resolve a full module-level path without reporting errors.
    fn silent_resolve(&self, from: DefId, segs: &[ast::Ident], last_ns: Ns) -> Option<DefId> {
        let first = &segs[0];
        let first_ns = if segs.len() == 1 { last_ns } else { Ns::Type };
        let base = self.path_base(from, first);
        let mut cur = match first.name.as_str() {
            "crater" => self.mod_data(base).krate,
            "self" => self.normal_module(base),
            "super" => self.mod_data(self.normal_module(base)).parent?,
            _ => match self.lookup_ident(from, first, first_ns)? {
                ModRes::Def(d) => d,
                ModRes::Prim(_) => return None,
            },
        };
        for (i, seg) in segs.iter().enumerate().skip(1) {
            let ns = if i + 1 == segs.len() { last_ns } else { Ns::Type };
            let kind = self.p.defs[cur.0 as usize].kind;
            if !matches!(kind, DefKind::Mod | DefKind::Enum) {
                return None;
            }
            let b = self.lookup_in(cur, &seg.name, ns).or_else(|| if kind == DefKind::Enum { self.lookup_in(cur, &seg.name, Ns::Value) } else { None })?;
            cur = b.def;
        }
        // `use a::b` where `b` is a value but we asked for a type: reject.
        let kind = self.p.defs[cur.0 as usize].kind;
        let ok = match last_ns {
            Ns::Type => matches!(kind, DefKind::Mod | DefKind::Struct | DefKind::Enum | DefKind::Trait | DefKind::TypeAlias | DefKind::Variant),
            Ns::Value => matches!(kind, DefKind::Fn | DefKind::Const | DefKind::Static | DefKind::Struct | DefKind::Variant),
            Ns::Macro => kind == DefKind::Macro,
        };
        ok.then_some(cur)
    }
}

fn flatten_use<'a>(
    m: DefId,
    mut prefix: Vec<ast::Ident>,
    tree: &'a ast::UseTree,
    is_pub: bool,
    out: &mut Vec<(DefId, Vec<ast::Ident>, &'a ast::UseTree, bool)>,
) {
    prefix.extend(tree.prefix.iter().cloned());
    match &tree.kind {
        ast::UseKind::Nested(children) => {
            for c in children {
                // `use a::{self}` imports `a` itself.
                if c.prefix.len() == 1 && c.prefix[0].name == "self" && matches!(c.kind, ast::UseKind::Simple(_)) {
                    out.push((m, prefix.clone(), c, is_pub));
                } else {
                    flatten_use(m, prefix.clone(), c, is_pub, out);
                }
            }
        }
        _ => out.push((m, prefix, tree, is_pub)),
    }
}

fn join(segs: &[ast::Ident]) -> String {
    segs.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::")
}

pub(crate) fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev = cur;
    }
    prev[b.len()]
}

#[allow(dead_code)]
fn _span(_: Span) {}
