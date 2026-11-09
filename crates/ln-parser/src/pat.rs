//! Patterns for `let`, `match`, `for`, `if let` and `while let`.

use crate::P;
use ln_ast::*;
use ln_lexer::Token as T;
use ln_parse_kit::{Expected, PResult};

impl P<'_, '_, '_> {
    /// A pattern, including top-level `a | b` alternatives.
    pub(crate) fn pat(&mut self) -> PResult<Pat> {
        let start = self.start();
        self.k.eat(T::Pipe);
        let first = self.pat_single()?;
        if !self.k.at(T::Pipe) {
            return Ok(first);
        }
        let mut alts = vec![first];
        while self.k.eat(T::Pipe) {
            alts.push(self.pat_single()?);
        }
        let id = self.id();
        Ok(Pat { kind: PatKind::Or(alts), span: self.span_from(start), id })
    }

    fn lit_pat_part(&mut self) -> PResult<Option<(Lit, bool)>> {
        let neg = self.k.at(T::Minus) && matches!(self.k.nth(1), Some(T::Number(_)));
        if neg {
            self.k.bump();
        }
        match self.k.peek() {
            Some(T::Number(_) | T::Str | T::Char | T::Byte | T::True | T::False) => {
                let lit = self.literal()?;
                Ok(Some((lit, neg)))
            }
            _ => Ok(None),
        }
    }

    fn pat_single(&mut self) -> PResult<Pat> {
        let start = self.start();
        let kind = match self.k.peek() {
            Some(T::Placeholder) => {
                self.k.bump();
                PatKind::Wild
            }
            Some(T::DotDot) => {
                self.k.bump();
                PatKind::Rest
            }
            Some(T::Mut) => {
                self.k.bump();
                PatKind::Binding { is_mut: true, name: self.ident()? }
            }
            Some(T::Minus | T::Number(_) | T::Str | T::Char | T::Byte | T::True | T::False) => {
                let lo = self.lit_pat_part()?.ok_or_else(|| self.k.error_expected_named("literal"))?;
                if self.k.at(T::DotDotEq) || self.k.at(T::DotDot) {
                    let inclusive = self.k.bump().kind == T::DotDotEq;
                    let hi = self.lit_pat_part()?.ok_or_else(|| self.k.error_expected_named("literal"))?;
                    PatKind::Range { lo, hi, inclusive }
                } else {
                    PatKind::Lit(lo.0, lo.1)
                }
            }
            Some(T::LParen) => {
                self.k.bump();
                let mut pats = Vec::new();
                let mut trailing = false;
                loop {
                    if self.k.eat(T::RParen) {
                        break;
                    }
                    pats.push(self.pat()?);
                    trailing = self.k.eat(T::Comma);
                    if !trailing {
                        self.k.expect(T::RParen)?;
                        break;
                    }
                }
                if pats.len() == 1 && !trailing && !matches!(pats[0].kind, PatKind::Rest) {
                    return Ok(pats.pop().unwrap());
                }
                PatKind::Tuple(pats)
            }
            Some(T::Ident | T::SelfType | T::Crater | T::Super) => {
                let path = self.path_ty_style()?;
                if self.at_mac_bang() {
                    PatKind::MacCall(self.mac_call(path)?)
                } else if self.k.at(T::LParen) {
                    self.k.bump();
                    let mut pats = Vec::new();
                    loop {
                        if self.k.eat(T::RParen) {
                            break;
                        }
                        pats.push(self.pat()?);
                        if !self.k.eat(T::Comma) {
                            self.k.expect(T::RParen)?;
                            break;
                        }
                    }
                    PatKind::TupleStruct { path, pats }
                } else if self.k.at(T::LBrace) {
                    self.k.bump();
                    let mut fields = Vec::new();
                    let mut rest = false;
                    loop {
                        if self.k.eat(T::RBrace) {
                            break;
                        }
                        if self.k.eat(T::DotDot) {
                            rest = true;
                            self.k.expect(T::RBrace)?;
                            break;
                        }
                        let fstart = self.start();
                        let is_mut = self.k.eat(T::Mut);
                        let name = self.ident()?;
                        let pat = if !is_mut && self.k.eat(T::Colon) {
                            self.pat()?
                        } else {
                            let id = self.id();
                            Pat { kind: PatKind::Binding { is_mut, name: name.clone() }, span: self.span_from(fstart), id }
                        };
                        fields.push(FieldPat { name, pat });
                        if !self.k.eat(T::Comma) {
                            self.k.expect(T::RBrace)?;
                            break;
                        }
                    }
                    PatKind::Struct { path, fields, rest }
                } else if path.is_single() && path.segments[0].ident.name.chars().next().is_some_and(|c| c.is_lowercase() || c == '_') {
                    PatKind::Binding { is_mut: false, name: path.segments[0].ident.clone() }
                } else {
                    PatKind::Path(path)
                }
            }
            _ => {
                self.k.expect_note(Expected::Named("pattern"));
                return Err(self.k.error_expected());
            }
        };
        let id = self.id();
        Ok(Pat { kind, span: self.span_from(start), id })
    }
}
