//! Macro syntax: token trees, `macro` definitions, `path!(...)` invocations,
//! and parsing what a macro expands to (see docs/design/macros.md).

use crate::{IdGen, P, lex_with_offset};
use ln_ast::*;
use ln_diag::{Diagnostic, FileId};
use ln_lexer::Token as T;
use ln_parse_kit::{Expected, PResult, Tok};
use std::collections::HashMap;

/// The syntax context of the identifier starting at each offset of an
/// expansion's text (offsets not listed are `ROOT`).
pub type Ctxs = HashMap<u32, SyntaxCtx>;

impl P<'_, '_, '_> {
    /// A `!` directly followed by a delimiter: after a path, a macro call.
    pub(crate) fn at_mac_bang(&mut self) -> bool {
        self.k.at(T::Bang) && matches!(self.k.nth(1), Some(T::LParen | T::LBracket | T::LBrace))
    }

    /// The rest of `path!(...)`, after the path.
    pub(crate) fn mac_call(&mut self, path: Path) -> PResult<MacCall> {
        let start = path.span.start;
        self.k.expect(T::Bang)?;
        let (delim, _, _, tokens) = self.delimited()?;
        Ok(MacCall { path, delim, tokens, span: self.span_from(start) })
    }

    /// A delimited group of token trees; the current token opens it.
    pub(crate) fn delimited(&mut self) -> PResult<(Delim, Span, Span, Vec<TokenTree>)> {
        let (delim, close) = match self.k.peek() {
            Some(T::LParen) => (Delim::Paren, T::RParen),
            Some(T::LBracket) => (Delim::Bracket, T::RBracket),
            Some(T::LBrace) => (Delim::Brace, T::RBrace),
            _ => return Err(self.k.error_expected_named("`(`, `[` or `{`")),
        };
        let open = self.k.bump();
        let mut trees = Vec::new();
        loop {
            match self.k.peek() {
                Some(k) if k == close => {
                    let c = self.k.bump();
                    return Ok((delim, self.tok_span(open), self.tok_span(c), trees));
                }
                // A closer that does not match, or the end of the input.
                None | Some(T::RParen | T::RBracket | T::RBrace) => {
                    self.k.expect(close)?;
                    unreachable!("expect fails on a different token");
                }
                _ => trees.push(self.token_tree()?),
            }
        }
    }

    fn token_tree(&mut self) -> PResult<TokenTree> {
        if matches!(self.k.peek(), Some(T::LParen | T::LBracket | T::LBrace)) {
            let (delim, open, close, trees) = self.delimited()?;
            return Ok(TokenTree::Group { delim, open, close, trees });
        }
        let t = self.k.bump();
        Ok(TokenTree::Token(self.tt(t)))
    }

    /// A statement without its trailing `;` (the `stmt` fragment).
    fn stmt_no_semi(&mut self) -> PResult<()> {
        if self.k.eat(T::Let) {
            self.pat()?;
            if self.k.eat(T::Colon) {
                self.ty()?;
            }
            if self.k.eat(T::Eq) {
                self.expr()?;
            }
            return Ok(());
        }
        if self.at_item_start() {
            self.item()?;
            return Ok(());
        }
        self.expr()?;
        Ok(())
    }

    fn tt(&self, t: Tok<T>) -> Tt {
        Tt { kind: t.kind, text: self.tok_text(t).to_string(), span: self.tok_span(t), ctx: self.ctx_at(t.start) }
    }

    /// `macro name(matcher) { body }`, or several rules:
    /// `macro name { (matcher) => { body }, ... }`.
    pub(crate) fn macro_def(&mut self) -> PResult<MacroDef> {
        self.k.expect(T::Macro)?;
        let name = self.ident()?;
        let mut rules = Vec::new();
        if self.k.at(T::LParen) {
            let start = self.start();
            let (_, _, _, matcher) = self.delimited()?;
            if !self.k.at(T::LBrace) {
                self.k.expect_note(Expected::Token(T::LBrace));
                return Err(self.k.error_expected());
            }
            let (_, _, _, body) = self.delimited()?;
            rules.push(MacroRule { matcher, body, span: self.span_from(start) });
            return Ok(MacroDef { name, rules });
        }
        self.k.expect(T::LBrace)?;
        loop {
            if self.k.eat(T::RBrace) {
                break;
            }
            let start = self.start();
            let (_, _, _, matcher) = self.delimited()?;
            self.k.expect(T::FatArrow)?;
            let (_, _, _, body) = self.delimited()?;
            rules.push(MacroRule { matcher, body, span: self.span_from(start) });
            if !self.k.eat(T::Comma) && !self.k.eat(T::Semi) {
                self.k.expect(T::RBrace)?;
                break;
            }
        }
        Ok(MacroDef { name, rules })
    }
}

/// What the text of a macro expansion is parsed as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpansionKind {
    /// Item position: zero or more items.
    Items,
    /// Expression or statement position: a block (the expander wraps the
    /// tokens in braces), so expansions may contain statements.
    Expr,
    Pat,
    Ty,
}

pub enum Expansion {
    Items(Vec<Item>),
    Expr(Expr),
    Pat(Pat),
    Ty(Ty),
}

/// Parse the text a macro expanded to, registered as `file`.
pub fn parse_expansion(src: &str, file: FileId, ctxs: Ctxs, ids: &mut IdGen, kind: ExpansionKind) -> (Option<Expansion>, Vec<Diagnostic>) {
    let (toks, docs, mut diags) = lex_with_offset(src, 0, file);
    let mut p = P::new(src, 0, &toks, docs, file, ids, src.len() as u32);
    p.ctxs = ctxs;
    let out = match kind {
        ExpansionKind::Items => Some(Expansion::Items(p.items_until_eof())),
        ExpansionKind::Expr => p.expr().ok().map(Expansion::Expr),
        ExpansionKind::Pat => p.pat().ok().map(Expansion::Pat),
        ExpansionKind::Ty => p.ty().ok().map(Expansion::Ty),
    };
    if out.is_some() && !p.k.at_eof() {
        p.k.expect_note(Expected::Named("the end of the macro expansion"));
        let _ = p.k.error_expected();
    }
    diags.extend(p.finish());
    (out, diags)
}

/// The token trees of `src`, a piece of `file` starting at offset `base`,
/// for a derive macro's input. Doc comments become `#[doc = "..."]`
/// attributes; `ctxs` gives the hygiene contexts if `file` is an expansion.
pub fn source_token_trees(src: &str, base: u32, file: FileId, ctxs: &Ctxs) -> Vec<TokenTree> {
    let (toks, _) = ln_lexer::lex(src);
    let mut stack: Vec<(Delim, Span, Vec<TokenTree>)> = Vec::new();
    let mut cur: Vec<TokenTree> = Vec::new();
    for t in toks {
        let span = Span::new(file, base + t.range.start as u32, base + t.range.end as u32);
        let text = &src[t.range.clone()];
        let tok = |kind: T, text: &str| TokenTree::Token(Tt { kind, text: text.to_string(), span, ctx: ctxs.get(&span.start).copied().unwrap_or_default() });
        match t.token {
            T::LParen => stack.push((Delim::Paren, span, std::mem::take(&mut cur))),
            T::LBracket => stack.push((Delim::Bracket, span, std::mem::take(&mut cur))),
            T::LBrace => stack.push((Delim::Brace, span, std::mem::take(&mut cur))),
            T::RParen | T::RBracket | T::RBrace => {
                if let Some((delim, open, outer)) = stack.pop() {
                    let trees = std::mem::replace(&mut cur, outer);
                    cur.push(TokenTree::Group { delim, open, close: span, trees });
                }
            }
            T::DocComment => {
                let doc = text.strip_prefix("///").unwrap_or(text).trim_end_matches('\r');
                let doc = doc.strip_prefix(' ').unwrap_or(doc);
                let lit = format!("\"{}\"", doc.replace('\\', "\\\\").replace('"', "\\\""));
                cur.push(tok(T::Hash, "#"));
                cur.push(TokenTree::Group {
                    delim: Delim::Bracket,
                    open: span,
                    close: span,
                    trees: vec![tok(T::Ident, "doc"), tok(T::Eq, "="), tok(T::Str, &lit)],
                });
            }
            kind => cur.push(tok(kind, text)),
        }
    }
    cur
}

/// A syntactic fragment a macro matcher can capture (`$x:expr`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FragmentKind {
    Path,
    Expr,
    Ty,
    Pat,
    Block,
    Stmt,
    Item,
    Vis,
    Meta,
    Generics,
}

/// Parse one fragment from the start of `src` (a macro's input tokens, as
/// text). Returns how many tokens it took, and for `generics` the parsed
/// parameters (with spans into `src`), or `None` if `src` does not start
/// with such a fragment.
pub fn parse_fragment(src: &str, file: FileId, ctxs: Ctxs, kind: FragmentKind) -> Option<(usize, Option<Generics>)> {
    let mut ids = IdGen::new();
    let (toks, docs, lex_diags) = lex_with_offset(src, 0, file);
    if !lex_diags.is_empty() {
        return None;
    }
    let mut p = P::new(src, 0, &toks, docs, file, &mut ids, src.len() as u32);
    p.ctxs = ctxs;
    let mut generics = None;
    let ok = match kind {
        FragmentKind::Path => p.path_ty_style().is_ok(),
        FragmentKind::Expr => p.expr().is_ok(),
        FragmentKind::Ty => p.ty().is_ok(),
        FragmentKind::Pat => p.pat().is_ok(),
        FragmentKind::Block => p.block().is_ok(),
        FragmentKind::Stmt => p.stmt_no_semi().is_ok(),
        FragmentKind::Item => p.item().is_ok(),
        FragmentKind::Vis => {
            p.k.eat(T::Pub);
            true
        }
        FragmentKind::Meta => p.meta_full().is_ok(),
        FragmentKind::Generics => match p.generics_decl() {
            Ok(g) => {
                generics = Some(g);
                true
            }
            Err(_) => false,
        },
    };
    let n = p.k.position();
    (ok && !p.k.has_errors() && p.diags.is_empty()).then_some((n, generics))
}
