//! Macro expansion (docs/design/macros.md): matching an invocation's tokens
//! against a macro's rules, transcribing the matching rule's body with
//! hygiene, and parsing the result where the invocation stood.
//!
//! The expanded tokens become the text of a new source file (`<vec!>`), so
//! diagnostics inside an expansion show the expanded code. Each identifier
//! keeps its syntax context through that text (see `ln_parser::Ctxs`).

use crate::defs::*;
use crate::loader::ModSource;
use crate::resolve::ModRes;
use crate::{AstRef, Sema};
use ln_ast::{self as ast, Delim, SyntaxCtx, TokenTree, Tt};
use ln_diag::{Diagnostic, FileId, Span, codes::macro_ as mc};
use ln_lexer::Token as T;
use ln_parser::{Ctxs, Expansion, ExpansionKind, FragmentKind};
use std::cell::RefCell;
use std::collections::HashMap;

/// Expansions nested deeper than this are an error: usually a macro that
/// expands to an invocation of itself.
const RECURSION_LIMIT: usize = 128;

/// Owns syntax made by expansions for as long as the analysis runs, so it
/// can be borrowed like the syntax of the loaded files.
pub(crate) struct Arena<T> {
    items: RefCell<Vec<Box<T>>>,
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self { items: RefCell::new(Vec::new()) }
    }
}

impl<T> Arena<T> {
    pub(crate) fn alloc(&self, value: T) -> &T {
        let b = Box::new(value);
        let p: *const T = &*b;
        self.items.borrow_mut().push(b);
        // SAFETY: the box is never moved out of or dropped before the arena
        // is, and the returned reference cannot outlive `&self`.
        unsafe { &*p }
    }
}

#[derive(Default)]
pub(crate) struct Arenas {
    pub(crate) items: Arena<Vec<ast::Item>>,
    pub(crate) exprs: Arena<ast::Expr>,
    pub(crate) pats: Arena<ast::Pat>,
    pub(crate) tys: Arena<ast::Ty>,
    pub(crate) mods: Arena<ModSource>,
}

/// One expansion, which is one hygiene context.
pub(crate) struct Mark {
    /// The module the macro is defined in: names its body wrote resolve there.
    pub(crate) def_module: DefId,
    /// The syntax context of the macro definition itself (`ROOT` unless an
    /// expansion defined the macro).
    pub(crate) def_ctx: SyntaxCtx,
    /// How many expansions this one is nested in.
    pub(crate) depth: usize,
}

/// An item-position invocation or a derive, waiting until names resolve.
pub(crate) enum Pending<'a> {
    Call { module: DefId, call: &'a ast::MacCall, owner: Owner },
    Derive { module: DefId, item: &'a ast::Item, name: ast::Ident },
}

/// Where the items an item-position macro expands to go.
#[derive(Clone, Copy)]
pub(crate) enum Owner {
    Module,
    Impl(DefId),
    Trait(DefId),
}

// ===================================================================== matchers

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Frag {
    Ident,
    Lit,
    Tt,
    Parsed(FragmentKind),
}

fn frag_kind(name: &str) -> Option<Frag> {
    Some(match name {
        "ident" => Frag::Ident,
        "lit" => Frag::Lit,
        "tt" => Frag::Tt,
        "path" => Frag::Parsed(FragmentKind::Path),
        "expr" => Frag::Parsed(FragmentKind::Expr),
        "ty" => Frag::Parsed(FragmentKind::Ty),
        "pat" => Frag::Parsed(FragmentKind::Pat),
        "block" => Frag::Parsed(FragmentKind::Block),
        "stmt" => Frag::Parsed(FragmentKind::Stmt),
        "item" => Frag::Parsed(FragmentKind::Item),
        "vis" => Frag::Parsed(FragmentKind::Vis),
        "meta" => Frag::Parsed(FragmentKind::Meta),
        "generics" => Frag::Parsed(FragmentKind::Generics),
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Star,
    Plus,
    Question,
}

/// A compiled matcher element.
#[derive(Clone, Debug)]
enum M {
    Tok(Tt),
    Group(Delim, Vec<M>),
    Frag { name: String, kind: Frag },
    Rep { inner: Vec<M>, sep: Option<Tt>, op: Op },
}

/// A rule ready to use: its compiled matcher and its body.
pub(crate) struct Rule<'a> {
    matcher: Vec<M>,
    body: &'a [TokenTree],
}

type MacroError = (ln_diag::Code, Span, String);

fn op_of(t: &TokenTree) -> Option<Op> {
    match t {
        TokenTree::Token(t) => match t.kind {
            T::Star => Some(Op::Star),
            T::Plus => Some(Op::Plus),
            T::Question => Some(Op::Question),
            _ => None,
        },
        _ => None,
    }
}

/// After `$( ... )`: an optional separator token, then `*`, `+` or `?`.
/// `$(..)+*` has the separator `+`; `$(..)+` has none.
fn rep_suffix(trees: &[TokenTree], i: &mut usize, at: Span) -> Result<(Option<Tt>, Op), MacroError> {
    let bad =
        |span: Span| (mc::BAD_MATCHER, span, "a repetition `$( ... )` must end with `*`, `+` or `?`, after an optional separator such as `,`".to_string());
    match (trees.get(*i), trees.get(*i + 1)) {
        (Some(TokenTree::Token(sep)), Some(op)) if op_of(op).is_some() && sep.kind != T::Dollar => {
            let op = op_of(op).unwrap();
            *i += 2;
            Ok((Some(sep.clone()), op))
        }
        (Some(t), _) if op_of(t).is_some() => {
            *i += 1;
            Ok((None, op_of(t).unwrap()))
        }
        (Some(t), _) => Err(bad(t.span())),
        (None, _) => Err(bad(at)),
    }
}

/// Compile a rule's matcher.
fn compile(trees: &[TokenTree]) -> Result<Vec<M>, MacroError> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < trees.len() {
        match &trees[i] {
            TokenTree::Token(t) if t.kind == T::Dollar => match trees.get(i + 1) {
                Some(TokenTree::Token(n)) if n.kind == T::Ident => match (trees.get(i + 2), trees.get(i + 3)) {
                    (Some(TokenTree::Token(c)), Some(TokenTree::Token(k))) if c.kind == T::Colon => {
                        let kind = frag_kind(&k.text).ok_or_else(|| {
                            (
                                mc::BAD_MATCHER,
                                k.span,
                                format!(
                                    "unknown fragment kind `{}`; use one of ident, path, expr, ty, pat, block, stmt, item, lit, vis, meta, tt, generics",
                                    k.text
                                ),
                            )
                        })?;
                        out.push(M::Frag { name: n.text.clone(), kind });
                        i += 4;
                    }
                    _ => return Err((mc::BAD_MATCHER, n.span, format!("`${}` in a matcher needs a fragment kind, as in `${}:expr`", n.text, n.text))),
                },
                Some(TokenTree::Group { delim: Delim::Paren, trees: inner, close, .. }) => {
                    let inner = compile(inner)?;
                    i += 2;
                    let (sep, op) = rep_suffix(trees, &mut i, *close)?;
                    out.push(M::Rep { inner, sep, op });
                }
                _ => return Err((mc::BAD_MATCHER, t.span, "`$` must be followed by a name (`$x:expr`) or a repetition (`$( ... )*`)".into())),
            },
            TokenTree::Token(t) => {
                out.push(M::Tok(t.clone()));
                i += 1;
            }
            TokenTree::Group { delim, trees: inner, .. } => {
                out.push(M::Group(*delim, compile(inner)?));
                i += 1;
            }
        }
    }
    check_follow(&out)?;
    Ok(out)
}

/// Rust's follow-set rule, for the fragments that can grow: what comes
/// right after one must be a token that cannot continue it, so matching
/// never has to guess where it ends.
fn check_follow(ms: &[M]) -> Result<(), MacroError> {
    for w in ms.windows(2) {
        let (M::Frag { name, kind: Frag::Parsed(k) }, M::Tok(next)) = (&w[0], &w[1]) else { continue };
        let ok: &[&str] = match k {
            FragmentKind::Expr | FragmentKind::Stmt => &["=>", ",", ";"],
            FragmentKind::Ty | FragmentKind::Path => &["=>", ",", "=", "|", ";", ":", ">", "[", "{", "as"],
            FragmentKind::Pat => &["=>", ",", "=", "if", "in"],
            _ => continue,
        };
        if !ok.contains(&next.text.as_str()) {
            return Err((
                mc::BAD_MATCHER,
                next.span,
                format!(
                    "`{}` cannot follow `${name}`, which could continue past it; allowed here: {}",
                    next.text,
                    ok.iter().map(|s| format!("`{s}`")).collect::<Vec<_>>().join(", ")
                ),
            ));
        }
    }
    for m in ms {
        match m {
            M::Group(_, inner) | M::Rep { inner, .. } => check_follow(inner)?,
            _ => {}
        }
    }
    Ok(())
}

// ===================================================================== matching

/// What a matcher variable captured.
#[derive(Clone, Debug)]
enum Bound {
    Frag(Fragment),
    /// One per repetition.
    Seq(Vec<Bound>),
}

#[derive(Clone, Debug)]
struct Fragment {
    kind: Frag,
    trees: Vec<TokenTree>,
    /// For `generics`: each parameter's name and bounds, as tokens.
    params: Vec<(Tt, Vec<Vec<Tt>>)>,
}

type Bindings = HashMap<String, Bound>;

fn same_tok(a: &Tt, b: &Tt) -> bool {
    a.kind == b.kind && a.text == b.text
}

fn names_in(ms: &[M], out: &mut Vec<String>) {
    for m in ms {
        match m {
            M::Frag { name, .. } => out.push(name.clone()),
            M::Group(_, inner) | M::Rep { inner, .. } => names_in(inner, out),
            M::Tok(_) => {}
        }
    }
}

/// Every token of `trees`, with group delimiters as tokens.
fn flatten(trees: &[TokenTree], out: &mut Vec<Tt>) {
    for t in trees {
        match t {
            TokenTree::Token(t) => out.push(t.clone()),
            TokenTree::Group { delim, open, close, trees } => {
                let (o, c) = match delim {
                    Delim::Paren => (T::LParen, T::RParen),
                    Delim::Bracket => (T::LBracket, T::RBracket),
                    Delim::Brace => (T::LBrace, T::RBrace),
                };
                out.push(Tt { kind: o, text: delim.open().into(), span: *open, ctx: SyntaxCtx::ROOT });
                flatten(trees, out);
                out.push(Tt { kind: c, text: delim.close().into(), span: *close, ctx: SyntaxCtx::ROOT });
            }
        }
    }
}

fn flat_len(t: &TokenTree) -> usize {
    match t {
        TokenTree::Token(_) => 1,
        TokenTree::Group { trees, .. } => 2 + trees.iter().map(flat_len).sum::<usize>(),
    }
}

/// Tokens as source text: tokens that were adjacent where they were
/// written stay adjacent (so `>` `>` is still a shift), others get a space.
/// Returns the text, each token's offset, and the identifiers' contexts.
fn render(toks: &[Tt]) -> (String, Vec<u32>, Ctxs) {
    let mut text = String::new();
    let mut offsets = Vec::with_capacity(toks.len());
    let mut ctxs = Ctxs::new();
    for (i, t) in toks.iter().enumerate() {
        if i > 0 {
            let p = &toks[i - 1];
            let adjacent = p.span.file == t.span.file && p.span.end == t.span.start && p.ctx == t.ctx && !p.span.is_dummy();
            if !adjacent {
                text.push(' ');
            }
        }
        let start = text.len() as u32;
        offsets.push(start);
        if !t.ctx.is_root() {
            ctxs.insert(start, t.ctx);
        }
        text.push_str(&t.text);
    }
    (text, offsets, ctxs)
}

/// Match a fragment of `kind` at the start of `input`: how many trees it
/// takes, and what it captured.
fn match_frag(kind: Frag, input: &[TokenTree]) -> Option<(usize, Fragment)> {
    let take = |n: usize, params| Some((n, Fragment { kind, trees: input[..n].to_vec(), params }));
    match kind {
        Frag::Tt => input.first().and(take(1, Vec::new())),
        Frag::Ident => match input.first()? {
            TokenTree::Token(t) if t.kind == T::Ident => take(1, Vec::new()),
            _ => None,
        },
        Frag::Lit => match input.first()? {
            TokenTree::Token(t) if matches!(t.kind, T::Number(_) | T::Str | T::Char | T::Byte | T::True | T::False) => take(1, Vec::new()),
            TokenTree::Token(t) if t.kind == T::Minus && matches!(input.get(1), Some(TokenTree::Token(n)) if matches!(n.kind, T::Number(_))) => {
                take(2, Vec::new())
            }
            _ => None,
        },
        Frag::Parsed(k) => {
            let mut flat = Vec::new();
            flatten(input, &mut flat);
            let (text, offsets, ctxs) = render(&flat);
            let (n_flat, generics) = ln_parser::parse_fragment(&text, 0, ctxs, k)?;
            // The fragment must end between trees, never inside a group.
            let mut n_trees = 0;
            let mut seen = 0;
            while seen < n_flat {
                seen += flat_len(input.get(n_trees)?);
                n_trees += 1;
            }
            if seen != n_flat {
                return None;
            }
            let mut params = Vec::new();
            if let Some(g) = generics {
                let index_of = |offset: u32| offsets.partition_point(|&o| o < offset);
                for p in &g.params {
                    let name = flat[index_of(p.name.span.start)].clone();
                    let bounds = p.bounds.iter().map(|b| flat[index_of(b.span.start)..index_of(b.span.end)].to_vec()).collect();
                    params.push((name, bounds));
                }
            }
            take(n_trees, params)
        }
    }
}

/// Match `ms` against `input` from `pos`; the position after the match.
fn match_ms(ms: &[M], input: &[TokenTree], mut pos: usize, b: &mut Bindings) -> Option<usize> {
    for m in ms {
        match m {
            M::Tok(t) => match input.get(pos) {
                Some(TokenTree::Token(x)) if same_tok(x, t) => pos += 1,
                _ => return None,
            },
            M::Group(d, inner) => match input.get(pos) {
                Some(TokenTree::Group { delim, trees, .. }) if delim == d => {
                    if match_ms(inner, trees, 0, b)? != trees.len() {
                        return None;
                    }
                    pos += 1;
                }
                _ => return None,
            },
            M::Frag { name, kind } => {
                let (n, frag) = match_frag(*kind, &input[pos..])?;
                b.insert(name.clone(), Bound::Frag(frag));
                pos += n;
            }
            M::Rep { inner, sep, op } => {
                let mut names = Vec::new();
                names_in(inner, &mut names);
                let mut iters: Vec<Bindings> = Vec::new();
                loop {
                    if *op == Op::Question && !iters.is_empty() {
                        break;
                    }
                    let mut p = pos;
                    if !iters.is_empty()
                        && let Some(s) = sep
                    {
                        match input.get(p) {
                            Some(TokenTree::Token(x)) if same_tok(x, s) => p += 1,
                            _ => break,
                        }
                    }
                    let mut ib = Bindings::new();
                    match match_ms(inner, input, p, &mut ib) {
                        // Each repetition must take something, or it would repeat forever.
                        Some(n) if n > pos => {
                            pos = n;
                            iters.push(ib);
                        }
                        _ => break,
                    }
                }
                if *op == Op::Plus && iters.is_empty() {
                    return None;
                }
                for name in names {
                    let seq = iters.iter().map(|ib| ib.get(&name).cloned().unwrap_or(Bound::Seq(Vec::new()))).collect();
                    b.insert(name, Bound::Seq(seq));
                }
            }
        }
    }
    Some(pos)
}

// ===================================================================== transcription

/// `$name` uses in a body (for the variables a repetition repeats over).
fn vars_in(body: &[TokenTree], out: &mut Vec<String>) {
    for (i, t) in body.iter().enumerate() {
        match t {
            TokenTree::Token(d) if d.kind == T::Dollar => {
                if let Some(TokenTree::Token(n)) = body.get(i + 1) {
                    out.push(n.text.clone());
                }
            }
            TokenTree::Group { trees, .. } => vars_in(trees, out),
            _ => {}
        }
    }
}

/// A token the transcriber makes up (delimiters, `::<`, `,`).
fn made(kind: T, text: &str, span: Span, ctx: SyntaxCtx) -> Tt {
    Tt { kind, text: text.into(), span, ctx }
}

struct Transcriber<'b> {
    mark: SyntaxCtx,
    call: Span,
    out: &'b mut Vec<Tt>,
}

impl Transcriber<'_> {
    fn push_mark(&mut self, t: &Tt) {
        self.out.push(Tt { ctx: self.mark, ..t.clone() });
    }

    fn body(&mut self, body: &[TokenTree], b: &Bindings) -> Result<(), MacroError> {
        let mut i = 0;
        while i < body.len() {
            match &body[i] {
                TokenTree::Token(t) if t.kind == T::Dollar => match body.get(i + 1) {
                    Some(TokenTree::Token(n)) => {
                        let frag = match b.get(&n.text) {
                            Some(Bound::Frag(f)) => f,
                            Some(Bound::Seq(_)) => {
                                return Err((
                                    mc::REPETITION,
                                    n.span,
                                    format!("`${}` was matched inside a repetition, so it can only be used inside `$( ... )`", n.text),
                                ));
                            }
                            None => return Err((mc::BAD_MATCHER, n.span, format!("`${}` is not matched by this rule's matcher", n.text))),
                        };
                        i += 2;
                        // `$g.args`, `$g.bound(Trait)` on a `generics` fragment.
                        let projection = match (body.get(i), body.get(i + 1)) {
                            (Some(TokenTree::Token(dot)), Some(TokenTree::Token(p)))
                                if dot.kind == T::Dot && frag.kind == Frag::Parsed(FragmentKind::Generics) =>
                            {
                                Some(p.text.clone())
                            }
                            _ => None,
                        };
                        match projection.as_deref() {
                            Some("args") => {
                                i += 2;
                                self.generics(frag, None);
                            }
                            Some("bound") => {
                                let Some(TokenTree::Group { delim: Delim::Paren, trees, .. }) = body.get(i + 2) else {
                                    return Err((mc::BAD_MATCHER, n.span, format!("write `${}.bound(Trait)`", n.text)));
                                };
                                i += 3;
                                let mut bound = Vec::new();
                                Transcriber { mark: self.mark, call: self.call, out: &mut bound }.body(trees, b)?;
                                self.generics(frag, Some(bound));
                            }
                            _ => self.fragment(frag),
                        }
                    }
                    Some(TokenTree::Group { delim: Delim::Paren, trees: inner, close, .. }) => {
                        i += 2;
                        let (sep, _) = rep_suffix(body, &mut i, *close)?;
                        let mut names = Vec::new();
                        vars_in(inner, &mut names);
                        let mut count: Option<(usize, String)> = None;
                        for n in &names {
                            if let Some(Bound::Seq(v)) = b.get(n) {
                                match &count {
                                    None => count = Some((v.len(), n.clone())),
                                    Some((c, first)) if *c != v.len() => {
                                        return Err((
                                            mc::REPETITION,
                                            *close,
                                            format!("`${first}` repeats {c} times but `${n}` repeats {} times in the same repetition", v.len()),
                                        ));
                                    }
                                    _ => {}
                                }
                            }
                        }
                        let Some((count, _)) = count else {
                            return Err((mc::REPETITION, *close, "this repetition uses no variable that was matched in a repetition".into()));
                        };
                        for k in 0..count {
                            if k > 0
                                && let Some(s) = &sep
                            {
                                self.push_mark(s);
                            }
                            let mut ib = b.clone();
                            for n in &names {
                                if let Some(Bound::Seq(v)) = b.get(n) {
                                    ib.insert(n.clone(), v[k].clone());
                                }
                            }
                            self.body(inner, &ib)?;
                        }
                    }
                    _ => return Err((mc::BAD_MATCHER, t.span, "`$` must be followed by a matched name or a repetition `$( ... )`".into())),
                },
                TokenTree::Token(t) => {
                    self.push_mark(t);
                    i += 1;
                }
                TokenTree::Group { delim, open, close, trees } => {
                    let (o, c) = match delim {
                        Delim::Paren => (T::LParen, T::RParen),
                        Delim::Bracket => (T::LBracket, T::RBracket),
                        Delim::Brace => (T::LBrace, T::RBrace),
                    };
                    self.out.push(made(o, delim.open(), *open, self.mark));
                    self.body(trees, b)?;
                    self.out.push(made(c, delim.close(), *close, self.mark));
                    i += 1;
                }
            }
        }
        Ok(())
    }

    /// A captured fragment, with the caller's contexts. An expression is
    /// parenthesized so it stays one operand: `$x * 2` with `1 + 1`.
    fn fragment(&mut self, f: &Fragment) {
        let mut flat = Vec::new();
        flatten(&f.trees, &mut flat);
        let wrap = f.kind == Frag::Parsed(FragmentKind::Expr) && flat.len() > 1;
        if wrap {
            self.out.push(made(T::LParen, "(", self.call, self.mark));
        }
        self.out.extend(flat);
        if wrap {
            self.out.push(made(T::RParen, ")", self.call, self.mark));
        }
    }

    /// `$g.args` (`::<T, U>`) or `$g.bound(B)` (`::<T: A + B, U: B>`).
    fn generics(&mut self, f: &Fragment, bound: Option<Vec<Tt>>) {
        if f.params.is_empty() {
            return;
        }
        let (call, mark) = (self.call, self.mark);
        self.out.push(made(T::PathSep, "::", call, mark));
        self.out.push(made(T::LAngle, "<", call, mark));
        for (i, (name, bounds)) in f.params.iter().enumerate() {
            if i > 0 {
                self.out.push(made(T::Comma, ",", call, mark));
            }
            self.out.push(name.clone());
            if let Some(extra) = &bound {
                self.out.push(made(T::Colon, ":", call, mark));
                for b in bounds {
                    self.out.extend(b.iter().cloned());
                    self.out.push(made(T::Plus, "+", call, mark));
                }
                self.out.extend(extra.iter().cloned());
            }
        }
        self.out.push(made(T::RAngle, ">", call, mark));
    }
}

// ===================================================================== expansion

impl<'a> Sema<'a> {
    /// Compile a macro's rules when it is defined, reporting bad matchers.
    pub(crate) fn define_macro(&mut self, def: DefId, m: &'a ast::MacroDef) {
        let mut rules = Vec::new();
        for r in &m.rules {
            match compile(&r.matcher) {
                Ok(matcher) => rules.push(Rule { matcher, body: &r.body }),
                Err((code, span, msg)) => self.err(Diagnostic::error(code, span, msg)),
            }
        }
        self.macro_rules.insert(def, rules);
    }

    /// The macro an invocation's path (or a `#[derive]`) names, or an error.
    fn resolve_macro(&mut self, module: DefId, path: &ast::Path, derive: bool) -> Option<DefId> {
        let segs: Vec<ast::Ident> = path.segments.iter().map(|s| s.ident.clone()).collect();
        let shown = path.to_string_lossy();
        let missing = if derive { format!("cannot find derive macro `{shown}` in this scope") } else { format!("cannot find macro `{shown}!` in this scope") };
        match self.resolve_prefix(module, &segs, Ns::Macro) {
            Some((ModRes::Def(d), n)) if n == segs.len() && self.p.def(d).kind == DefKind::Macro => Some(d),
            Some((ModRes::Def(d), n)) if n == segs.len() => {
                let kind = self.p.def(d).kind.describe();
                self.err(Diagnostic::error(mc::UNDEFINED, path.span, format!("`{shown}` is a {kind}, not a macro")));
                None
            }
            Some(_) => {
                self.err(Diagnostic::error(mc::UNDEFINED, path.span, missing));
                None
            }
            None => {
                if segs.len() == 1 {
                    self.err(Diagnostic::error(mc::UNDEFINED, path.span, missing));
                }
                None
            }
        }
    }

    /// Expand `call` in module `module`, parsing the result as `kind`.
    pub(crate) fn expand_call(&mut self, module: DefId, call: &ast::MacCall, kind: ExpansionKind) -> Option<Expansion> {
        let def = self.resolve_macro(module, &call.path, false)?;
        if self.p.def(def).attrs.derive_macro {
            let name = &self.p.def(def).name;
            self.err(
                Diagnostic::error(mc::UNDEFINED, call.path.span, format!("`{name}` is a derive macro, used as `#[derive({name})]` on a struct or enum"))
                    .with_help("only macros without `#[derive_macro]` can be invoked with `!`"),
            );
            return None;
        }
        let parent = call.path.segments[0].ident.ctx;
        self.expand_with(def, &call.tokens, parent, call.span, kind, false)
    }

    /// Run a derive macro on `item`, whose tokens are its input.
    pub(crate) fn expand_derive(&mut self, module: DefId, item: &'a ast::Item, name: &ast::Ident) -> Option<Vec<ast::Item>> {
        let path = ast::Path { segments: vec![ast::PathSegment { ident: name.clone(), args: None }], span: name.span, id: ast::NodeId(0) };
        let def = self.resolve_macro(module, &path, true)?;
        if !self.p.def(def).attrs.derive_macro {
            self.err(
                Diagnostic::error(mc::UNDEFINED, name.span, format!("`{}` is not a derive macro", name.name))
                    .with_help("a derive is a `macro` marked `#[derive_macro]`; ordinary macros are invoked with `name!(...)`"),
            );
            return None;
        }
        let input = self.derive_input(item);
        match self.expand_with(def, &input, name.ctx, name.span, ExpansionKind::Items, true)? {
            Expansion::Items(items) => Some(items),
            _ => None,
        }
    }

    /// An item's tokens, without its `#[derive(...)]` attributes.
    fn derive_input(&self, item: &ast::Item) -> Vec<TokenTree> {
        let file = self.map.get(item.span.file);
        let text = &file.text[item.span.start as usize..item.span.end as usize];
        let empty = Ctxs::new();
        let ctxs = self.file_ctxs.get(&item.span.file).unwrap_or(&empty);
        let trees = ln_parser::source_token_trees(text, item.span.start, item.span.file, ctxs);
        let mut out = Vec::new();
        let mut i = 0;
        while i < trees.len() {
            let is_derive = matches!(&trees[i], TokenTree::Token(t) if t.kind == T::Hash)
                && matches!(trees.get(i + 1), Some(TokenTree::Group { delim: Delim::Bracket, trees: inner, .. })
                    if matches!(inner.first(), Some(TokenTree::Token(t)) if t.text == "derive"));
            if is_derive {
                i += 2;
                continue;
            }
            out.push(trees[i].clone());
            i += 1;
        }
        out
    }

    fn expand_with(&mut self, def: DefId, input: &[TokenTree], parent: SyntaxCtx, call: Span, kind: ExpansionKind, derive: bool) -> Option<Expansion> {
        let name = self.p.def(def).name.clone();
        let depth = if parent.is_root() { 0 } else { self.marks[parent.0 as usize].depth } + 1;
        if depth > RECURSION_LIMIT {
            self.err(
                Diagnostic::error(mc::RECURSION_LIMIT, call, format!("expanding `{name}!` nests more than {RECURSION_LIMIT} expansions deep"))
                    .with_help("a macro that expands to an invocation of itself needs a rule that stops"),
            );
            return None;
        }
        let rules = self.macro_rules.get(&def)?;
        let n = rules.len();
        let mut found = None;
        for r in rules {
            let mut b = Bindings::new();
            if match_ms(&r.matcher, input, 0, &mut b) == Some(input.len()) {
                found = Some((r.body, b));
                break;
            }
        }
        let Some((body, bindings)) = found else {
            self.err(
                Diagnostic::error(mc::NO_MATCH, call, format!("no rule of `{name}!` matches these tokens"))
                    .with_note(format!("`{name}!` has {n} rule{}; each is tried in order", if n == 1 { "" } else { "s" })),
            );
            return None;
        };
        let AstRef::Macro(m) = self.asts[def.0 as usize] else { return None };
        let mark = SyntaxCtx(self.marks.len() as u32);
        self.marks.push(Mark { def_module: self.p.def(def).module, def_ctx: m.name.ctx, depth });
        let mut toks = Vec::new();
        if kind == ExpansionKind::Expr {
            toks.push(made(T::LBrace, "{", Span::DUMMY, mark));
        }
        if let Err((code, span, msg)) = (Transcriber { mark, call, out: &mut toks }).body(body, &bindings) {
            self.err(Diagnostic::error(code, span, msg).with_secondary(call, format!("in this expansion of `{name}!`")));
            return None;
        }
        if kind == ExpansionKind::Expr {
            toks.push(made(T::RBrace, "}", Span::DUMMY, mark));
        }
        let (text, _, ctxs) = render(&toks);
        let file: FileId = self.map.add(format!("<{name}!>"), None, text.clone());
        self.file_ctxs.insert(file, ctxs.clone());
        self.p.expansion_sites.insert(file, call);
        let (out, diags) = ln_parser::parse_expansion(&text, file, ctxs, &mut self.ids, kind);
        if derive && !diags.is_empty() {
            // The parse errors would only restate this from inside the
            // derive's output.
            self.err(
                Diagnostic::error(mc::DERIVE_OUTPUT, call, format!("the derive `{name}` expanded to something that is not a list of items"))
                    .with_note("a derive can only add items (usually an `impl`) next to the struct or enum it is on")
                    .with_secondary(Span::new(file, 0, text.len() as u32), "what it expanded to"),
            );
            return None;
        }
        for d in diags {
            self.err(d.with_secondary(call, format!("in this expansion of `{name}!`")));
        }
        out
    }

    /// Expand item-position invocations and derives until none are left,
    /// collecting what they define and resolving the imports they add.
    pub(crate) fn expand_items(&mut self) {
        for _ in 0..RECURSION_LIMIT {
            let pending = std::mem::take(&mut self.pending_macros);
            if pending.is_empty() {
                return;
            }
            for p in pending {
                let (module, owner, items, span) = match p {
                    Pending::Call { module, call, owner } => match self.expand_call(module, call, ExpansionKind::Items) {
                        Some(Expansion::Items(items)) => (module, owner, items, call.span),
                        _ => continue,
                    },
                    Pending::Derive { module, item, name } => match self.expand_derive(module, item, &name) {
                        Some(items) => (module, Owner::Module, items, name.span),
                        None => continue,
                    },
                };
                let arenas: &'a Arenas = self.arenas;
                let items: &'a Vec<ast::Item> = arenas.items.alloc(items);
                let file = items.first().map_or(span.file, |i| i.span.file);
                let src: &'a ModSource = arenas.mods.alloc(ModSource { file, ast: ast::File { items: Vec::new(), span }, children: HashMap::new() });
                match owner {
                    Owner::Module => {
                        for item in items {
                            self.collect_expanded_item(module, item, src);
                        }
                    }
                    Owner::Impl(id) => self.collect_impl_items(id, module, items, src),
                    Owner::Trait(id) => self.collect_trait_items(id, module, items, src),
                }
            }
            self.resolve_imports();
        }
    }

    /// Expand a macro in expression or statement position.
    pub(crate) fn expand_expr(&mut self, module: DefId, call: &ast::MacCall) -> Option<&'a ast::Expr> {
        let Expansion::Expr(e) = self.expand_call(module, call, ExpansionKind::Expr)? else { return None };
        if let ast::ExprKind::Block(b) = &e.kind
            && let Some(s) = b.stmts.iter().find(|s| matches!(s.kind, ast::StmtKind::Item(_)))
        {
            self.err(
                Diagnostic::error(mc::NOT_SUPPORTED, s.span, "items in a macro expanded in expression position are not supported yet")
                    .with_secondary(call.span, "expanded here")
                    .with_help("define the items outside the macro, or invoke a macro in item position (outside any function) to define them"),
            );
            return None;
        }
        let arenas: &'a Arenas = self.arenas;
        Some(arenas.exprs.alloc(e))
    }

    pub(crate) fn expand_pat(&mut self, module: DefId, call: &ast::MacCall) -> Option<&'a ast::Pat> {
        let Expansion::Pat(p) = self.expand_call(module, call, ExpansionKind::Pat)? else { return None };
        let arenas: &'a Arenas = self.arenas;
        Some(arenas.pats.alloc(p))
    }

    /// Expand a macro in type position. Types are lowered more than once,
    /// so each invocation is expanded once and remembered.
    pub(crate) fn expand_ty(&mut self, module: DefId, call: &ast::MacCall) -> Option<&'a ast::Ty> {
        let key = (call.span.file, call.span.start);
        if let Some(t) = self.ty_expansions.get(&key) {
            return *t;
        }
        let arenas: &'a Arenas = self.arenas;
        let t = match self.expand_call(module, call, ExpansionKind::Ty) {
            Some(Expansion::Ty(t)) => Some(arenas.tys.alloc(t)),
            _ => None,
        };
        self.ty_expansions.insert(key, t);
        t
    }
}
