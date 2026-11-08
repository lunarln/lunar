//! Types and paths.

use crate::P;
use ln_ast::*;
use ln_lexer::{NumKind, Token as T};
use ln_parse_kit::{Expected, PResult};

impl P<'_, '_, '_> {
    /// A path such as `std::io::println`, `Pair::<i32>::new`, `Self`, `crate::x`.
    /// Generic arguments are always introduced with `::<`.
    pub(crate) fn path_ty_style(&mut self) -> PResult<Path> {
        let start = self.start();
        let mut segments = Vec::new();
        loop {
            let ident = self.path_segment_ident()?;
            let mut args = None;
            if self.k.at(T::PathSep) && self.k.nth_at(1, T::LAngle) {
                self.k.bump();
                args = Some(self.generic_args()?);
            }
            segments.push(PathSegment { ident, args });
            if self.k.at(T::PathSep) && matches!(self.k.nth(1), Some(T::Ident | T::SelfType | T::SelfValue | T::Super | T::Crate)) {
                self.k.bump();
                continue;
            }
            break;
        }
        let id = self.id();
        Ok(Path { segments, span: self.span_from(start), id })
    }

    /// `<A, B>` (the leading `::` already consumed).
    pub(crate) fn generic_args(&mut self) -> PResult<Vec<Ty>> {
        self.k.expect(T::LAngle)?;
        let mut out = Vec::new();
        loop {
            if self.k.eat(T::RAngle) {
                return Ok(out);
            }
            out.push(self.ty()?);
            if !self.k.eat(T::Comma) {
                self.k.expect(T::RAngle)?;
                return Ok(out);
            }
        }
    }

    /// A return type: `!` alone means "never returns".
    pub(crate) fn ret_ty(&mut self) -> PResult<Ty> {
        if self.k.at(T::Bang) && !self.ty_can_start(1) {
            let t = self.k.bump();
            let id = self.id();
            return Ok(Ty { kind: TyKind::Never, span: self.tok_span(t), id });
        }
        self.ty()
    }

    fn ty_can_start(&mut self, n: usize) -> bool {
        matches!(
            self.k.nth(n),
            Some(T::Ident | T::SelfType | T::Crate | T::Super | T::SelfValue | T::LBracket | T::LParen | T::Star | T::Bang | T::Fn | T::Placeholder)
        )
    }

    pub(crate) fn ty(&mut self) -> PResult<Ty> {
        let start = self.start();
        let kind = match self.k.peek() {
            Some(T::Ident | T::SelfType | T::Crate | T::Super | T::SelfValue) => TyKind::Path(self.path_ty_style()?),
            Some(T::Placeholder) => {
                self.k.bump();
                TyKind::Infer
            }
            Some(T::Bang) => {
                self.k.bump();
                if self.ty_can_start(0) { TyKind::Bang(Box::new(self.ty()?)) } else { TyKind::Never }
            }
            Some(T::Star) => {
                self.k.bump();
                let is_mut = self.k.eat(T::Mut);
                TyKind::Ptr { is_mut, inner: Box::new(self.ty()?) }
            }
            Some(T::LParen) => {
                self.k.bump();
                let mut elems = Vec::new();
                let mut trailing_comma = false;
                loop {
                    if self.k.eat(T::RParen) {
                        break;
                    }
                    elems.push(self.ty()?);
                    trailing_comma = self.k.eat(T::Comma);
                    if !trailing_comma {
                        self.k.expect(T::RParen)?;
                        break;
                    }
                }
                if elems.len() == 1 && !trailing_comma {
                    return Ok(elems.pop().unwrap());
                }
                TyKind::Tuple(elems)
            }
            Some(T::Fn) => {
                self.k.bump();
                self.k.expect(T::LParen)?;
                let mut params = Vec::new();
                loop {
                    if self.k.eat(T::RParen) {
                        break;
                    }
                    params.push(self.ty()?);
                    if !self.k.eat(T::Comma) {
                        self.k.expect(T::RParen)?;
                        break;
                    }
                }
                let ret = if self.k.eat(T::Colon) { Some(Box::new(self.ret_ty()?)) } else { None };
                TyKind::Fn { params, ret }
            }
            Some(T::LBracket) => {
                self.k.bump();
                let elem = Box::new(self.ty()?);
                if self.k.eat(T::RBracket) {
                    TyKind::DynArray(elem)
                } else {
                    self.k.expect(T::Semi)?;
                    let size = self.array_size()?;
                    self.k.expect(T::RBracket)?;
                    TyKind::Array(elem, size)
                }
            }
            _ => {
                self.k.expect_note(Expected::Named("type"));
                return Err(self.k.error_expected());
            }
        };
        let id = self.id();
        Ok(Ty { kind, span: self.span_from(start), id })
    }

    /// The part after `;` in `[T; ...]`.
    fn array_size(&mut self) -> PResult<ArraySize> {
        match self.k.peek() {
            Some(T::Placeholder) => {
                self.k.bump();
                Ok(ArraySize::Inferred)
            }
            Some(T::Amp) => {
                self.k.bump();
                Ok(if self.k.eat(T::Mut) { ArraySize::SliceMut } else { ArraySize::Slice })
            }
            Some(T::DotDot) if self.k.nth_at(1, T::Plus) => {
                self.k.bump();
                self.k.bump();
                Ok(ArraySize::Grow)
            }
            Some(T::Minus) if self.k.nth_at(1, T::DotDot) && self.k.nth_at(2, T::Plus) => {
                self.k.bump();
                self.k.bump();
                self.k.bump();
                Ok(ArraySize::GrowShrink)
            }
            Some(T::Number(NumKind::Int) | T::Ident | T::LParen | T::LBrace) => Ok(ArraySize::Fixed(Box::new(self.expr()?))),
            _ => {
                self.k.expect_note(Expected::Named("array size (`N`, `_`, `&`, `&mut`, `..+` or `-..+`)"));
                Err(self.k.error_expected())
            }
        }
    }
}
