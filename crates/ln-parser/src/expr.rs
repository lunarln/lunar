//! Expressions, statements and blocks.

use crate::{NEST, P};
use ln_ast::*;
use ln_diag::{Diagnostic, codes::syntax};
use ln_lexer::{NumKind, Token as T};
use ln_parse_kit::{Expected, Fatal, PResult};

/// Restrictions on what an expression may contain.
#[derive(Clone, Copy, Default)]
pub(crate) struct Restr {
    /// Struct literals are not allowed (conditions of `if`, `while`, `match`, `for`).
    no_struct: bool,
}

const NO_STRUCT: Restr = Restr { no_struct: true };

impl P<'_, '_, '_> {
    fn mk(&mut self, kind: ExprKind, start: u32) -> Expr {
        let id = self.id();
        Expr { kind, span: self.span_from(start), id }
    }

    fn mk_span(&mut self, kind: ExprKind, span: Span) -> Expr {
        let id = self.id();
        Expr { kind, span, id }
    }

    pub(crate) fn expr(&mut self) -> PResult<Expr> {
        self.expr_r(Restr::default())
    }

    fn expr_no_struct(&mut self) -> PResult<Expr> {
        self.expr_r(NO_STRUCT)
    }

    fn expr_r(&mut self, r: Restr) -> PResult<Expr> {
        self.assign(r)
    }

    // ----- assignment -----

    fn assign_op(&mut self) -> Option<(Option<BinOp>, usize)> {
        let op = match self.k.peek()? {
            T::Eq => return Some((None, 1)),
            T::PlusEq => BinOp::Add,
            T::MinusEq => BinOp::Sub,
            T::StarEq => BinOp::Mul,
            T::SlashEq => BinOp::Div,
            T::PercentEq => BinOp::Rem,
            T::AmpEq => BinOp::BitAnd,
            T::PipeEq => BinOp::BitOr,
            T::CaretEq => BinOp::BitXor,
            T::ShlEq => BinOp::Shl,
            T::RAngle if self.at_glued3(T::RAngle, T::RAngle, T::Eq) => return Some((Some(BinOp::Shr), 3)),
            _ => return None,
        };
        Some((Some(op), 1))
    }

    fn assign(&mut self, r: Restr) -> PResult<Expr> {
        let start = self.start();
        let lhs = self.range(r)?;
        if let Some((op, n)) = self.assign_op() {
            for _ in 0..n {
                self.k.bump();
            }
            let rhs = self.assign(r)?;
            let kind = match op {
                None => ExprKind::Assign(Box::new(lhs), Box::new(rhs)),
                Some(op) => ExprKind::AssignOp(op, Box::new(lhs), Box::new(rhs)),
            };
            return Ok(self.mk(kind, start));
        }
        Ok(lhs)
    }

    // ----- ranges -----

    /// Whether the current token can begin an expression.
    fn can_start_expr(&mut self) -> bool {
        matches!(
            self.k.peek(),
            Some(
                T::Ident
                   
                    | T::Number(_)
                    | T::Str
                    | T::Char
                    | T::True
                    | T::False
                    | T::LParen
                    | T::LBracket
                    | T::LBrace
                    | T::Minus
                    | T::Bang
                    | T::Star
                    | T::SelfValue
                    | T::SelfType
                    | T::Crate
                    | T::Super
                    | T::If
                    | T::Match
                    | T::Unsafe
                    | T::Loop
                    | T::While
                    | T::For
            )
        )
    }

    fn range(&mut self, r: Restr) -> PResult<Expr> {
        let start = self.start();
        if self.k.at(T::DotDot) || self.k.at(T::DotDotEq) {
            let inclusive = self.k.bump().kind == T::DotDotEq;
            let end = if self.can_start_expr() && !(r.no_struct && self.k.at(T::LBrace)) { Some(Box::new(self.binary(0, r)?)) } else { None };
            return Ok(self.mk(ExprKind::Range { start: None, end, inclusive }, start));
        }
        let lhs = self.binary(0, r)?;
        if self.k.at(T::DotDot) || self.k.at(T::DotDotEq) {
            let inclusive = self.k.bump().kind == T::DotDotEq;
            let end = if self.can_start_expr() && !(r.no_struct && self.k.at(T::LBrace)) { Some(Box::new(self.binary(0, r)?)) } else { None };
            return Ok(self.mk(ExprKind::Range { start: Some(Box::new(lhs)), end, inclusive }, start));
        }
        Ok(lhs)
    }

    // ----- binary operators (precedence climbing) -----

    /// The binary operator at the cursor: (op, precedence, token count).
    fn peek_binop(&mut self) -> Option<(BinOp, u8, usize)> {
        use BinOp::*;
        let t = self.k.peek()?;
        Some(match t {
            T::PipePipe => (Or, 1, 1),
            T::AmpAmp => (And, 2, 1),
            T::EqEq => (Eq, 3, 1),
            T::BangEq => (Ne, 3, 1),
            T::LAngle => (Lt, 3, 1),
            T::Le => (Le, 3, 1),
            T::RAngle => {
                if self.at_glued(T::RAngle, T::RAngle) {
                    if self.at_glued3(T::RAngle, T::RAngle, T::Eq) {
                        return None; // `>>=` is an assignment operator
                    }
                    (Shr, 7, 2)
                } else if self.at_glued(T::RAngle, T::Eq) {
                    (Ge, 3, 2)
                } else {
                    (Gt, 3, 1)
                }
            }
            T::Pipe => (BitOr, 4, 1),
            T::Caret => (BitXor, 5, 1),
            T::Amp => (BitAnd, 6, 1),
            T::Shl => (Shl, 7, 1),
            T::Plus => (Add, 8, 1),
            T::Minus => (Sub, 8, 1),
            T::Star => (Mul, 9, 1),
            T::Slash => (Div, 9, 1),
            T::Percent => (Rem, 9, 1),
            _ => return None,
        })
    }

    fn binary(&mut self, min_prec: u8, r: Restr) -> PResult<Expr> {
        let start = self.start();
        let mut lhs = self.cast(r)?;
        while let Some((op, prec, n)) = self.peek_binop() {
            if prec <= min_prec {
                break;
            }
            for _ in 0..n {
                self.k.bump();
            }
            // Comparisons are non-associative: parse the rhs one level higher.
            let rhs = self.binary(prec, r)?;
            lhs = self.mk(ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)), start);
            if op.is_comparison()
                && let Some((op2, 3, _)) = self.peek_binop()
            {
                let s = self.start();
                self.error(
                    Diagnostic::error(syntax::UNEXPECTED_TOKEN, self.span(s, s + 1), format!("comparison operators cannot be chained (`{}` after `{}`)", op2.as_str(), op.as_str()))
                        .with_help("split the comparison with `&&`, e.g. `a < b && b < c`"),
                );
            }
        }
        Ok(lhs)
    }

    fn cast(&mut self, r: Restr) -> PResult<Expr> {
        let start = self.start();
        let mut e = self.unary(r)?;
        while self.k.eat(T::As) {
            let ty = self.ty()?;
            e = self.mk(ExprKind::Cast(Box::new(e), ty), start);
        }
        Ok(e)
    }

    fn unary(&mut self, r: Restr) -> PResult<Expr> {
        let start = self.start();
        let op = match self.k.peek() {
            Some(T::Minus) => UnOp::Neg,
            Some(T::Bang) => UnOp::Not,
            Some(T::Star) => UnOp::Deref,
            _ => return self.postfix(r),
        };
        self.k.bump();
        let inner = self.unary(r)?;
        Ok(self.mk(ExprKind::Unary(op, Box::new(inner)), start))
    }

    fn call_args(&mut self) -> PResult<Vec<Expr>> {
        self.k.expect(T::LParen)?;
        let mut args = Vec::new();
        loop {
            if self.k.eat(T::RParen) {
                return Ok(args);
            }
            match self.expr() {
                Ok(e) => args.push(e),
                Err(Fatal) => self.k.recover_until(&[T::Comma, T::RParen], NEST),
            }
            if !self.k.eat(T::Comma) {
                self.k.expect(T::RParen)?;
                return Ok(args);
            }
        }
    }

    fn postfix(&mut self, r: Restr) -> PResult<Expr> {
        let start = self.start();
        let e = self.primary(r)?;
        self.postfix_ops(e, start)
    }

    /// Continue parsing after an already-parsed block-like expression.
    fn continue_expr(&mut self, e: Expr, start: u32) -> PResult<Expr> {
        let mut e = self.postfix_ops(e, start)?;
        while self.k.eat(T::As) {
            let ty = self.ty()?;
            e = self.mk(ExprKind::Cast(Box::new(e), ty), start);
        }
        Ok(e)
    }

    fn postfix_ops(&mut self, mut e: Expr, start: u32) -> PResult<Expr> {
        loop {
            match self.k.peek() {
                Some(T::LParen) => {
                    let args = self.call_args()?;
                    e = self.mk(ExprKind::Call(Box::new(e), args), start);
                }
                Some(T::LBracket) => {
                    self.k.bump();
                    let idx = self.expr()?;
                    self.k.expect(T::RBracket)?;
                    e = self.mk(ExprKind::Index(Box::new(e), Box::new(idx)), start);
                }
                Some(T::Question) => {
                    self.k.bump();
                    e = self.mk(ExprKind::Try(Box::new(e)), start);
                }
                Some(T::Dot) => {
                    self.k.bump();
                    match self.k.peek() {
                        Some(T::Number(kind)) => {
                            let t = self.k.bump();
                            let text = self.tok_text(t);
                            // `t.0.1` lexes `0.1` as a float: split it.
                            let parts: Vec<&str> = if kind == NumKind::Float { text.split('.').collect() } else { vec![text] };
                            let mut off = t.start;
                            for part in parts {
                                let sp = self.span(off, off + part.len() as u32);
                                off += part.len() as u32 + 1;
                                match part.parse::<u32>() {
                                    Ok(i) => e = self.mk_span(ExprKind::TupleField(Box::new(e), i, sp), self.span(start, sp.end)),
                                    Err(_) => {
                                        self.error(Diagnostic::error(syntax::BAD_NUMBER, sp, "invalid tuple field index"));
                                    }
                                }
                            }
                        }
                        _ => {
                            let name = self.ident()?;
                            let generics = if self.k.at(T::PathSep) && self.k.nth_at(1, T::LAngle) {
                                self.k.bump();
                                Some(self.generic_args()?)
                            } else {
                                None
                            };
                            if self.k.at(T::LParen) {
                                let args = self.call_args()?;
                                e = self.mk(ExprKind::MethodCall { recv: Box::new(e), name, generics, args }, start);
                            } else {
                                if generics.is_some() {
                                    self.error(Diagnostic::error(syntax::UNEXPECTED_TOKEN, name.span, "field access cannot take generic arguments"));
                                }
                                e = self.mk(ExprKind::Field(Box::new(e), name), start);
                            }
                        }
                    }
                }
                _ => return Ok(e),
            }
        }
    }

    // ----- primary expressions -----

    pub(crate) fn literal(&mut self) -> PResult<Lit> {
        let t = self.k.bump();
        let text = self.tok_text(t);
        let sp = self.tok_span(t);
        Ok(match t.kind {
            T::True => Lit::Bool(true),
            T::False => Lit::Bool(false),
            T::Number(NumKind::Int) => match crate::parse_int_lit(text) {
                Ok((value, suffix)) => Lit::Int { value, suffix },
                Err(m) => {
                    self.error(Diagnostic::error(syntax::BAD_NUMBER, sp, m));
                    Lit::Int { value: 0, suffix: None }
                }
            },
            T::Number(NumKind::Float) => match crate::parse_float_lit(text) {
                Ok((value, suffix)) => Lit::Float { value, suffix },
                Err(m) => {
                    self.error(Diagnostic::error(syntax::BAD_NUMBER, sp, m));
                    Lit::Float { value: 0.0, suffix: None }
                }
            },
            T::Char => {
                let raw = &text[1..text.len() - 1];
                match crate::lit::unescape(raw) {
                    Ok(s) if s.chars().count() == 1 => Lit::Char(s.chars().next().unwrap()),
                    Ok(_) => {
                        self.error(Diagnostic::error(syntax::BAD_ESCAPE, sp, "character literal must contain exactly one character"));
                        Lit::Char('\0')
                    }
                    Err((_, _, m)) => {
                        self.error(Diagnostic::error(syntax::BAD_ESCAPE, sp, m));
                        Lit::Char('\0')
                    }
                }
            }
            T::Str => {
                let raw = &text[1..text.len() - 1];
                let value = match crate::lit::unescape(raw) {
                    Ok(s) => s,
                    Err((a, b, m)) => {
                        let s = Span::new(self.file, t.start + 1 + a as u32, t.start + 1 + b as u32);
                        self.error(Diagnostic::error(syntax::BAD_ESCAPE, s, m));
                        String::new()
                    }
                };
                let fmt = if raw.contains('{') || raw.contains('}') {
                    crate::lit::parse_format(raw, t.start + 1, self.file, self.ids)
                } else {
                    FmtString::Ok(if value.is_empty() { Vec::new() } else { vec![FmtPiece::Text(value.clone())] })
                };
                Lit::Str { value, fmt }
            }
            _ => unreachable!("literal() called on non-literal"),
        })
    }

    fn primary(&mut self, r: Restr) -> PResult<Expr> {
        let start = self.start();
        let Some(t) = self.k.peek() else {
            self.k.expect_note(Expected::Named("expression"));
            return Err(self.k.error_expected());
        };
        let kind = match t {
            T::Number(_) | T::Str | T::Char | T::True | T::False => ExprKind::Lit(self.literal()?),
            T::Ident | T::SelfValue | T::SelfType | T::Crate | T::Super => {
                let path = self.path_ty_style()?;
                if self.k.at(T::LBrace) && !r.no_struct && self.looks_like_struct_lit(&path) {
                    self.struct_lit(path)?
                } else {
                    ExprKind::Path(path)
                }
            }
            T::LParen => {
                self.k.bump();
                if self.k.eat(T::RParen) {
                    ExprKind::Tuple(Vec::new())
                } else {
                    let first = self.expr()?;
                    if self.k.eat(T::RParen) {
                        ExprKind::Paren(Box::new(first))
                    } else {
                        let mut elems = vec![first];
                        while self.k.eat(T::Comma) {
                            if self.k.at(T::RParen) {
                                break;
                            }
                            elems.push(self.expr()?);
                        }
                        self.k.expect(T::RParen)?;
                        ExprKind::Tuple(elems)
                    }
                }
            }
            T::LBracket => {
                self.k.bump();
                if self.k.eat(T::RBracket) {
                    ExprKind::Array(Vec::new())
                } else {
                    let first = self.expr()?;
                    if self.k.eat(T::Semi) {
                        let count = self.expr()?;
                        self.k.expect(T::RBracket)?;
                        ExprKind::ArrayRepeat(Box::new(first), Box::new(count))
                    } else {
                        let mut elems = vec![first];
                        while self.k.eat(T::Comma) {
                            if self.k.at(T::RBracket) {
                                break;
                            }
                            elems.push(self.expr()?);
                        }
                        self.k.expect(T::RBracket)?;
                        ExprKind::Array(elems)
                    }
                }
            }
            T::LBrace => ExprKind::Block(self.block()?),
            T::Unsafe => {
                self.k.bump();
                ExprKind::Unsafe(self.block()?)
            }
            T::If => return self.if_expr(),
            T::Match => {
                self.k.bump();
                let scrut = Box::new(self.expr_no_struct()?);
                let arms = self.match_arms()?;
                ExprKind::Match { scrut, arms }
            }
            T::While => {
                self.k.bump();
                if self.k.eat(T::Let) {
                    let pat = self.pat()?;
                    self.k.expect(T::Eq)?;
                    let scrut = Box::new(self.expr_no_struct()?);
                    let body = self.block()?;
                    ExprKind::WhileLet { pat, scrut, body }
                } else {
                    let cond = Box::new(self.expr_no_struct()?);
                    let body = self.block()?;
                    ExprKind::While { cond, body }
                }
            }
            T::Loop => {
                self.k.bump();
                ExprKind::Loop(self.block()?)
            }
            T::For => {
                self.k.bump();
                let pat = self.pat()?;
                self.k.expect(T::In)?;
                let iter = Box::new(self.expr_no_struct()?);
                let body = self.block()?;
                ExprKind::For { pat, iter, body }
            }
            T::Return => {
                self.k.bump();
                let v = if self.can_start_expr() || self.k.at(T::DotDot) { Some(Box::new(self.expr_r(r)?)) } else { None };
                ExprKind::Return(v)
            }
            T::Break => {
                self.k.bump();
                let v = if self.can_start_expr() && !(r.no_struct && self.k.at(T::LBrace)) { Some(Box::new(self.expr_r(r)?)) } else { None };
                ExprKind::Break(v)
            }
            T::Continue => {
                self.k.bump();
                ExprKind::Continue
            }
            _ => {
                self.k.expect_note(Expected::Named("expression"));
                return Err(self.k.error_expected());
            }
        };
        Ok(self.mk(kind, start))
    }

    /// After a path, `{` starts a struct literal only if it looks like one:
    /// `{ }`, `{ ident: ...`, `{ ident, ...` or `{ ident }`.
    fn looks_like_struct_lit(&mut self, path: &Path) -> bool {
        let starts_upper = path.last().ident.name.chars().next().is_some_and(|c| c.is_uppercase());
        match (self.k.nth(1), self.k.nth(2)) {
            (Some(T::RBrace), _) => starts_upper,
            (Some(T::Ident), Some(T::Colon)) => true,
            (Some(T::Ident), Some(T::Comma | T::RBrace)) => starts_upper,
            _ => false,
        }
    }

    fn struct_lit(&mut self, path: Path) -> PResult<ExprKind> {
        self.k.expect(T::LBrace)?;
        let mut fields = Vec::new();
        loop {
            if self.k.eat(T::RBrace) {
                break;
            }
            let start = self.start();
            let name = self.ident()?;
            let value = if self.k.eat(T::Colon) { Some(self.expr()?) } else { None };
            fields.push(FieldInit { name, value, span: self.span_from(start) });
            if !self.k.eat(T::Comma) {
                self.k.expect(T::RBrace)?;
                break;
            }
        }
        Ok(ExprKind::StructLit { path, fields })
    }

    fn if_expr(&mut self) -> PResult<Expr> {
        let start = self.start();
        self.k.expect(T::If)?;
        if self.k.eat(T::Let) {
            let pat = self.pat()?;
            self.k.expect(T::Eq)?;
            let scrut = Box::new(self.expr_no_struct()?);
            let then = self.block()?;
            let else_ = self.else_branch()?;
            return Ok(self.mk(ExprKind::IfLet { pat, scrut, then, else_ }, start));
        }
        let cond = Box::new(self.expr_no_struct()?);
        let then = self.block()?;
        let else_ = self.else_branch()?;
        Ok(self.mk(ExprKind::If { cond, then, else_ }, start))
    }

    fn else_branch(&mut self) -> PResult<Option<Box<Expr>>> {
        if !self.k.eat(T::Else) {
            return Ok(None);
        }
        if self.k.at(T::If) {
            return Ok(Some(Box::new(self.if_expr()?)));
        }
        let start = self.start();
        let b = self.block()?;
        Ok(Some(Box::new(self.mk(ExprKind::Block(b), start))))
    }

    fn match_arms(&mut self) -> PResult<Vec<Arm>> {
        self.k.expect(T::LBrace)?;
        let mut arms = Vec::new();
        loop {
            if self.k.eat(T::RBrace) {
                return Ok(arms);
            }
            if self.k.at_eof() {
                return Err(self.k.error_expected());
            }
            let start = self.start();
            let arm = (|| -> PResult<Arm> {
                let pat = self.pat()?;
                let guard = if self.k.eat(T::If) { Some(self.expr()?) } else { None };
                self.k.expect(T::FatArrow)?;
                let body = self.expr()?;
                Ok(Arm { pat, guard, body, span: self.span_from(start) })
            })();
            match arm {
                Ok(a) => {
                    let block_like = a.body.kind.is_block_like();
                    arms.push(a);
                    if !self.k.eat(T::Comma) && !block_like && !self.k.at(T::RBrace) {
                        let _ = self.k.error_expected();
                        self.k.recover_until(&[T::Comma, T::RBrace], NEST);
                        self.k.eat(T::Comma);
                    }
                }
                Err(Fatal) => {
                    self.k.recover_until(&[T::Comma, T::RBrace], NEST);
                    self.k.eat(T::Comma);
                }
            }
        }
    }

    // ----- blocks & statements -----

    pub(crate) fn block(&mut self) -> PResult<Block> {
        let start = self.start();
        self.k.expect(T::LBrace)?;
        let mut stmts = Vec::new();
        let mut tail = None;
        loop {
            if self.k.eat(T::RBrace) {
                break;
            }
            if self.k.at_eof() {
                self.k.expect_note(Expected::Token(T::RBrace));
                return Err(self.k.error_expected());
            }
            if self.k.eat(T::Semi) {
                continue;
            }
            match self.stmt() {
                Ok(StmtOrTail::Stmt(s)) => stmts.push(s),
                Ok(StmtOrTail::Tail(e)) => {
                    tail = Some(Box::new(e));
                    self.k.expect(T::RBrace)?;
                    break;
                }
                Err(Fatal) => {
                    let before = self.k.position();
                    self.k.recover_until(&[T::Semi, T::RBrace, T::Let], NEST);
                    if !self.k.eat(T::Semi) && self.k.position() == before && !self.k.at(T::RBrace) && !self.k.at_eof() {
                        // A stray closer like `)`: skip it to guarantee progress.
                        self.k.bump();
                    }
                }
            }
        }
        let id = self.id();
        Ok(Block { stmts, tail, span: self.span_from(start), id })
    }

    fn stmt(&mut self) -> PResult<StmtOrTail> {
        let start = self.start();
        if self.k.at(T::Let) {
            self.k.bump();
            let pat = self.pat()?;
            let ty = if self.k.eat(T::Colon) { Some(self.ty()?) } else { None };
            let init = if self.k.eat(T::Eq) { Some(self.expr()?) } else { None };
            self.expect_semi()?;
            return Ok(StmtOrTail::Stmt(Stmt { kind: StmtKind::Let { pat, ty, init }, span: self.span_from(start) }));
        }
        if self.at_item_start() {
            let item = self.item()?;
            return Ok(StmtOrTail::Stmt(Stmt { kind: StmtKind::Item(Box::new(item)), span: self.span_from(start) }));
        }
        // A statement starting with a block-like expression ends with it, so
        // `if c { } *p = 1;` is two statements, not a multiplication.
        let block_start = matches!(
            self.k.peek(),
            Some(T::If | T::Match | T::While | T::Loop | T::For | T::LBrace)
        ) || (self.k.at(T::Unsafe) && self.k.nth_at(1, T::LBrace));
        let e = if block_start {
            let e = self.primary(Restr::default())?;
            if self.k.at(T::Dot) || self.k.at(T::Question) || self.k.at(T::As) {
                // `match x { .. }.method()` style: keep parsing as an expression.
                self.continue_expr(e, start)?
            } else {
                e
            }
        } else {
            self.expr()?
        };
        if self.k.eat(T::Semi) {
            return Ok(StmtOrTail::Stmt(Stmt { kind: StmtKind::Expr(e), span: self.span_from(start) }));
        }
        if self.k.at(T::RBrace) {
            return Ok(StmtOrTail::Tail(e));
        }
        if e.kind.is_block_like() {
            return Ok(StmtOrTail::Stmt(Stmt { kind: StmtKind::Expr(e), span: self.span_from(start) }));
        }
        let at = e.span.shrink_to_hi();
        self.error(Diagnostic::error(syntax::MISSING_SEMI, at, "expected `;` after expression").with_label("add `;` here"));
        Ok(StmtOrTail::Stmt(Stmt { kind: StmtKind::Expr(e), span: self.span_from(start) }))
    }

    fn expect_semi(&mut self) -> PResult<()> {
        if self.k.eat(T::Semi) {
            return Ok(());
        }
        let at = self.span(self.k.prev_end(), self.k.prev_end());
        self.error(Diagnostic::error(syntax::MISSING_SEMI, at, "expected `;`").with_label("add `;` here"));
        Ok(())
    }
}

enum StmtOrTail {
    Stmt(Stmt),
    Tail(Expr),
}
