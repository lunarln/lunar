//! Phase 1: create a [`DefId`] for every item and build module namespaces.

use crate::defs::*;
use crate::loader::{CrateSource, ModSource};
use crate::{AstRef, Sema};
use ln_ast as ast;
use ln_diag::{Diagnostic, Span, codes::lint, codes::name, codes::syntax};

/// Values of `target_os` the compiler can be run for or with (the values
/// of Rust's `std::env::consts::OS` for hosts LLVM supports).
const KNOWN_OS: &[&str] = &["linux", "macos", "windows", "ios", "android", "freebsd", "netbsd", "openbsd", "dragonfly", "solaris", "illumos"];

/// Common `target_arch` values, only used to suggest `target_arch = "..."`.
const KNOWN_ARCH: &[&str] = &["x86_64", "x86", "aarch64", "arm", "riscv64", "wasm32"];

impl<'a> Sema<'a> {
    pub(crate) fn new_def(&mut self, name: &str, kind: DefKind, module: Option<DefId>, span: Span, ast: AstRef<'a>) -> DefId {
        let id = DefId(self.p.defs.len() as u32);
        let module = module.unwrap_or(id);
        let data = match kind {
            DefKind::Mod => DefData::Mod(ModData::default()),
            _ => DefData::None,
        };
        self.p.defs.push(DefInfo { name: name.to_string(), kind, module, parent: None, is_pub: false, span, attrs: ItemAttrs::default(), data });
        self.asts.push(ast);
        id
    }

    pub(crate) fn mod_data(&self, m: DefId) -> &ModData {
        match &self.p.defs[m.0 as usize].data {
            DefData::Mod(d) => d,
            _ => panic!("not a module: {:?}", self.p.defs[m.0 as usize].name),
        }
    }

    pub(crate) fn mod_data_mut(&mut self, m: DefId) -> &mut ModData {
        match &mut self.p.defs[m.0 as usize].data {
            DefData::Mod(d) => d,
            _ => panic!("not a module"),
        }
    }

    fn bind(&mut self, module: DefId, ns: Ns, name: &str, def: DefId, is_pub: bool, span: Span) {
        if let Some(prev) = self.mod_data(module).ns(ns).get(name) {
            let prev_span = prev.span;
            self.err(
                Diagnostic::error(name::DUPLICATE, span, format!("`{name}` is defined multiple times")).with_secondary(prev_span, "previous definition here"),
            );
            return;
        }
        self.mod_data_mut(module).ns_mut(ns).insert(name.to_string(), Binding { def, is_pub, span });
    }

    pub(crate) fn collect_crate(&mut self, c: &'a CrateSource) -> DefId {
        let root = self.new_def(&c.name, DefKind::Mod, None, Span::new(c.root.file, 0, 0), AstRef::None);
        self.mod_data_mut(root).krate = root;
        self.p.defs[root.0 as usize].is_pub = true;
        self.p.crate_roots.push((c.name.clone(), root));
        self.collect_items(root, &c.root.ast.items, &c.root, &mut Vec::new());
        root
    }

    // ----- attributes -----

    /// Evaluate `#[cfg(...)]` attributes; `false` means the item is compiled out.
    pub(crate) fn cfg_enabled(&mut self, attrs: &[ast::Attr]) -> bool {
        attrs.iter().all(|a| match &a.meta {
            ast::Meta::List(n, args) if n.name == "cfg" => {
                if args.len() != 1 {
                    self.err(Diagnostic::error(syntax::BAD_ATTRIBUTE, a.span, "`cfg` takes exactly one predicate"));
                    return true;
                }
                self.eval_cfg(&args[0])
            }
            _ => true,
        })
    }

    /// `lint::unknown_cfg`: a cfg name or value the compiler never sets, so
    /// the predicate is always false (often a typo or a Rust-ism such as
    /// `#[cfg(linux)]`).
    fn unknown_cfg(&mut self, span: Span, what: String, help: Option<String>) {
        let mut d = Diagnostic::warning(lint::UNKNOWN_CFG, span, format!("unknown cfg {what}"))
            .with_label("this is never set, so the condition is always false")
            .with_note("valid cfg options are `unix`, `windows`, `test`, `target_os = \"...\"`, `target_family = \"...\"` and `target_arch = \"...\"`, combined with `not(...)`, `all(...)` and `any(...)`");
        if let Some(h) = help {
            d = d.with_help(h);
        }
        self.err(d);
    }

    fn eval_cfg(&mut self, m: &ast::Meta) -> bool {
        match m {
            ast::Meta::Word(w) => match w.name.as_str() {
                "windows" => self.opts.target_family == "windows",
                "unix" => self.opts.target_family == "unix",
                "test" => self.opts.test,
                n => {
                    let help = if KNOWN_OS.contains(&n) {
                        Some(format!("to test the operating system, write `target_os = \"{n}\"`"))
                    } else if KNOWN_ARCH.contains(&n) {
                        Some(format!("to test the architecture, write `target_arch = \"{n}\"`"))
                    } else if matches!(n, "target_os" | "target_family" | "target_arch") {
                        Some(format!("`{n}` needs a value, as in `{n} = \"...\"`"))
                    } else {
                        None
                    };
                    self.unknown_cfg(w.span, format!("name `{n}`"), help);
                    false
                }
            },
            ast::Meta::NameValue(k, v) => match k.name.as_str() {
                "target_os" => {
                    if !KNOWN_OS.contains(&v.as_str()) {
                        let help = format!("known operating systems: {}", KNOWN_OS.iter().map(|o| format!("`{o}`")).collect::<Vec<_>>().join(", "));
                        self.unknown_cfg(k.span, format!("value `target_os = \"{v}\"`"), Some(help));
                    }
                    self.opts.target_os == *v
                }
                "target_arch" => self.opts.target_arch == *v,
                "target_family" => {
                    if !matches!(v.as_str(), "unix" | "windows") {
                        self.unknown_cfg(k.span, format!("value `target_family = \"{v}\"`"), Some("the families are `\"unix\"` and `\"windows\"`".into()));
                    }
                    self.opts.target_family == *v
                }
                n => {
                    let help = matches!(n, "unix" | "windows" | "test").then(|| format!("`{n}` takes no value: write `{n}`"));
                    self.unknown_cfg(k.span, format!("name `{n}`"), help);
                    false
                }
            },
            ast::Meta::List(k, args) => match k.name.as_str() {
                "not" if args.len() == 1 => !self.eval_cfg(&args[0]),
                // Evaluate every argument (no short-circuit) so each unknown
                // name is reported.
                "all" => args.iter().map(|a| self.eval_cfg(a)).fold(true, |acc, x| acc && x),
                "any" => args.iter().map(|a| self.eval_cfg(a)).fold(false, |acc, x| acc || x),
                _ => {
                    self.err(Diagnostic::error(syntax::BAD_ATTRIBUTE, k.span, format!("unknown cfg predicate `{}`", k.name)));
                    false
                }
            },
        }
    }

    fn item_attrs(&mut self, attrs: &[ast::Attr]) -> ItemAttrs {
        let mut out = ItemAttrs::default();
        for a in attrs {
            match &a.meta {
                ast::Meta::List(n, _) if n.name == "cfg" => {}
                ast::Meta::List(n, _) if n.name == "allow" => {}
                ast::Meta::NameValue(n, v) if n.name == "lang" => out.lang = Some(v.clone()),
                ast::Meta::NameValue(n, v) if n.name == "link_name" => out.link_name = Some(v.clone()),
                ast::Meta::Word(n) if n.name == "intrinsic" => out.intrinsic = true,
                ast::Meta::Word(n) if n.name == "test" => out.test = true,
                ast::Meta::Word(n) if n.name == "inline" => out.inline = true,
                ast::Meta::Word(n) if n.name == "derive_macro" => out.derive_macro = true,
                ast::Meta::Word(n) if n.name == "export" => {
                    if n.ctx.is_root() {
                        self.err(
                            Diagnostic::error(syntax::BAD_ATTRIBUTE, a.span, "`#[export]` only applies to items written in a macro's body")
                                .with_note("it makes an item a macro defines visible where the macro is invoked; items written by hand are visible anyway"),
                        );
                    }
                    out.export = true;
                }
                ast::Meta::List(n, list) if n.name == "derive" => {
                    for m in list {
                        match m {
                            ast::Meta::Word(d) => out.derive.push(d.clone()),
                            _ => self.err(Diagnostic::error(syntax::BAD_ATTRIBUTE, a.span, "write derives by name: `#[derive(Eq, Debug)]`")),
                        }
                    }
                }
                ast::Meta::NameValue(n, _) if n.name == "doc" => {}
                m => {
                    let n = m.name();
                    // A known attribute written in the wrong form gets the
                    // right form, not "unknown attribute".
                    let usage = match n.name.as_str() {
                        "test" => Some("#[test]"),
                        "inline" => Some("#[inline]"),
                        "intrinsic" => Some("#[intrinsic]"),
                        "lang" => Some("#[lang = \"name\"]"),
                        "link_name" => Some("#[link_name = \"symbol\"]"),
                        "doc" => Some("#[doc = \"text\"] (or a `///` comment)"),
                        "cfg" => Some("#[cfg(predicate)]"),
                        "allow" => Some("#[allow(lint_name)]"),
                        "derive" => Some("#[derive(Trait, ...)]"),
                        "derive_macro" => Some("#[derive_macro]"),
                        "export" => Some("#[export]"),
                        _ => None,
                    };
                    let d = match usage {
                        Some(u) => Diagnostic::error(syntax::BAD_ATTRIBUTE, a.span, format!("malformed `{}` attribute", n.name))
                            .with_help(format!("write it as `{u}`")),
                        None => Diagnostic::error(syntax::BAD_ATTRIBUTE, a.span, format!("unknown attribute `{}`", n.name))
                            .with_note("known attributes: cfg, test, inline, lang, intrinsic, link_name, allow, doc, derive, derive_macro, export"),
                    };
                    self.err(d);
                }
            }
        }
        out
    }

    // ----- items -----

    fn collect_items(&mut self, module: DefId, items: &'a [ast::Item], src: &'a ModSource, chain: &mut Vec<String>) {
        for item in items {
            self.collect_item(module, item, src, chain, None);
        }
    }

    fn finish_def(&mut self, id: DefId, item: &ast::Item, attrs: ItemAttrs) {
        let d = &mut self.p.defs[id.0 as usize];
        d.is_pub = item.is_pub;
        if let Some(lang) = &attrs.lang {
            if let Some(prev) = self.p.lang.insert(lang.clone(), id) {
                let _ = prev;
            }
        }
        self.p.defs[id.0 as usize].attrs = attrs;
    }

    fn collect_item(&mut self, module: DefId, item: &'a ast::Item, src: &'a ModSource, chain: &mut Vec<String>, extern_abi: Option<&str>) {
        if !self.cfg_enabled(&item.attrs) {
            return;
        }
        let attrs = self.item_attrs(&item.attrs);
        // Hygiene: an item a macro's body names is hidden from other code
        // unless it is `#[export]`ed (docs/design/macros.md, section 4).
        let export = attrs.export;
        let key = |id: &ast::Ident| if id.ctx.is_root() || export { id.name.clone() } else { crate::resolve::hidden_name(&id.name, id.ctx) };
        for d in &attrs.derive {
            if !matches!(item.kind, ast::ItemKind::Struct(_) | ast::ItemKind::Enum(_)) {
                self.err(Diagnostic::error(syntax::BAD_ATTRIBUTE, d.span, "`#[derive]` applies only to structs and enums"));
                break;
            }
            self.pending_macros.push(crate::expand::Pending::Derive { module, item, name: d.clone() });
        }
        match &item.kind {
            ast::ItemKind::Fn(f) => {
                let id = self.new_def(&f.name.name, DefKind::Fn, Some(module), f.name.span, AstRef::Fn(f, extern_abi.is_some()));
                // Only the crater being tested contributes tests, not the
                // craters it depends on.
                let in_root = self.p.crate_roots.first().is_some_and(|(_, r)| *r == self.mod_data(module).krate);
                if attrs.test && self.opts.test && in_root {
                    self.p.tests.push(id);
                }
                self.finish_def(id, item, attrs);
                self.bind(module, Ns::Value, &key(&f.name), id, item.is_pub, f.name.span);
                if let Some(body) = &f.body {
                    self.collect_block_items(module, body, src);
                }
            }
            ast::ItemKind::Struct(s) => {
                let id = self.new_def(&s.name.name, DefKind::Struct, Some(module), s.name.span, AstRef::Struct(s));
                self.finish_def(id, item, attrs);
                self.bind(module, Ns::Type, &key(&s.name), id, item.is_pub, s.name.span);
                if !matches!(s.kind, ast::VariantData::Named(_)) {
                    self.bind(module, Ns::Value, &key(&s.name), id, item.is_pub, s.name.span);
                }
            }
            ast::ItemKind::Enum(e) => {
                let id = self.new_def(&e.name.name, DefKind::Enum, Some(module), e.name.span, AstRef::Enum(e));
                self.finish_def(id, item, attrs);
                self.bind(module, Ns::Type, &key(&e.name), id, item.is_pub, e.name.span);
                let mut variants = Vec::new();
                for (i, v) in e.variants.iter().enumerate() {
                    for a in &v.attrs {
                        let ok =
                            matches!(&a.meta, ast::Meta::NameValue(n, _) if n.name == "doc") || matches!(&a.meta, ast::Meta::Word(n) if n.name == "default");
                        if !ok {
                            self.err(
                                Diagnostic::error(syntax::BAD_ATTRIBUTE, a.span, format!("unknown attribute `{}` on an enum variant", a.meta.name().name))
                                    .with_note("variants take doc comments and `#[default]` (used by `#[derive(Default)]`)"),
                            );
                        }
                    }
                    let vid = self.new_def(&v.name.name, DefKind::Variant, Some(module), v.name.span, AstRef::Variant(v));
                    let vd = &mut self.p.defs[vid.0 as usize];
                    vd.parent = Some(id);
                    vd.is_pub = true;
                    vd.data = DefData::Variant { enum_id: id, index: i };
                    let ctor = match v.data {
                        ast::VariantData::Unit => CtorKind::Unit,
                        ast::VariantData::Tuple(_) => CtorKind::Tuple,
                        ast::VariantData::Named(_) => CtorKind::Named,
                    };
                    if variants.iter().any(|x: &VariantDef| x.name == v.name.name) {
                        self.err(Diagnostic::error(name::DUPLICATE, v.name.span, format!("variant `{}` is defined multiple times", v.name.name)));
                    }
                    variants.push(VariantDef { name: v.name.name.clone(), def: vid, ctor, fields: Vec::new(), span: v.span });
                }
                self.p.defs[id.0 as usize].data = DefData::Adt(Box::new(AdtData { generics: Generics::default(), variants, is_enum: true }));
            }
            ast::ItemKind::Trait(t) => {
                let id = self.new_def(&t.name.name, DefKind::Trait, Some(module), t.name.span, AstRef::Trait(t));
                self.finish_def(id, item, attrs);
                self.bind(module, Ns::Type, &key(&t.name), id, item.is_pub, t.name.span);
                self.p.defs[id.0 as usize].data =
                    DefData::Trait(Box::new(TraitData { generics: Generics::default(), supertraits: Vec::new(), methods: Vec::new(), auto: false }));
                self.collect_trait_items(id, module, &t.items, src);
            }
            ast::ItemKind::Impl(im) => {
                let id = self.new_def("<impl>", DefKind::Impl, Some(module), item.span, AstRef::Impl(im));
                self.finish_def(id, item, attrs);
                self.p.impls.push(id);
                self.collect_impl_items(id, module, &im.items, src);
            }
            ast::ItemKind::Macro(m) => {
                let id = self.new_def(&m.name.name, DefKind::Macro, Some(module), m.name.span, AstRef::Macro(m));
                self.finish_def(id, item, attrs);
                self.bind(module, Ns::Macro, &key(&m.name), id, item.is_pub, m.name.span);
                self.define_macro(id, m);
            }
            ast::ItemKind::MacCall(call) => {
                self.pending_macros.push(crate::expand::Pending::Call { module, call, owner: crate::expand::Owner::Module });
            }
            ast::ItemKind::Use(tree) => self.pending_uses.push((module, tree, item.is_pub)),
            ast::ItemKind::Mod(m) => {
                let id = self.new_def(&m.name.name, DefKind::Mod, Some(module), m.name.span, AstRef::None);
                let krate = self.mod_data(module).krate;
                {
                    let md = self.mod_data_mut(id);
                    md.parent = Some(module);
                    md.krate = krate;
                }
                self.finish_def(id, item, attrs);
                self.bind(module, Ns::Type, &key(&m.name), id, item.is_pub, m.name.span);
                match &m.items {
                    Some(items) => {
                        chain.push(m.name.name.clone());
                        self.collect_items(id, items, src, chain);
                        chain.pop();
                    }
                    None => {
                        let mut key = chain.clone();
                        key.push(m.name.name.clone());
                        if let Some(child) = src.children.get(&key) {
                            self.collect_items(id, &child.ast.items, child, &mut Vec::new());
                        }
                    }
                }
            }
            ast::ItemKind::Const(c) => {
                let id = self.new_def(&c.name.name, DefKind::Const, Some(module), c.name.span, AstRef::Const(c));
                self.finish_def(id, item, attrs);
                self.bind(module, Ns::Value, &key(&c.name), id, item.is_pub, c.name.span);
            }
            ast::ItemKind::Static(s) => {
                let id = self.new_def(&s.name.name, DefKind::Static, Some(module), s.name.span, AstRef::Static(s));
                self.finish_def(id, item, attrs);
                self.bind(module, Ns::Value, &key(&s.name), id, item.is_pub, s.name.span);
            }
            ast::ItemKind::Extern(block) => {
                let abi = block.abi.as_deref().unwrap_or("C");
                if abi != "C" {
                    self.err(Diagnostic::error(syntax::BAD_ATTRIBUTE, item.span, format!("unsupported ABI `{abi}` (only \"C\" is supported)")));
                }
                for inner in &block.items {
                    match &inner.kind {
                        ast::ItemKind::Fn(f) if f.body.is_none() => self.collect_item(module, inner, src, chain, Some(abi)),
                        _ => self.err(Diagnostic::error(
                            syntax::UNEXPECTED_TOKEN,
                            inner.span,
                            "extern blocks may only contain function declarations without bodies",
                        )),
                    }
                }
            }
            ast::ItemKind::TypeAlias(t) => {
                let id = self.new_def(&t.name.name, DefKind::TypeAlias, Some(module), t.name.span, AstRef::Alias(t));
                self.finish_def(id, item, attrs);
                self.bind(module, Ns::Type, &key(&t.name), id, item.is_pub, t.name.span);
            }
        }
    }

    /// An item a macro expanded to, in item position of `module`.
    pub(crate) fn collect_expanded_item(&mut self, module: DefId, item: &'a ast::Item, src: &'a ModSource) {
        self.collect_item(module, item, src, &mut Vec::new(), None);
    }

    /// The functions of trait `id` (also those a macro in its body expands to).
    pub(crate) fn collect_trait_items(&mut self, id: DefId, module: DefId, items: &'a [ast::Item], src: &'a ModSource) {
        for ti in items {
            if !self.cfg_enabled(&ti.attrs) {
                continue;
            }
            let f = match &ti.kind {
                ast::ItemKind::Fn(f) => f,
                ast::ItemKind::MacCall(call) => {
                    self.pending_macros.push(crate::expand::Pending::Call { module, call, owner: crate::expand::Owner::Trait(id) });
                    continue;
                }
                _ => {
                    self.err(Diagnostic::error(syntax::UNEXPECTED_TOKEN, ti.span, "only functions are allowed inside traits"));
                    continue;
                }
            };
            let fattrs = self.item_attrs(&ti.attrs);
            let fid = self.new_def(&f.name.name, DefKind::Fn, Some(module), f.name.span, AstRef::Fn(f, false));
            self.p.defs[fid.0 as usize].parent = Some(id);
            self.p.defs[fid.0 as usize].is_pub = true;
            self.p.defs[fid.0 as usize].attrs = fattrs;
            let DefData::Trait(t) = &mut self.p.defs[id.0 as usize].data else { unreachable!("not a trait") };
            let dup = t.methods.iter().any(|m| m.name == f.name.name);
            t.methods.push(TraitMethod { name: f.name.name.clone(), def: fid, has_default: f.body.is_some() });
            if dup {
                self.err(Diagnostic::error(name::DUPLICATE, f.name.span, format!("method `{}` is defined multiple times", f.name.name)));
            }
            if let Some(body) = &f.body {
                self.collect_block_items(module, body, src);
            }
        }
    }

    /// The functions of impl `id` (also those a macro in its body expands to).
    pub(crate) fn collect_impl_items(&mut self, id: DefId, module: DefId, items: &'a [ast::Item], src: &'a ModSource) {
        let is_trait_impl = matches!(self.asts[id.0 as usize], AstRef::Impl(im) if im.trait_ref.is_some());
        for ii in items {
            if !self.cfg_enabled(&ii.attrs) {
                continue;
            }
            let f = match &ii.kind {
                ast::ItemKind::Fn(f) => f,
                ast::ItemKind::MacCall(call) => {
                    self.pending_macros.push(crate::expand::Pending::Call { module, call, owner: crate::expand::Owner::Impl(id) });
                    continue;
                }
                _ => {
                    self.err(Diagnostic::error(syntax::UNEXPECTED_TOKEN, ii.span, "only functions are allowed inside impls"));
                    continue;
                }
            };
            let fattrs = self.item_attrs(&ii.attrs);
            let fid = self.new_def(&f.name.name, DefKind::Fn, Some(module), f.name.span, AstRef::Fn(f, false));
            let d = &mut self.p.defs[fid.0 as usize];
            d.parent = Some(id);
            d.is_pub = ii.is_pub || is_trait_impl;
            d.attrs = fattrs;
            self.impl_items.entry(id).or_default().push(fid);
            if let Some(body) = &f.body {
                self.collect_block_items(module, body, src);
            }
        }
    }

    /// Items declared inside function bodies live in anonymous block modules.
    fn collect_block_items(&mut self, module: DefId, block: &'a ast::Block, src: &'a ModSource) {
        let mut v = BlockFinder { blocks: Vec::new() };
        v.block(block);
        // Blocks are found outermost-first, so parents exist before children.
        let mut scope_of: Vec<(ast::NodeId, DefId)> = Vec::new();
        for (b, parent_block) in v.blocks {
            let parent = parent_block.and_then(|pb| scope_of.iter().find(|(id, _)| *id == pb).map(|x| x.1)).unwrap_or(module);
            let anon = self.new_def("<block>", DefKind::Mod, Some(parent), b.span, AstRef::None);
            let krate = self.mod_data(module).krate;
            {
                let md = self.mod_data_mut(anon);
                md.parent = Some(parent);
                md.krate = krate;
                md.is_block = true;
            }
            self.block_scopes.insert(b.id, anon);
            scope_of.push((b.id, anon));
            for s in &b.stmts {
                if let ast::StmtKind::Item(item) = &s.kind {
                    self.collect_item(anon, item, src, &mut Vec::new(), None);
                }
            }
        }
    }
}

/// Finds blocks that directly contain item statements, outermost first, with
/// the nearest enclosing such block.
struct BlockFinder<'a> {
    blocks: Vec<(&'a ast::Block, Option<ast::NodeId>)>,
}

impl<'a> BlockFinder<'a> {
    fn block(&mut self, b: &'a ast::Block) {
        self.block_in(b, None);
    }

    fn block_in(&mut self, b: &'a ast::Block, parent: Option<ast::NodeId>) {
        let has_items = b.stmts.iter().any(|s| matches!(s.kind, ast::StmtKind::Item(_)));
        let mine = if has_items {
            self.blocks.push((b, parent));
            Some(b.id)
        } else {
            parent
        };
        for s in &b.stmts {
            match &s.kind {
                ast::StmtKind::Let { init: Some(e), else_, .. } => {
                    self.expr(e, mine);
                    if let Some(b) = else_ {
                        self.block_in(b, mine);
                    }
                }
                ast::StmtKind::Expr(e) => self.expr(e, mine),
                _ => {}
            }
        }
        if let Some(t) = &b.tail {
            self.expr(t, mine);
        }
    }

    fn expr(&mut self, e: &'a ast::Expr, p: Option<ast::NodeId>) {
        use ast::ExprKind::*;
        match &e.kind {
            Block(b) | Unsafe(b) | Loop(b) => self.block_in(b, p),
            If { cond, then, else_ } => {
                self.expr(cond, p);
                self.block_in(then, p);
                if let Some(e) = else_ {
                    self.expr(e, p);
                }
            }
            IfLet { scrut, then, else_, .. } => {
                self.expr(scrut, p);
                self.block_in(then, p);
                if let Some(e) = else_ {
                    self.expr(e, p);
                }
            }
            While { cond, body } => {
                self.expr(cond, p);
                self.block_in(body, p);
            }
            WhileLet { scrut, body, .. } => {
                self.expr(scrut, p);
                self.block_in(body, p);
            }
            For { iter, body, .. } => {
                self.expr(iter, p);
                self.block_in(body, p);
            }
            Match { scrut, arms } => {
                self.expr(scrut, p);
                for a in arms {
                    self.expr(&a.body, p);
                }
            }
            Unary(_, x) | Paren(x) | Try(x) | Cast(x, _) | Field(x, _) | TupleField(x, _, _) => self.expr(x, p),
            Binary(_, a, b) | Assign(a, b) | AssignOp(_, a, b) | Index(a, b) | ArrayRepeat(a, b) => {
                self.expr(a, p);
                self.expr(b, p);
            }
            Call(f, args) => {
                self.expr(f, p);
                args.iter().for_each(|a| self.expr(a, p));
            }
            MethodCall { recv, args, .. } => {
                self.expr(recv, p);
                args.iter().for_each(|a| self.expr(a, p));
            }
            Array(es) | Tuple(es) => es.iter().for_each(|a| self.expr(a, p)),
            StructLit { fields, .. } => fields.iter().filter_map(|f| f.value.as_ref()).for_each(|a| self.expr(a, p)),
            Return(Some(x)) | Break(Some(x)) => self.expr(x, p),
            Range { start, end, .. } => {
                if let Some(s) = start {
                    self.expr(s, p);
                }
                if let Some(s) = end {
                    self.expr(s, p);
                }
            }
            _ => {}
        }
    }
}
