//! A minimal, portable recursive-descent parsing framework.
//!
//! `ln-parse-kit` knows nothing about Lunar: it works over any token kind `K`
//! that is `Copy + Eq`. It provides a [`Parser`] cursor with lookahead,
//! expectation tracking for good "expected X, found Y" errors, checkpoints for
//! backtracking, delimited-list helpers and panic-mode error recovery.
//!
//! ```
//! use ln_parse_kit::{Parser, Tok};
//! #[derive(Clone, Copy, PartialEq, Eq, Debug)]
//! enum K { Num, Plus }
//! let toks = vec![Tok::new(K::Num, 0, 1), Tok::new(K::Plus, 1, 2), Tok::new(K::Num, 2, 3)];
//! let mut p = Parser::new(&toks, 3);
//! assert!(p.eat(K::Num));
//! assert!(p.at(K::Plus));
//! ```

use std::fmt::Debug;

/// A token: its kind plus the byte range it covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tok<K> {
    pub kind: K,
    pub start: u32,
    pub end: u32,
}

impl<K> Tok<K> {
    pub fn new(kind: K, start: u32, end: u32) -> Self {
        Self { kind, start, end }
    }
}

/// What the parser was looking for when an error happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expected<K> {
    Token(K),
    /// A named syntactic category, e.g. "expression" or "type".
    Named(&'static str),
}

/// A parse error. `start..end` is where it happened (the offending token, or
/// an empty range at end of input).
#[derive(Clone, Debug)]
pub struct ParseError<K> {
    pub start: u32,
    pub end: u32,
    pub kind: ParseErrorKind<K>,
}

#[derive(Clone, Debug)]
pub enum ParseErrorKind<K> {
    /// Expected one of the listed things but found `found` (`None` = end of input).
    Expected { expected: Vec<Expected<K>>, found: Option<K> },
    /// A free-form error message.
    Custom(String),
}

/// Result of a parse function; errors have already been recorded.
pub type PResult<T> = Result<T, Fatal>;

/// Marker returned by a parse function that has recorded an error and could
/// not produce a value. Callers either propagate it or recover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fatal;

/// A saved parser position for backtracking.
#[derive(Clone, Copy, Debug)]
pub struct Checkpoint {
    pos: usize,
    errors: usize,
}

/// A cursor over a token slice.
pub struct Parser<'t, K> {
    toks: &'t [Tok<K>],
    pos: usize,
    eof: u32,
    errors: Vec<ParseError<K>>,
    /// Expectations accumulated since the last consumed token.
    expected: Vec<Expected<K>>,
    /// Guard against infinite loops in buggy grammars.
    fuel: u32,
}

impl<'t, K: Copy + Eq + Debug> Parser<'t, K> {
    /// `eof` is the byte offset reported for "end of input" errors.
    pub fn new(toks: &'t [Tok<K>], eof: u32) -> Self {
        Self { toks, pos: 0, eof, errors: Vec::new(), expected: Vec::new(), fuel: 1024 }
    }

    // ----- inspection -----

    /// Kind of the current token.
    pub fn peek(&mut self) -> Option<K> {
        self.nth(0)
    }

    /// Kind of the token `n` ahead of the current one.
    pub fn nth(&mut self, n: usize) -> Option<K> {
        self.fuel = self.fuel.saturating_sub(1);
        assert!(self.fuel > 0, "parser made no progress (grammar bug)");
        self.toks.get(self.pos + n).map(|t| t.kind)
    }

    /// The current token.
    pub fn current(&self) -> Option<Tok<K>> {
        self.toks.get(self.pos).copied()
    }

    pub fn nth_tok(&self, n: usize) -> Option<Tok<K>> {
        self.toks.get(self.pos + n).copied()
    }

    /// The most recently consumed token.
    pub fn prev(&self) -> Option<Tok<K>> {
        self.pos.checked_sub(1).and_then(|i| self.toks.get(i)).copied()
    }

    /// Index of the current token (for progress checks).
    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn at_eof(&self) -> bool {
        self.pos >= self.toks.len()
    }

    /// Whether the current token has kind `k`. Records `k` as expected.
    pub fn at(&mut self, k: K) -> bool {
        if self.nth(0) == Some(k) {
            true
        } else {
            self.expect_note(Expected::Token(k));
            false
        }
    }

    pub fn at_any(&mut self, ks: &[K]) -> bool {
        let mut hit = false;
        for &k in ks {
            hit |= self.at(k);
        }
        hit
    }

    /// Whether token `n` ahead has kind `k` (does not record expectations).
    pub fn nth_at(&mut self, n: usize, k: K) -> bool {
        self.nth(n) == Some(k)
    }

    /// Whether tokens `n` and `n + 1` ahead touch (no whitespace between them).
    pub fn adjacent(&self, n: usize) -> bool {
        match (self.toks.get(self.pos + n), self.toks.get(self.pos + n + 1)) {
            (Some(a), Some(b)) => a.end == b.start,
            _ => false,
        }
    }

    /// Start offset of the current token (or EOF).
    pub fn pos_start(&self) -> u32 {
        self.current().map_or(self.eof, |t| t.start)
    }

    /// End offset of the previously consumed token.
    pub fn prev_end(&self) -> u32 {
        self.prev().map_or(0, |t| t.end)
    }

    // ----- consumption -----

    /// Consume the current token unconditionally.
    pub fn bump(&mut self) -> Tok<K> {
        let t = self.toks.get(self.pos).copied().expect("bump past end of input");
        self.pos += 1;
        self.expected.clear();
        self.fuel = 1024;
        t
    }

    /// Consume `n` tokens, returning a token spanning all of them with kind `kind`.
    pub fn bump_glued(&mut self, n: usize, kind: K) -> Tok<K> {
        let first = self.bump();
        let mut last = first;
        for _ in 1..n {
            last = self.bump();
        }
        Tok::new(kind, first.start, last.end)
    }

    /// Consume the current token if it has kind `k`.
    pub fn eat(&mut self, k: K) -> bool {
        if self.at(k) {
            self.bump();
            true
        } else {
            false
        }
    }

    /// Consume a token of kind `k` or record an error.
    pub fn expect(&mut self, k: K) -> PResult<Tok<K>> {
        if self.at(k) { Ok(self.bump()) } else { Err(self.error_expected()) }
    }

    /// Note that a named category would have been accepted here.
    pub fn expect_note(&mut self, e: Expected<K>) {
        if !self.expected.contains(&e) {
            self.expected.push(e);
        }
    }

    // ----- errors -----

    /// Record an "expected ..., found ..." error at the current token using
    /// every expectation noted since the last consumed token.
    pub fn error_expected(&mut self) -> Fatal {
        let expected = std::mem::take(&mut self.expected);
        let (start, end, found) = match self.current() {
            Some(t) => (t.start, t.end, Some(t.kind)),
            None => (self.eof, self.eof, None),
        };
        self.push_error(ParseError { start, end, kind: ParseErrorKind::Expected { expected, found } });
        Fatal
    }

    /// Record "expected <what>" at the current token.
    pub fn error_expected_named(&mut self, what: &'static str) -> Fatal {
        self.expect_note(Expected::Named(what));
        self.error_expected()
    }

    /// Record a custom error over `start..end`.
    pub fn error_at(&mut self, start: u32, end: u32, msg: impl Into<String>) -> Fatal {
        self.push_error(ParseError { start, end, kind: ParseErrorKind::Custom(msg.into()) });
        Fatal
    }

    fn push_error(&mut self, e: ParseError<K>) {
        // Avoid cascades: one error per position.
        if self.errors.last().is_some_and(|l| l.start == e.start) {
            return;
        }
        self.errors.push(e);
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    pub fn error_count(&self) -> usize {
        self.errors.len()
    }

    pub fn take_errors(&mut self) -> Vec<ParseError<K>> {
        std::mem::take(&mut self.errors)
    }

    // ----- backtracking -----

    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint { pos: self.pos, errors: self.errors.len() }
    }

    /// Return to a checkpoint, discarding errors recorded since.
    pub fn rewind(&mut self, cp: Checkpoint) {
        self.pos = cp.pos;
        self.errors.truncate(cp.errors);
        self.expected.clear();
    }

    /// Run `f`; if it fails, rewind and return `None`.
    pub fn attempt<T>(&mut self, f: impl FnOnce(&mut Self) -> PResult<T>) -> Option<T> {
        let cp = self.checkpoint();
        match f(self) {
            Ok(v) if self.errors.len() == cp.errors => Some(v),
            _ => {
                self.rewind(cp);
                None
            }
        }
    }

    // ----- recovery -----

    /// Skip tokens until one of `stop` is current (not consumed) or input
    /// ends. Balanced delimiter pairs in `nest` are skipped as a unit, and an
    /// unbalanced closer stops recovery (it belongs to an enclosing construct).
    pub fn recover_until(&mut self, stop: &[K], nest: &[(K, K)]) {
        let mut depth: Vec<K> = Vec::new();
        while let Some(t) = self.current() {
            if depth.is_empty() && stop.contains(&t.kind) {
                return;
            }
            if let Some(&(_, close)) = nest.iter().find(|(o, _)| *o == t.kind) {
                depth.push(close);
            } else if depth.last() == Some(&t.kind) {
                depth.pop();
            } else if depth.is_empty() && nest.iter().any(|(_, c)| *c == t.kind) {
                return;
            }
            self.bump();
        }
    }

    // ----- combinators -----

    /// Parse `open item (sep item)* sep? close`. Errors inside an item recover
    /// to the next separator or closer.
    pub fn delimited<T>(&mut self, open: K, close: K, sep: K, nest: &[(K, K)], mut item: impl FnMut(&mut Self) -> PResult<T>) -> PResult<Vec<T>> {
        self.expect(open)?;
        let mut out = Vec::new();
        loop {
            if self.eat(close) {
                return Ok(out);
            }
            if self.at_eof() {
                return Err(self.error_expected());
            }
            match item(self) {
                Ok(v) => out.push(v),
                Err(Fatal) => self.recover_until(&[sep, close], nest),
            }
            if !self.eat(sep) {
                self.expect(close)?;
                return Ok(out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum K {
        A,
        Comma,
        L,
        R,
    }

    fn toks(ks: &[K]) -> Vec<Tok<K>> {
        ks.iter().enumerate().map(|(i, &k)| Tok::new(k, i as u32 * 2, i as u32 * 2 + 1)).collect()
    }

    #[test]
    fn delimited_list_with_trailing_sep() {
        let t = toks(&[K::L, K::A, K::Comma, K::A, K::Comma, K::R]);
        let mut p = Parser::new(&t, 100);
        let v = p.delimited(K::L, K::R, K::Comma, &[], |p| p.expect(K::A)).unwrap();
        assert_eq!(v.len(), 2);
        assert!(p.at_eof() && !p.has_errors());
    }

    #[test]
    fn expected_errors_accumulate() {
        let t = toks(&[K::R]);
        let mut p = Parser::new(&t, 100);
        assert!(!p.at(K::A));
        assert!(!p.at(K::L));
        let _ = p.error_expected();
        let e = p.take_errors();
        match &e[0].kind {
            ParseErrorKind::Expected { expected, found } => {
                assert_eq!(expected.len(), 2);
                assert_eq!(*found, Some(K::R));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn attempt_rewinds() {
        let t = toks(&[K::A, K::R]);
        let mut p = Parser::new(&t, 100);
        let r = p.attempt(|p| {
            p.expect(K::A)?;
            p.expect(K::A)
        });
        assert!(r.is_none());
        assert!(p.at(K::A) && !p.has_errors());
    }

    #[test]
    fn recovery_skips_nested() {
        let t = toks(&[K::A, K::L, K::Comma, K::R, K::Comma, K::A]);
        let mut p = Parser::new(&t, 100);
        p.recover_until(&[K::Comma], &[(K::L, K::R)]);
        assert!(p.at(K::Comma));
        p.bump();
        assert!(p.at(K::A));
    }
}
