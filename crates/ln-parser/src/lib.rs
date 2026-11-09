//! Parser for the Lunar programming language.
//!
//! Consumes tokens from `ln-lexer` and builds an `ln-ast` tree using the
//! generic cursor from `ln-parse-kit`.

mod expr;
mod item;
mod lit;
mod pat;
mod ty;

use ln_ast::*;
use ln_diag::{Diagnostic, FileId, codes::syntax};
use ln_lexer::Token;
use ln_parse_kit::{Expected, PResult, ParseErrorKind, Tok};

pub use lit::{parse_float_lit, parse_int_lit};

/// Hands out node ids that are unique across every file of a compilation.
#[derive(Default, Debug)]
pub struct IdGen {
    next: u32,
}

impl IdGen {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn next(&mut self) -> NodeId {
        let id = NodeId(self.next);
        self.next += 1;
        id
    }

    pub fn count(&self) -> u32 {
        self.next
    }
}

/// Parse a complete source file.
pub fn parse_file(src: &str, file: FileId, ids: &mut IdGen) -> (File, Vec<Diagnostic>) {
    // A half-merged file would only produce a cascade of syntax errors.
    let conflicts = ln_lexer::conflict_markers(src);
    if !conflicts.is_empty() {
        let diags = conflicts.iter().map(|c| conflict_diag(src, file, c)).collect();
        return (File { items: Vec::new(), span: Span::new(file, 0, src.len() as u32) }, diags);
    }
    let (toks, docs, diags) = lex_with_offset(src, 0, file);
    let mut p = P::new(src, 0, &toks, docs, file, ids, src.len() as u32);
    p.diags = diags;
    let items = p.items_until_eof();
    let file_ast = File { items, span: Span::new(file, 0, src.len() as u32) };
    let diags = p.finish();
    (file_ast, diags)
}

/// Parse a single expression from `src`, whose first byte sits at `offset` in
/// file `file`. Used for format-string interpolation holes.
pub(crate) fn parse_expr_at(src: &str, offset: u32, file: FileId, ids: &mut IdGen) -> Result<Expr, Vec<Diagnostic>> {
    let (toks, docs, mut diags) = lex_with_offset(src, offset, file);
    let eof = offset + src.len() as u32;
    let mut p = P::new(src, offset, &toks, docs, file, ids, eof);
    let e = p.expr();
    if e.is_ok() && !p.k.at_eof() {
        let _ = p.k.error_expected();
    }
    diags.extend(p.finish());
    match e {
        Ok(e) if diags.is_empty() => Ok(e),
        _ => Err(diags),
    }
}

/// `err(syntax::conflict_marker)` for one conflict region.
fn conflict_diag(src: &str, file: FileId, c: &ln_lexer::Conflict) -> Diagnostic {
    let span = |r: &std::ops::Range<usize>| Span::new(file, r.start as u32, (r.start + 7).min(r.end) as u32);
    // The text after the marker names the side, e.g. `HEAD` or a branch.
    let side = |r: &std::ops::Range<usize>| {
        let t = src[r.start + 7.min(r.len())..r.end].trim();
        if t.is_empty() { String::new() } else { format!(" (`{t}`)") }
    };
    let kind = &src[c.start.start..c.start.start + 1];
    if kind != "<" {
        return Diagnostic::error(syntax::CONFLICT_MARKER, span(&c.start), "encountered a version control conflict marker")
            .with_label("conflict marker without a matching `<<<<<<<`")
            .with_help("finish resolving the conflict, then delete every line that holds a conflict marker");
    }
    let mut d = Diagnostic::error(syntax::CONFLICT_MARKER, span(&c.start), "encountered a version control conflict marker");
    let ours = side(&c.start);
    match (&c.base, &c.mid) {
        (Some(_), _) => d = d.with_label(format!("between this marker and `|||||||` is our code{ours}")),
        (None, Some(_)) => d = d.with_label(format!("between this marker and `=======` is our code{ours}")),
        (None, None) => d = d.with_label(format!("conflict region starts here{ours}, with no `=======` or `>>>>>>>` after it")),
    }
    if let Some(b) = &c.base {
        d = d.with_secondary(span(b), format!("between this marker and `=======` is the original code both sides changed{}", side(b)));
    }
    if let Some(m) = &c.mid {
        let theirs = c.end.as_ref().map(side).unwrap_or_default();
        d = d.with_secondary(span(m), format!("between this marker and `>>>>>>>` is the incoming code{theirs}"));
    }
    if let Some(e) = &c.end {
        d = d.with_secondary(span(e), "this marker concludes the conflict region");
    }
    d.with_note("conflict markers indicate that a merge was started but could not be completed due to merge conflicts")
        .with_note("if you are in a merge, the top section is the code you already had checked out and the bottom section is the new code")
        .with_note("if you are in a rebase, the top section is the code being rebased onto and the bottom section is the code you had checked out which is being rebased")
        .with_help("keep the code you want from each section, then delete the lines that hold conflict markers")
}

/// The lexer's `Char` token only matches well-formed literals, so `'ab'`
/// and `''` come out as stray quotes around other tokens. Turn a `'` that
/// is closed later on the same line back into one `Char` token, so the
/// parser reports `syntax::bad_char` once for the whole literal. A `'`
/// that is never closed stays an error (`syntax::unterminated`).
fn rejoin_char_literals(src: &str, toks: &mut Vec<ln_lexer::Spanned>, errs: &mut Vec<ln_lexer::LexError>) {
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < errs.len() {
        let start = errs[i].range.start;
        let mut end = None;
        let mut j = start + 1;
        while bytes.get(start) == Some(&b'\'') && j < bytes.len() {
            match bytes[j] {
                b'\n' => break,
                b'\\' => j += 2,
                b'\'' => {
                    end = Some(j + 1);
                    break;
                }
                _ => j += 1,
            }
        }
        let Some(end) = end else {
            i += 1;
            continue;
        };
        let (start, token) = with_byte_prefix(start, toks);
        toks.retain(|t| t.range.end <= start || t.range.start >= end);
        // An error inside the literal may also have swallowed text after it
        // (the lexer consumed `';` trying to match a literal): lex that
        // text again.
        let mut leftover = Vec::new();
        errs.retain(|e| {
            let inside = e.range.start >= start && e.range.start < end;
            if inside && e.range.end > end {
                leftover.push(end..e.range.end);
            }
            !inside
        });
        toks.push(ln_lexer::Spanned { token, range: start..end });
        for r in leftover {
            let (more, more_errs) = ln_lexer::lex(&src[r.clone()]);
            toks.extend(more.into_iter().map(|t| ln_lexer::Spanned { token: t.token, range: t.range.start + r.start..t.range.end + r.start }));
            errs.extend(more_errs.into_iter().map(|e| ln_lexer::LexError { range: e.range.start + r.start..e.range.end + r.start, kind: e.kind }));
        }
        toks.sort_by_key(|t| t.range.start);
        errs.sort_by_key(|e| e.range.start);
    }
}

/// An unclosed character literal (`'a;`) is already reported by the lexer.
/// Put a `Char` token in its place, covering the rest of the word, so the
/// parser sees a complete expression instead of reporting a second error.
fn placeholder_for_unclosed_chars(src: &str, toks: &mut Vec<ln_lexer::Spanned>, errs: &[ln_lexer::LexError]) {
    let bytes = src.as_bytes();
    for e in errs {
        if e.kind != ln_lexer::LexErrorKind::Invalid || bytes.get(e.range.start) != Some(&b'\'') {
            continue;
        }
        let start = e.range.start;
        let mut end = start + 1;
        while end < bytes.len() && !matches!(bytes[end], b' ' | b'\t' | b'\r' | b'\n' | b';' | b',' | b')' | b']' | b'}' | b'\'') {
            end += 1;
        }
        let (start, token) = with_byte_prefix(start, toks);
        toks.retain(|t| t.range.end <= start || t.range.start >= end);
        toks.push(ln_lexer::Spanned { token, range: start..end });
    }
    toks.sort_by_key(|t| t.range.start);
}

/// A malformed literal starting with the quote at `quote`: a byte literal
/// (`b'ab'`) when the quote directly follows an identifier `b`, else a
/// character literal. Returns where the literal starts and its token.
fn with_byte_prefix(quote: usize, toks: &[ln_lexer::Spanned]) -> (usize, Token) {
    let prefixed = quote > 0 && toks.iter().any(|t| t.token == Token::Ident && t.range == (quote - 1..quote));
    if prefixed { (quote - 1, Token::Byte) } else { (quote, Token::Char) }
}

/// Doc comments, by the start of the token that follows them (where an item
/// that they document would start).
pub(crate) type Docs = std::collections::HashMap<u32, Vec<(u32, u32)>>;

fn lex_with_offset(src: &str, offset: u32, file: FileId) -> (Vec<Tok<Token>>, Docs, Vec<Diagnostic>) {
    let (mut toks, mut errs) = ln_lexer::lex(src);
    rejoin_char_literals(src, &mut toks, &mut errs);
    placeholder_for_unclosed_chars(src, &mut toks, &errs);
    // Doc comments leave the token stream; `P::attrs` attaches them to the
    // item they precede, and anywhere else they are plain comments.
    let mut docs = Docs::new();
    let mut pending = Vec::new();
    let mut out = Vec::with_capacity(toks.len());
    for s in toks {
        let (start, end) = (s.range.start as u32 + offset, s.range.end as u32 + offset);
        if s.token == Token::DocComment {
            pending.push((start, end));
            continue;
        }
        if !pending.is_empty() {
            docs.insert(start, std::mem::take(&mut pending));
        }
        out.push(Tok::new(s.token, start, end));
    }
    let toks = out;
    let diags = errs
        .into_iter()
        .map(|e| {
            let span = Span::new(file, e.range.start as u32 + offset, e.range.end as u32 + offset);
            let text = &src[e.range.clone()];
            if e.kind == ln_lexer::LexErrorKind::UnterminatedComment {
                let open = Span::new(file, span.start, span.start + 2);
                return Diagnostic::error(syntax::UNTERMINATED, open, "unterminated block comment")
                    .with_label("this comment is never closed")
                    .with_note("block comments nest: every `/*` inside a comment needs its own `*/`")
                    .with_help("add `*/` where the comment should end");
            }
            let (code, msg) = if text.starts_with('"') {
                (syntax::UNTERMINATED, "unterminated string literal".to_string())
            } else if text.starts_with('\'') {
                (syntax::UNTERMINATED, "unterminated character literal".to_string())
            } else {
                (syntax::UNKNOWN_CHAR, format!("unknown character(s) `{text}`"))
            };
            Diagnostic::error(code, span, msg)
        })
        .collect();
    (toks, docs, diags)
}

/// Delimiter pairs for error recovery.
pub(crate) const NEST: &[(Token, Token)] = &[(Token::LParen, Token::RParen), (Token::LBracket, Token::RBracket), (Token::LBrace, Token::RBrace)];

/// The Lunar parser: a thin grammar layer over [`ln_parse_kit::Parser`].
pub(crate) struct P<'s, 't, 'i> {
    pub(crate) k: ln_parse_kit::Parser<'t, Token>,
    /// Source text; token offsets index into it after subtracting `base`.
    src: &'s str,
    base: u32,
    pub(crate) file: FileId,
    pub(crate) ids: &'i mut IdGen,
    pub(crate) diags: Vec<Diagnostic>,
    /// Doc comments not yet attached to an item.
    pub(crate) docs: Docs,
}

impl<'s, 't, 'i> P<'s, 't, 'i> {
    fn new(src: &'s str, base: u32, toks: &'t [Tok<Token>], docs: Docs, file: FileId, ids: &'i mut IdGen, eof: u32) -> Self {
        Self { k: ln_parse_kit::Parser::new(toks, eof), src, base, file, ids, diags: Vec::new(), docs }
    }

    fn finish(mut self) -> Vec<Diagnostic> {
        let mut out = std::mem::take(&mut self.diags);
        for e in self.k.take_errors() {
            let span = Span::new(self.file, e.start, e.end);
            let d = match e.kind {
                ParseErrorKind::Expected { expected, found } => {
                    let mut names: Vec<String> = expected
                        .iter()
                        .map(|x| match x {
                            Expected::Token(t) => t.describe().to_string(),
                            Expected::Named(n) => n.to_string(),
                        })
                        .collect();
                    names.dedup();
                    let exp = match names.len() {
                        0 => "something else".to_string(),
                        1 => names[0].clone(),
                        _ => format!("one of {}", names.join(", ")),
                    };
                    let found_s = match found {
                        Some(t) => {
                            let text = self.text_of(e.start, e.end);
                            if matches!(t, Token::Ident) { format!("`{text}`") } else { t.describe().to_string() }
                        }
                        None => "end of file".to_string(),
                    };
                    Diagnostic::error(syntax::UNEXPECTED_TOKEN, span, format!("expected {exp}, found {found_s}")).with_label(format!("expected {exp}"))
                }
                ParseErrorKind::Custom(msg) => Diagnostic::error(syntax::UNEXPECTED_TOKEN, span, msg),
            };
            out.push(d);
        }
        out.sort_by_key(|d| (d.span.file, d.span.start));
        out
    }

    // ----- helpers -----

    pub(crate) fn text_of(&self, start: u32, end: u32) -> &'s str {
        &self.src[(start - self.base) as usize..(end - self.base) as usize]
    }

    pub(crate) fn tok_text(&self, t: Tok<Token>) -> &'s str {
        self.text_of(t.start, t.end)
    }

    pub(crate) fn span(&self, start: u32, end: u32) -> Span {
        Span::new(self.file, start, end)
    }

    pub(crate) fn tok_span(&self, t: Tok<Token>) -> Span {
        self.span(t.start, t.end)
    }

    /// Span from `start` to the end of the last consumed token.
    pub(crate) fn span_from(&self, start: u32) -> Span {
        let end = self.k.prev_end().max(start);
        self.span(start, end)
    }

    pub(crate) fn start(&self) -> u32 {
        self.k.pos_start()
    }

    pub(crate) fn id(&mut self) -> NodeId {
        self.ids.next()
    }

    pub(crate) fn ident(&mut self) -> PResult<Ident> {
        let t = self.k.expect(Token::Ident)?;
        Ok(Ident::new(self.tok_text(t), self.tok_span(t)))
    }

    pub(crate) fn error(&mut self, d: Diagnostic) {
        self.diags.push(d);
    }

    /// Tokens `a` and `b` are next and touch (e.g. `>` `>` forming `>>`).
    pub(crate) fn at_glued(&mut self, a: Token, b: Token) -> bool {
        self.k.nth_at(0, a) && self.k.nth_at(1, b) && self.k.adjacent(0)
    }

    pub(crate) fn at_glued3(&mut self, a: Token, b: Token, c: Token) -> bool {
        self.at_glued(a, b) && self.k.nth_at(2, c) && self.k.adjacent(1)
    }
}

#[cfg(test)]
mod tests;
