//! Phase 1: create a [`DefId`] for every item and build module namespaces.

use crate::defs::*;
use crate::loader::{CrateSource, ModSource};
use crate::{AstRef, Sema};
use ln_ast as ast;
use ln_diag::{Diagnostic, Span, codes::name, codes::syntax};

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
                Diagnostic::error(name::DUPLICATE, span, format!("`{name}` is defined multiple times"))
                    .with_secondary(prev_span, "previous definition here"),
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

    fn eval_cfg(&mut self, m: &ast::Meta) -> bool {
        match m {
            ast::Meta::Word(w) => match w.name.as_str() {
                "windows" => self.opts.target_family == "windows",
                "unix" => self.opts.target_family == "unix",
                "test" => self.opts.test,
                _ => false,
            },
            ast::Meta::NameValue(k, v) => match k.name.as_str() {
                "target_os" => self.opts.target_os == *v,
                "target_arch" => self.opts.target_arch == *v,
                "target_family" => self.opts.target_family == *v,
                _ => false,
            },
            ast::Meta::List(k, args) => match k.name.as_str() {
                "not" if args.len() == 1 => !self.eval_cfg(&args[0]),
                "all" => args.iter().all(|a| self.eval_cfg(a)),
                "any" => args.iter().any(|a| self.eval_cfg(a)),
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
                ast::Meta::NameValue(n, _) if n.name == "doc" => {}
                m => {
                    let n = m.name();
                    self.err(Diagnostic::error(syntax::BAD_ATTRIBUTE, a.span, format!("unknown attribute `{}`", n.name)).with_note(
                        "known attributes: cfg, test, inline, lang, intrinsic, link_name, allow, doc",
                    ));
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
        match &item.kind {
            ast::ItemKind::Fn(f) => {
                let id = self.new_def(&f.name.name, DefKind::Fn, Some(module), f.name.span, AstRef::Fn(f, extern_abi.is_some()));
                if attrs.test {
                    if self.opts.test {
                        self.p.tests.push(id);
                    }
                }
                self.finish_def(id, item, attrs);
                self.bind(module, Ns::Value, &f.name.name, id, item.is_pub, f.name.span);
                if let Some(body) = &f.body {
                    self.collect_block_items(module, body, src);
                }
            }
            ast::ItemKind::Struct(s) => {
                let id = self.new_def(&s.name.name, DefKind::Struct, Some(module), s.name.span, AstRef::Struct(s));
                self.finish_def(id, item, attrs);
                self.bind(module, Ns::Type, &s.name.name, id, item.is_pub, s.name.span);
                if !matches!(s.kind, ast::VariantData::Named(_)) {
                    self.bind(module, Ns::Value, &s.name.name, id, item.is_pub, s.name.span);
                }
            }
            ast::ItemKind::Enum(e) => {
                let id = self.new_def(&e.name.name, DefKind::Enum, Some(module), e.name.span, AstRef::Enum(e));
                self.finish_def(id, item, attrs);
                self.bind(module, Ns::Type, &e.name.name, id, item.is_pub, e.name.span);
                let mut variants = Vec::new();
                for (i, v) in e.variants.iter().enumerate() {
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
                self.p.defs[id.0 as usize].data =
                    DefData::Adt(Box::new(AdtData { generics: Generics::default(), variants, is_enum: true }));
            }
            ast::ItemKind::Trait(t) => {
                let id = self.new_def(&t.name.name, DefKind::Trait, Some(module), t.name.span, AstRef::Trait(t));
                self.finish_def(id, item, attrs);
                self.bind(module, Ns::Type, &t.name.name, id, item.is_pub, t.name.span);
                let mut methods = Vec::new();
                for ti in &t.items {
                    if !self.cfg_enabled(&ti.attrs) {
                        continue;
                    }
                    let ast::ItemKind::Fn(f) = &ti.kind else {
                        self.err(Diagnostic::error(syntax::UNEXPECTED_TOKEN, ti.span, "only functions are allowed inside traits"));
                        continue;
                    };
                    let fattrs = self.item_attrs(&ti.attrs);
                    let fid = self.new_def(&f.name.name, DefKind::Fn, Some(module), f.name.span, AstRef::Fn(f, false));
                    self.p.defs[fid.0 as usize].parent = Some(id);
                    self.p.defs[fid.0 as usize].is_pub = true;
                    self.p.defs[fid.0 as usize].attrs = fattrs;
                    if methods.iter().any(|m: &TraitMethod| m.name == f.name.name) {
                        self.err(Diagnostic::error(name::DUPLICATE, f.name.span, format!("method `{}` is defined multiple times", f.name.name)));
                    }
                    methods.push(TraitMethod { name: f.name.name.clone(), def: fid, has_default: f.body.is_some() });
                    if let Some(body) = &f.body {
                        self.collect_block_items(module, body, src);
                    }
                }
                self.p.defs[id.0 as usize].data = DefData::Trait(Box::new(TraitData {
                    generics: Generics::default(),
                    supertraits: Vec::new(),
                    methods,
                    auto: false,
                }));
            }
            ast::ItemKind::Impl(im) => {
                let id = self.new_def("<impl>", DefKind::Impl, Some(module), item.span, AstRef::Impl(im));
                self.finish_def(id, item, attrs);
                self.p.impls.push(id);
                for ii in &im.items {
                    if !self.cfg_enabled(&ii.attrs) {
                        continue;
                    }
                    let ast::ItemKind::Fn(f) = &ii.kind else {
                        self.err(Diagnostic::error(syntax::UNEXPECTED_TOKEN, ii.span, "only functions are allowed inside impls"));
                        continue;
                    };
                    let fattrs = self.item_attrs(&ii.attrs);
                    let fid = self.new_def(&f.name.name, DefKind::Fn, Some(module), f.name.span, AstRef::Fn(f, false));
                    let d = &mut self.p.defs[fid.0 as usize];
                    d.parent = Some(id);
                    d.is_pub = ii.is_pub || im.trait_ref.is_some();
                    d.attrs = fattrs;
                    self.impl_items.entry(id).or_default().push(fid);
                    if let Some(body) = &f.body {
                        self.collect_block_items(module, body, src);
                    }
                }
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
                self.bind(module, Ns::Type, &m.name.name, id, item.is_pub, m.name.span);
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
                self.bind(module, Ns::Value, &c.name.name, id, item.is_pub, c.name.span);
            }
            ast::ItemKind::Static(s) => {
                let id = self.new_def(&s.name.name, DefKind::Static, Some(module), s.name.span, AstRef::Static(s));
                self.finish_def(id, item, attrs);
                self.bind(module, Ns::Value, &s.name.name, id, item.is_pub, s.name.span);
            }
            ast::ItemKind::Extern(block) => {
                let abi = block.abi.as_deref().unwrap_or("C");
                if abi != "C" {
                    self.err(Diagnostic::error(syntax::BAD_ATTRIBUTE, item.span, format!("unsupported ABI `{abi}` (only \"C\" is supported)")));
                }
                for inner in &block.items {
                    match &inner.kind {
                        ast::ItemKind::Fn(f) if f.body.is_none() => self.collect_item(module, inner, src, chain, Some(abi)),
                        _ => self.err(Diagnostic::error(syntax::UNEXPECTED_TOKEN, inner.span, "extern blocks may only contain function declarations without bodies")),
                    }
                }
            }
            ast::ItemKind::TypeAlias(t) => {
                let id = self.new_def(&t.name.name, DefKind::TypeAlias, Some(module), t.name.span, AstRef::Alias(t));
                self.finish_def(id, item, attrs);
                self.bind(module, Ns::Type, &t.name.name, id, item.is_pub, t.name.span);
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
                ast::StmtKind::Let { init: Some(e), .. } | ast::StmtKind::Expr(e) => self.expr(e, mine),
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
