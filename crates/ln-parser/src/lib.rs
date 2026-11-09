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
    let (toks, diags) = lex_with_offset(src, 0, file);
    let mut p = P::new(src, 0, &toks, file, ids, src.len() as u32);
    p.diags = diags;
    let items = p.items_until_eof();
    let file_ast = File { items, span: Span::new(file, 0, src.len() as u32) };
    let diags = p.finish();
    (file_ast, diags)
}

/// Parse a single expression from `src`, whose first byte sits at `offset` in
/// file `file`. Used for format-string interpolation holes.
pub(crate) fn parse_expr_at(src: &str, offset: u32, file: FileId, ids: &mut IdGen) -> Result<Expr, Vec<Diagnostic>> {
    let (toks, mut diags) = lex_with_offset(src, offset, file);
    let eof = offset + src.len() as u32;
    let mut p = P::new(src, offset, &toks, file, ids, eof);
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
        toks.push(ln_lexer::Spanned { token: Token::Char, range: start..end });
        for r in leftover {
            let (more, more_errs) = ln_lexer::lex(&src[r.clone()]);
            toks.extend(more.into_iter().map(|t| ln_lexer::Spanned { token: t.token, range: t.range.start + r.start..t.range.end + r.start }));
            errs.extend(more_errs.into_iter().map(|e| ln_lexer::LexError { range: e.range.start + r.start..e.range.end + r.start }));
        }
        toks.sort_by_key(|t| t.range.start);
        errs.sort_by_key(|e| e.range.start);
    }
}

fn lex_with_offset(src: &str, offset: u32, file: FileId) -> (Vec<Tok<Token>>, Vec<Diagnostic>) {
    let (mut toks, mut errs) = ln_lexer::lex(src);
    rejoin_char_literals(src, &mut toks, &mut errs);
    let toks = toks.into_iter().map(|s| Tok::new(s.token, s.range.start as u32 + offset, s.range.end as u32 + offset)).collect();
    let diags = errs
        .into_iter()
        .map(|e| {
            let span = Span::new(file, e.range.start as u32 + offset, e.range.end as u32 + offset);
            let text = &src[e.range.clone()];
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
    (toks, diags)
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
}

impl<'s, 't, 'i> P<'s, 't, 'i> {
    fn new(src: &'s str, base: u32, toks: &'t [Tok<Token>], file: FileId, ids: &'i mut IdGen, eof: u32) -> Self {
        Self { k: ln_parse_kit::Parser::new(toks, eof), src, base, file, ids, diags: Vec::new() }
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
