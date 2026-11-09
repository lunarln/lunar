//! A canonical text dump of a parsed file: one node per line, children
//! indented, spans as byte offsets. The parser written in Lunar
//! (`craters/ln-parser`) prints the same format, and
//! `crates/ln-driver/tests/selfhost.rs` compares the two over every program.
//!
//! Node ids and hygiene contexts are left out (they depend on the
//! compilation, not the text), and float literals show only their suffix.

use crate::*;
use std::fmt::Write;

/// The dump of `file`'s items.
pub fn file(f: &File) -> String {
    let mut d = Dump { out: String::new(), depth: 0 };
    for it in &f.items {
        d.item(it);
    }
    d.out
}

/// A string literal's text: quoted, with `\\`, `"`, `\n`, `\r`, `\t` and
/// other control characters (`\u{..}`) escaped.
pub fn escape(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                let _ = write!(out, "\\u{{{:x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

struct Dump {
    out: String,
    depth: usize,
}

fn at(s: Span) -> String {
    format!("@{}..{}", s.start, s.end)
}

fn ident(i: &Ident) -> String {
    format!("{}{}", i.name, at(i.span))
}

fn mode(m: ParamMode) -> &'static str {
    match m {
        ParamMode::Take => "take",
        ParamMode::Borrow => "borrow",
        ParamMode::InOut => "inout",
    }
}

fn delim(d: Delim) -> &'static str {
    match d {
        Delim::Paren => "paren",
        Delim::Bracket => "bracket",
        Delim::Brace => "brace",
    }
}

impl Dump {
    fn line(&mut self, s: impl AsRef<str>) {
        for _ in 0..self.depth {
            self.out.push_str("  ");
        }
        self.out.push_str(s.as_ref());
        self.out.push('\n');
    }

    /// `head`, then whatever `f` prints indented under it.
    fn node(&mut self, head: impl AsRef<str>, f: impl FnOnce(&mut Self)) {
        self.line(head);
        self.depth += 1;
        f(self);
        self.depth -= 1;
    }

    // ===================================================================== items

    fn item(&mut self, it: &Item) {
        let head = format!("item{} {}", if it.is_pub { " pub" } else { "" }, at(it.span));
        self.node(head, |d| {
            for a in &it.attrs {
                d.node(format!("attr {}", at(a.span)), |d| d.meta(&a.meta));
            }
            d.item_kind(&it.kind);
        });
    }

    fn meta(&mut self, m: &Meta) {
        match m {
            Meta::Word(i) => self.line(format!("word {}", ident(i))),
            Meta::NameValue(i, v) => self.line(format!("name_value {} {}", ident(i), escape(v))),
            Meta::List(i, ms) => self.node(format!("list {}", ident(i)), |d| ms.iter().for_each(|m| d.meta(m))),
        }
    }

    fn items(&mut self, items: &[Item]) {
        for it in items {
            self.item(it);
        }
    }

    fn item_kind(&mut self, k: &ItemKind) {
        match k {
            ItemKind::Fn(f) => {
                let head = format!(
                    "fn {}{}{} sig{}",
                    ident(&f.name),
                    if f.is_unsafe { " unsafe" } else { "" },
                    if f.variadic { " variadic" } else { "" },
                    at(f.sig_span)
                );
                self.node(head, |d| {
                    d.generics(&f.generics);
                    for p in &f.params {
                        d.param(p);
                    }
                    if let Some(r) = &f.ret {
                        d.node("ret", |d| d.ty(r));
                    }
                    if let Some(b) = &f.body {
                        d.block(b);
                    }
                });
            }
            ItemKind::Struct(s) => self.node(format!("struct {}", ident(&s.name)), |d| {
                d.generics(&s.generics);
                d.variant_data(&s.kind);
            }),
            ItemKind::Enum(e) => self.node(format!("enum {}", ident(&e.name)), |d| {
                d.generics(&e.generics);
                for v in &e.variants {
                    d.node(format!("variant {} {}", ident(&v.name), at(v.span)), |d| {
                        for a in &v.attrs {
                            d.node(format!("attr {}", at(a.span)), |d| d.meta(&a.meta));
                        }
                        d.variant_data(&v.data);
                    });
                }
            }),
            ItemKind::Trait(t) => self.node(format!("trait {}", ident(&t.name)), |d| {
                d.generics(&t.generics);
                d.node("supertraits", |d| t.supertraits.iter().for_each(|p| d.path(p)));
                d.items(&t.items);
            }),
            ItemKind::Impl(i) => self.node(format!("impl{}", if i.negative { " negative" } else { "" }), |d| {
                d.generics(&i.generics);
                if let Some(t) = &i.trait_ref {
                    d.node("trait", |d| d.path(t));
                }
                d.node("self", |d| d.ty(&i.self_ty));
                d.items(&i.items);
            }),
            ItemKind::Use(u) => self.use_tree(u),
            ItemKind::Mod(m) => match &m.items {
                Some(items) => self.node(format!("mod {}", ident(&m.name)), |d| d.items(items)),
                None => self.line(format!("mod {} external", ident(&m.name))),
            },
            ItemKind::Const(c) => self.node(format!("const {}", ident(&c.name)), |d| {
                d.ty(&c.ty);
                d.expr(&c.value);
            }),
            ItemKind::Static(s) => self.node(format!("static{} {}", if s.is_mut { " mut" } else { "" }, ident(&s.name)), |d| {
                d.ty(&s.ty);
                d.expr(&s.value);
            }),
            ItemKind::Extern(e) => self.node(format!("extern {}", e.abi.as_deref().map_or("-".to_string(), escape)), |d| d.items(&e.items)),
            ItemKind::TypeAlias(t) => self.node(format!("type {}", ident(&t.name)), |d| {
                d.generics(&t.generics);
                d.ty(&t.ty);
            }),
            ItemKind::Macro(m) => self.node(format!("macro {}", ident(&m.name)), |d| {
                for r in &m.rules {
                    d.node(format!("rule {}", at(r.span)), |d| {
                        d.node("matcher", |d| d.tokens(&r.matcher));
                        d.node("body", |d| d.tokens(&r.body));
                    });
                }
            }),
            ItemKind::MacCall(m) => self.mac_call(m),
        }
    }

    fn generics(&mut self, g: &Generics) {
        self.node(format!("generics {}", at(g.span)), |d| {
            for p in &g.params {
                d.node(format!("param {}", ident(&p.name)), |d| p.bounds.iter().for_each(|b| d.path(b)));
            }
        });
    }

    fn param(&mut self, p: &Param) {
        let kind = match &p.kind {
            ParamKind::SelfParam => "self".to_string(),
            ParamKind::Named(i) => ident(i),
            ParamKind::Wild => "_".to_string(),
        };
        let head = format!("param {}{}{} {kind} {}", mode(p.mode), if p.is_mut { " mut" } else { "" }, if p.self_bang { " bang" } else { "" }, at(p.span));
        self.node(head, |d| {
            if let Some(t) = &p.ty {
                d.ty(t);
            }
        });
    }

    fn variant_data(&mut self, v: &VariantData) {
        match v {
            VariantData::Unit => self.line("unit"),
            VariantData::Tuple(fs) => self.node("tuple", |d| {
                for f in fs {
                    d.node(format!("field{}", if f.is_pub { " pub" } else { "" }), |d| d.ty(&f.ty));
                }
            }),
            VariantData::Named(fs) => self.node("named", |d| {
                for f in fs {
                    d.node(format!("field{} {} {}", if f.is_pub { " pub" } else { "" }, ident(&f.name), at(f.span)), |d| {
                        for a in &f.attrs {
                            d.node(format!("attr {}", at(a.span)), |d| d.meta(&a.meta));
                        }
                        d.ty(&f.ty);
                    });
                }
            }),
        }
    }

    fn use_tree(&mut self, u: &UseTree) {
        let prefix = u.prefix.iter().map(ident).collect::<Vec<_>>().join(" ");
        let head = format!("use [{prefix}] {}", at(u.span));
        match &u.kind {
            UseKind::Simple(None) => self.line(format!("{head} simple")),
            UseKind::Simple(Some(r)) => self.line(format!("{head} as {}", ident(r))),
            UseKind::Glob => self.line(format!("{head} glob")),
            UseKind::Nested(ts) => self.node(format!("{head} nested"), |d| ts.iter().for_each(|t| d.use_tree(t))),
        }
    }

    // ===================================================================== macros

    fn mac_call(&mut self, m: &MacCall) {
        self.node(format!("mac_call {} {}", delim(m.delim), at(m.span)), |d| {
            d.path(&m.path);
            d.tokens(&m.tokens);
        });
    }

    fn tokens(&mut self, ts: &[TokenTree]) {
        for t in ts {
            match t {
                TokenTree::Token(tt) => self.line(format!("tt {:?} {}", tt.kind, at(tt.span))),
                TokenTree::Group { delim: dl, open, close, trees } => {
                    self.node(format!("group {} @{}..{}", delim(*dl), open.start, close.end), |d| d.tokens(trees))
                }
            }
        }
    }

    // ===================================================================== paths & types

    fn path(&mut self, p: &Path) {
        self.node(format!("path {}", at(p.span)), |d| {
            for s in &p.segments {
                match &s.args {
                    None => d.line(format!("seg {}", ident(&s.ident))),
                    Some(args) => d.node(format!("seg {} args", ident(&s.ident)), |d| args.iter().for_each(|t| d.ty(t))),
                }
            }
        });
    }

    fn ty(&mut self, t: &Ty) {
        let sp = at(t.span);
        match &t.kind {
            TyKind::Path(p) => self.node(format!("ty path {sp}"), |d| d.path(p)),
            TyKind::Array(e, size) => {
                let s = match size {
                    ArraySize::Fixed(_) => "fixed",
                    ArraySize::Inferred => "inferred",
                    ArraySize::Grow => "grow",
                    ArraySize::GrowShrink => "growshrink",
                    ArraySize::Slice => "slice",
                    ArraySize::SliceMut => "slicemut",
                };
                self.node(format!("ty array {s} {sp}"), |d| {
                    d.ty(e);
                    if let ArraySize::Fixed(n) = size {
                        d.expr(n);
                    }
                });
            }
            TyKind::DynArray(e) => self.node(format!("ty dyn_array {sp}"), |d| d.ty(e)),
            TyKind::Bang(e) => self.node(format!("ty bang {sp}"), |d| d.ty(e)),
            TyKind::Ptr { is_mut, inner } => self.node(format!("ty ptr{} {sp}", if *is_mut { " mut" } else { "" }), |d| d.ty(inner)),
            TyKind::Tuple(ts) => self.node(format!("ty tuple {sp}"), |d| ts.iter().for_each(|t| d.ty(t))),
            TyKind::Fn { params, ret } => self.node(format!("ty fn {sp}"), |d| {
                for (m, t) in params {
                    d.node(format!("fn_param {}", mode(*m)), |d| d.ty(t));
                }
                if let Some(r) = ret {
                    d.node("ret", |d| d.ty(r));
                }
            }),
            TyKind::Never => self.line(format!("ty never {sp}")),
            TyKind::Infer => self.line(format!("ty infer {sp}")),
            TyKind::MacCall(m) => self.node(format!("ty mac_call {sp}"), |d| d.mac_call(m)),
            TyKind::ImplTrait(bs) => self.node(format!("ty impl {sp}"), |d| bs.iter().for_each(|b| d.path(b))),
        }
    }

    // ===================================================================== statements & expressions

    fn block(&mut self, b: &Block) {
        self.node(format!("block {}", at(b.span)), |d| {
            for s in &b.stmts {
                d.stmt(s);
            }
            if let Some(t) = &b.tail {
                d.node("tail", |d| d.expr(t));
            }
        });
    }

    fn stmt(&mut self, s: &Stmt) {
        let sp = at(s.span);
        match &s.kind {
            StmtKind::Let { pat, ty, init } => self.node(format!("stmt let {sp}"), |d| {
                d.pat(pat);
                if let Some(t) = ty {
                    d.node("ty", |d| d.ty(t));
                }
                if let Some(e) = init {
                    d.node("init", |d| d.expr(e));
                }
            }),
            StmtKind::Expr(e) => self.node(format!("stmt expr {sp}"), |d| d.expr(e)),
            StmtKind::Item(it) => self.node(format!("stmt item {sp}"), |d| d.item(it)),
        }
    }

    fn lit(&mut self, l: &Lit) {
        match l {
            Lit::Int { value, suffix } => self.line(format!("int {value} {}", suffix.as_deref().unwrap_or("-"))),
            Lit::Float { suffix, .. } => self.line(format!("float {}", suffix.as_deref().unwrap_or("-"))),
            Lit::Bool(b) => self.line(format!("bool {b}")),
            Lit::Char(c) => self.line(format!("char {}", *c as u32)),
            Lit::Str { value, fmt } => self.node(format!("str {}", escape(value)), |d| match fmt {
                FmtString::Ok(pieces) => {
                    for p in pieces {
                        match p {
                            FmtPiece::Text(t) => d.line(format!("text {}", escape(t))),
                            FmtPiece::Expr(e, spec) => {
                                let s = if *spec == FmtSpec::Debug { "debug" } else { "display" };
                                d.node(format!("hole {s}"), |d| d.expr(e));
                            }
                        }
                    }
                }
                FmtString::Err(msg, sp) => d.line(format!("fmt_err {} {}", escape(msg), at(*sp))),
            }),
        }
    }

    fn opt(&mut self, label: &str, e: &Option<Box<Expr>>) {
        if let Some(e) = e {
            self.node(label, |d| d.expr(e));
        }
    }

    fn expr(&mut self, e: &Expr) {
        let sp = at(e.span);
        match &e.kind {
            ExprKind::Lit(l) => self.node(format!("expr lit {sp}"), |d| d.lit(l)),
            ExprKind::Path(p) => self.node(format!("expr path {sp}"), |d| d.path(p)),
            ExprKind::Unary(op, x) => {
                let o = match op {
                    UnOp::Neg => "neg",
                    UnOp::Not => "not",
                    UnOp::Deref => "deref",
                };
                self.node(format!("expr unary {o} {sp}"), |d| d.expr(x));
            }
            ExprKind::Borrow { mutable, expr } => self.node(format!("expr borrow{} {sp}", if *mutable { " mut" } else { "" }), |d| d.expr(expr)),
            ExprKind::Binary(op, a, b) => self.node(format!("expr binary {} {sp}", op.as_str()), |d| {
                d.expr(a);
                d.expr(b);
            }),
            ExprKind::Assign(a, b) => self.node(format!("expr assign {sp}"), |d| {
                d.expr(a);
                d.expr(b);
            }),
            ExprKind::AssignOp(op, a, b) => self.node(format!("expr assign_op {} {sp}", op.as_str()), |d| {
                d.expr(a);
                d.expr(b);
            }),
            ExprKind::Call(f, args) => self.node(format!("expr call {sp}"), |d| {
                d.expr(f);
                args.iter().for_each(|a| d.expr(a));
            }),
            ExprKind::MethodCall { recv, name, generics, args } => self.node(format!("expr method {} {sp}", ident(name)), |d| {
                d.expr(recv);
                if let Some(g) = generics {
                    d.node("generics", |d| g.iter().for_each(|t| d.ty(t)));
                }
                args.iter().for_each(|a| d.expr(a));
            }),
            ExprKind::Field(x, name) => self.node(format!("expr field {} {sp}", ident(name)), |d| d.expr(x)),
            ExprKind::TupleField(x, i, isp) => self.node(format!("expr tuple_field {i}{} {sp}", at(*isp)), |d| d.expr(x)),
            ExprKind::Index(a, b) => self.node(format!("expr index {sp}"), |d| {
                d.expr(a);
                d.expr(b);
            }),
            ExprKind::Range { start, end, inclusive } => self.node(format!("expr range{} {sp}", if *inclusive { " inclusive" } else { "" }), |d| {
                d.opt("start", start);
                d.opt("end", end);
            }),
            ExprKind::Array(es) => self.node(format!("expr array {sp}"), |d| es.iter().for_each(|x| d.expr(x))),
            ExprKind::ArrayRepeat(v, n) => self.node(format!("expr array_repeat {sp}"), |d| {
                d.expr(v);
                d.expr(n);
            }),
            ExprKind::Tuple(es) => self.node(format!("expr tuple {sp}"), |d| es.iter().for_each(|x| d.expr(x))),
            ExprKind::StructLit { path, fields } => self.node(format!("expr struct {sp}"), |d| {
                d.path(path);
                for f in fields {
                    d.node(format!("field_init {} {}", ident(&f.name), at(f.span)), |d| {
                        if let Some(v) = &f.value {
                            d.expr(v);
                        }
                    });
                }
            }),
            ExprKind::Block(b) => self.node(format!("expr block {sp}"), |d| d.block(b)),
            ExprKind::Unsafe(b) => self.node(format!("expr unsafe {sp}"), |d| d.block(b)),
            ExprKind::If { cond, then, else_ } => self.node(format!("expr if {sp}"), |d| {
                d.expr(cond);
                d.block(then);
                d.opt("else", else_);
            }),
            ExprKind::IfLet { pat, scrut, then, else_ } => self.node(format!("expr if_let {sp}"), |d| {
                d.pat(pat);
                d.expr(scrut);
                d.block(then);
                d.opt("else", else_);
            }),
            ExprKind::Match { scrut, arms } => self.node(format!("expr match {sp}"), |d| {
                d.expr(scrut);
                for a in arms {
                    d.node(format!("arm {}", at(a.span)), |d| {
                        d.pat(&a.pat);
                        if let Some(g) = &a.guard {
                            d.node("guard", |d| d.expr(g));
                        }
                        d.expr(&a.body);
                    });
                }
            }),
            ExprKind::While { cond, body } => self.node(format!("expr while {sp}"), |d| {
                d.expr(cond);
                d.block(body);
            }),
            ExprKind::WhileLet { pat, scrut, body } => self.node(format!("expr while_let {sp}"), |d| {
                d.pat(pat);
                d.expr(scrut);
                d.block(body);
            }),
            ExprKind::Loop(b) => self.node(format!("expr loop {sp}"), |d| d.block(b)),
            ExprKind::For { pat, iter, body } => self.node(format!("expr for {sp}"), |d| {
                d.pat(pat);
                d.expr(iter);
                d.block(body);
            }),
            ExprKind::Break(v) => self.node(format!("expr break {sp}"), |d| d.opt("value", v)),
            ExprKind::Continue => self.line(format!("expr continue {sp}")),
            ExprKind::Return(v) => self.node(format!("expr return {sp}"), |d| d.opt("value", v)),
            ExprKind::Cast(x, t) => self.node(format!("expr cast {sp}"), |d| {
                d.expr(x);
                d.ty(t);
            }),
            ExprKind::Paren(x) => self.node(format!("expr paren {sp}"), |d| d.expr(x)),
            ExprKind::Try(x) => self.node(format!("expr try {sp}"), |d| d.expr(x)),
            ExprKind::MacCall(m) => self.node(format!("expr mac_call {sp}"), |d| d.mac_call(m)),
            ExprKind::Closure(c) => self.node(format!("expr closure{} {sp}", if c.is_move { " move" } else { "" }), |d| {
                for p in &c.params {
                    let name = p.name.as_ref().map_or("_".to_string(), ident);
                    let m = p.mode.map_or("-", mode);
                    d.node(format!("closure_param{} {name} {m} {}", if p.is_mut { " mut" } else { "" }, at(p.span)), |d| {
                        if let Some(t) = &p.ty {
                            d.ty(t);
                        }
                    });
                }
                if let Some(r) = &c.ret {
                    d.node("ret", |d| d.ty(r));
                }
                d.expr(&c.body);
            }),
            ExprKind::Error => self.line(format!("expr error {sp}")),
        }
    }

    // ===================================================================== patterns

    fn pat(&mut self, p: &Pat) {
        let sp = at(p.span);
        match &p.kind {
            PatKind::Wild => self.line(format!("pat wild {sp}")),
            PatKind::Binding { is_mut, name } => self.line(format!("pat binding{} {} {sp}", if *is_mut { " mut" } else { "" }, ident(name))),
            PatKind::Lit(l, neg) => self.node(format!("pat lit{} {sp}", if *neg { " neg" } else { "" }), |d| d.lit(l)),
            PatKind::Range { lo, hi, inclusive } => self.node(format!("pat range{} {sp}", if *inclusive { " inclusive" } else { "" }), |d| {
                d.node(format!("lo{}", if lo.1 { " neg" } else { "" }), |d| d.lit(&lo.0));
                d.node(format!("hi{}", if hi.1 { " neg" } else { "" }), |d| d.lit(&hi.0));
            }),
            PatKind::Tuple(ps) => self.node(format!("pat tuple {sp}"), |d| ps.iter().for_each(|x| d.pat(x))),
            PatKind::TupleStruct { path, pats } => self.node(format!("pat tuple_struct {sp}"), |d| {
                d.path(path);
                pats.iter().for_each(|x| d.pat(x));
            }),
            PatKind::Struct { path, fields, rest } => self.node(format!("pat struct{} {sp}", if *rest { " rest" } else { "" }), |d| {
                d.path(path);
                for f in fields {
                    d.node(format!("field_pat {}", ident(&f.name)), |d| d.pat(&f.pat));
                }
            }),
            PatKind::Path(path) => self.node(format!("pat path {sp}"), |d| d.path(path)),
            PatKind::Or(ps) => self.node(format!("pat or {sp}"), |d| ps.iter().for_each(|x| d.pat(x))),
            PatKind::Rest => self.line(format!("pat rest {sp}")),
            PatKind::MacCall(m) => self.node(format!("pat mac_call {sp}"), |d| d.mac_call(m)),
        }
    }
}
