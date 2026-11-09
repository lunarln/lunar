//! Type checking of expressions, statements and blocks.

use super::{FnCtx, lang_trait_ref};
use crate::defs::*;
use crate::lower::{DeferKind, TyPos};
use crate::thir::{self, Arm, Callee, Coercion, Expr, ExprKind, FmtPiece, LocalKind, PatKind, ReadMode};
use crate::ty::{ArrKind, IntTy, TraitRef, Ty};
use ln_ast as ast;
use ln_ast::{BinOp, UnOp};
use ln_diag::{Diagnostic, Span, codes::name, codes::ty as tyc};

impl<'s, 'a> FnCtx<'s, 'a> {
    pub(crate) fn mk(&self, kind: ExprKind, ty: Ty, span: Span) -> Expr {
        Expr { kind, ty, span }
    }

    pub(crate) fn error_expr(&self, span: Span) -> Expr {
        Expr { kind: ExprKind::Error, ty: Ty::Error, span }
    }

    pub(crate) fn unit_expr(&self, span: Span) -> Expr {
        Expr { kind: ExprKind::Tuple(Vec::new()), ty: Ty::unit(), span }
    }

    /// Lower a type annotation inside a body.
    pub(crate) fn lower_local_ty(&mut self, t: &ast::Ty) -> Ty {
        let cx = self.lower_cx();
        let before = self.s.deferred.len();
        let ty = self.s.lower_ty(&cx, t, TyPos::Other);
        for d in self.s.deferred.drain(before..).collect::<Vec<_>>() {
            if let DeferKind::ArrayElem = d.kind {
                self.dull_checks.push((d.ty, d.span));
            }
        }
        self.instantiate(&ty, t.span)
    }

    // ===================================================================== entry points

    /// A value expression whose result is consumed (moved if it is an owner).
    pub(crate) fn check_expr(&mut self, e: &ast::Expr, expect: Option<&Ty>) -> Expr {
        self.check_expr_mode(e, expect, ReadMode::Move)
    }

    /// A value expression passed to a callee: owners are borrowed, not moved.
    pub(crate) fn check_expr_borrow(&mut self, e: &ast::Expr, expect: Option<&Ty>) -> Expr {
        self.check_expr_mode(e, expect, ReadMode::Borrow)
    }

    /// Check `e` and coerce it to `ty`.
    pub(crate) fn check_expr_coerce(&mut self, e: &ast::Expr, ty: &Ty) -> Expr {
        let x = self.check_expr(e, Some(ty));
        self.coerce(x, ty, e.span)
    }

    pub(crate) fn check_expr_mode(&mut self, e: &ast::Expr, expect: Option<&Ty>, mode: ReadMode) -> Expr {
        if let Some(place) = self.check_place(e) {
            if matches!(place.kind, ExprKind::Static(_)) {
                self.static_access(&place, e.span);
            }
            return self.read(place, mode);
        }
        self.check_value(e, expect, mode)
    }

    /// A place if `e` denotes one, otherwise its value materialized in a temporary.
    pub(crate) fn check_place_or_temp(&mut self, e: &ast::Expr) -> Expr {
        match self.check_place(e) {
            Some(p) => p,
            None => {
                let v = self.check_value(e, None, ReadMode::Move);
                self.expr_as_place(v)
            }
        }
    }

    fn static_access(&mut self, place: &Expr, span: Span) {
        if let ExprKind::Static(d) = place.kind
            && let DefData::Static(s) = &self.s.p.def(d).data
            && s.mutable
        {
            self.require_unsafe(span, "accessing a `static mut`");
        }
    }

    // ===================================================================== places

    pub(crate) fn check_place(&mut self, e: &ast::Expr) -> Option<Expr> {
        match &e.kind {
            ast::ExprKind::Paren(x) => self.check_place(x),
            ast::ExprKind::Path(p) if p.is_single() => {
                let n = &p.segments[0].ident.name;
                if let Some(l) = self.lookup_local(n) {
                    self.record_local(p.segments[0].ident.span, l);
                    let ty = self.locals[l.0 as usize].ty.clone();
                    return Some(self.mk(ExprKind::Local(l), ty, e.span));
                }
                self.static_place(p, e.span)
            }
            ast::ExprKind::Path(p) => self.static_place(p, e.span),
            ast::ExprKind::Field(base, field) => {
                let b = self.check_place_or_temp(base);
                Some(self.field_place(b, field))
            }
            ast::ExprKind::TupleField(base, idx, sp) => {
                let b = self.check_place_or_temp(base);
                let bt = self.shallow(&b.ty);
                let bt = bt.strip_bang().clone();
                match bt {
                    Ty::Tuple(ts) => match ts.get(*idx as usize) {
                        Some(t) => Some(self.mk(ExprKind::Field { base: Box::new(b), variant: 0, field: *idx as usize }, t.clone(), e.span)),
                        None => {
                            self.err(Diagnostic::error(tyc::NO_FIELD, *sp, format!("tuple has no field `{idx}`")));
                            Some(self.error_expr(e.span))
                        }
                    },
                    Ty::Adt(d, args) if !self.s.p.is_enum(d) => {
                        let fields = self.s.p.adt(d).struct_fields().to_vec();
                        match fields.get(*idx as usize) {
                            Some(f) if f.name == idx.to_string() => {
                                let t = f.ty.subst(&args);
                                Some(self.mk(ExprKind::Field { base: Box::new(b), variant: 0, field: *idx as usize }, t, e.span))
                            }
                            _ => {
                                self.err(Diagnostic::error(tyc::NO_FIELD, *sp, format!("no field `{idx}` on type `{}`", self.ty_str(&Ty::Adt(d, args)))));
                                Some(self.error_expr(e.span))
                            }
                        }
                    }
                    Ty::Error => Some(self.error_expr(e.span)),
                    t => {
                        if self.is_unbound_var(&t) {
                            self.annotations_needed(base.span);
                        } else {
                            self.err(Diagnostic::error(tyc::NO_FIELD, *sp, format!("type `{}` has no tuple fields", self.ty_str(&t))));
                        }
                        Some(self.error_expr(e.span))
                    }
                }
            }
            ast::ExprKind::Index(base, idx) if !matches!(idx.kind, ast::ExprKind::Range { .. }) => {
                let b = self.check_place_or_temp(base);
                let bt = self.shallow(&b.ty);
                let bt = bt.strip_bang().clone();
                let elem = match &bt {
                    Ty::Array(el, _) => (**el).clone(),
                    Ty::Error => Ty::Error,
                    Ty::Str | Ty::String => {
                        self.err(
                            Diagnostic::error(tyc::NOT_INDEXABLE, e.span, "strings cannot be indexed by position")
                                .with_help("use `.as_bytes()[i]` for a byte, or slice with a range: `s[a..b]`"),
                        );
                        Ty::Error
                    }
                    t => {
                        if self.is_unbound_var(t) {
                            self.annotations_needed(base.span);
                        } else {
                            self.err(Diagnostic::error(tyc::NOT_INDEXABLE, e.span, format!("cannot index into a value of type `{}`", self.ty_str(t))));
                        }
                        Ty::Error
                    }
                };
                let i = self.check_expr(idx, Some(&Ty::usize()));
                let i = self.coerce(i, &Ty::usize(), idx.span);
                Some(self.mk(ExprKind::Index { base: Box::new(b), index: Box::new(i) }, elem, e.span))
            }
            ast::ExprKind::Unary(UnOp::Deref, inner) => {
                let p = self.check_expr(inner, None);
                let pt = self.shallow(&p.ty);
                match pt {
                    Ty::Ptr(t, _) => {
                        self.require_unsafe(e.span, "dereferencing a raw pointer");
                        Some(self.mk(ExprKind::Deref(Box::new(p)), *t, e.span))
                    }
                    Ty::Error => Some(self.error_expr(e.span)),
                    t => {
                        self.err(
                            Diagnostic::error(tyc::BAD_OPERAND, e.span, format!("type `{}` cannot be dereferenced", self.ty_str(&t)))
                                .with_note("only raw pointers (`*T`, `*mut T`) can be dereferenced"),
                        );
                        Some(self.error_expr(e.span))
                    }
                }
            }
            _ => None,
        }
    }

    fn static_place(&mut self, p: &ast::Path, span: Span) -> Option<Expr> {
        let idents: Vec<ast::Ident> = p.segments.iter().map(|s| s.ident.clone()).collect();
        let module = self.module();
        // Silent lookup: only statics become places.
        let def = if idents.len() == 1 {
            match self.s.lookup_unqualified(module, &idents[0].name, Ns::Value) {
                Some(crate::resolve::ModRes::Def(d)) => d,
                _ => return None,
            }
        } else {
            let saved = self.s.diags.len();
            let r = self.s.resolve_prefix(module, &idents, Ns::Value);
            if self.s.diags.len() != saved {
                return None;
            }
            match r {
                Some((crate::resolve::ModRes::Def(d), n)) if n == idents.len() => d,
                _ => return None,
            }
        };
        match &self.s.p.def(def).data {
            DefData::Static(s) => {
                let ty = s.ty.clone();
                Some(self.mk(ExprKind::Static(def), ty, span))
            }
            _ => None,
        }
    }

    pub(crate) fn field_place(&mut self, b: Expr, field: &ast::Ident) -> Expr {
        let span = b.span.to(field.span);
        let bt = self.shallow(&b.ty);
        let bt = bt.strip_bang().clone();
        match &bt {
            Ty::Adt(d, args) if !self.s.p.is_enum(*d) => {
                let adt = self.s.p.adt(*d);
                let fields = adt.struct_fields().to_vec();
                if let Some((i, f)) = fields.iter().enumerate().find(|(_, f)| f.name == field.name) {
                    let owner_mod = self.s.p.def(*d).module;
                    if !f.is_pub && !self.s.is_ancestor_or_self(owner_mod, self.module()) {
                        self.err(
                            Diagnostic::error(name::PRIVATE, field.span, format!("field `{}` of `{}` is private", field.name, self.s.p.def(*d).name))
                                .with_secondary(f.span, "declared here"),
                        );
                    }
                    let t = f.ty.subst(args);
                    self.s.p.refs.push(crate::Ref { span: field.span, target: crate::RefTarget::Field { adt: *d, variant: 0, field: i } });
                    return self.mk(ExprKind::Field { base: Box::new(b), variant: 0, field: i }, t, span);
                }
                let names: Vec<String> = fields.iter().map(|f| f.name.clone()).collect();
                let mut d = Diagnostic::error(tyc::NO_FIELD, field.span, format!("no field `{}` on type `{}`", field.name, self.ty_str(&bt)));
                if !names.is_empty() {
                    d = d.with_note(format!("available fields: {}", names.join(", ")));
                }
                self.err(d);
                self.error_expr(span)
            }
            Ty::Error => self.error_expr(span),
            t => {
                if self.is_unbound_var(t) {
                    self.annotations_needed(b.span);
                } else {
                    let mut d = Diagnostic::error(tyc::NO_FIELD, field.span, format!("no field `{}` on type `{}`", field.name, self.ty_str(t)));
                    if matches!(t, Ty::Array(..) | Ty::Str | Ty::String) && field.name == "len" {
                        d = d.with_help("call it as a method: `.len()`");
                    }
                    self.err(d);
                }
                self.error_expr(span)
            }
        }
    }

    // ===================================================================== values

    fn check_value(&mut self, e: &ast::Expr, expect: Option<&Ty>, mode: ReadMode) -> Expr {
        use ast::ExprKind as K;
        let span = e.span;
        match &e.kind {
            K::Error => self.error_expr(span),
            K::Lit(l) => self.check_lit(l, expect, span),
            K::Paren(x) => self.check_expr_mode(x, expect, mode),
            K::Path(p) => self.check_path_value(p, expect, span),
            K::Unary(op, x) => self.check_unary(*op, x, expect, span),
            K::Binary(op, a, b) => self.check_binary(*op, a, b, expect, span),
            K::Assign(lhs, rhs) => self.check_assign(lhs, rhs, span),
            K::AssignOp(op, lhs, rhs) => self.check_assign_op(*op, lhs, rhs, span),
            K::Call(f, args) => self.check_call(f, args, expect, span),
            K::MethodCall { recv, name, generics, args } => self.check_method_call(recv, name, generics.as_deref(), args, span),
            K::Index(base, idx) => {
                let ast::ExprKind::Range { start, end, inclusive } = &idx.kind else { unreachable!("non-range index handled as place") };
                self.check_slice(base, start.as_deref(), end.as_deref(), *inclusive, expect, span)
            }
            K::Range { start, end, inclusive } => self.check_range_value(start.as_deref(), end.as_deref(), *inclusive, expect, span),
            K::Array(elems) => self.check_array(elems, expect, span),
            K::ArrayRepeat(v, n) => self.check_array_repeat(v, n, expect, span),
            K::Tuple(es) => {
                let exp: Option<Vec<Ty>> = expect.map(|t| self.shallow(t)).and_then(|t| match t {
                    Ty::Tuple(ts) if ts.len() == es.len() => Some(ts),
                    _ => None,
                });
                let mut out = Vec::new();
                let mut tys = Vec::new();
                for (i, x) in es.iter().enumerate() {
                    let et = exp.as_ref().map(|v| v[i].clone());
                    let v = self.check_expr(x, et.as_ref());
                    let v = match &et {
                        Some(t) => self.coerce(v, t, x.span),
                        None => v,
                    };
                    tys.push(v.ty.clone());
                    out.push(v);
                }
                self.mk(ExprKind::Tuple(out), Ty::Tuple(tys), span)
            }
            K::StructLit { path, fields } => self.check_struct_lit(path, fields, expect, span),
            K::Block(b) => self.check_block_expr(b, expect, false),
            K::Unsafe(b) => {
                self.unsafe_depth += 1;
                let r = self.check_block_expr(b, expect, false);
                self.unsafe_depth -= 1;
                r
            }
            K::If { cond, then, else_ } => self.check_if(cond, then, else_.as_deref(), expect, span),
            K::IfLet { pat, scrut, then, else_ } => self.check_if_let(pat, scrut, then, else_.as_deref(), expect, span),
            K::Match { scrut, arms } => self.check_match(scrut, arms, expect, span),
            K::While { cond, body } => {
                let c = self.check_expr_coerce(cond, &Ty::Bool);
                let id = self.new_loop(false);
                let b = self.check_block(body, Some(&Ty::unit()));
                self.loops.pop();
                self.mk(ExprKind::While { cond: Box::new(c), body: b, id }, Ty::unit(), span)
            }
            K::WhileLet { pat, scrut, body } => self.check_while_let(pat, scrut, body, span),
            K::Loop(body) => {
                let id = self.new_loop(true);
                let b = self.check_block(body, Some(&Ty::unit()));
                let ctx = self.loops.pop().unwrap();
                let ty = if ctx.saw_break { ctx.break_ty.unwrap_or_else(Ty::unit) } else { Ty::Never };
                self.mk(ExprKind::Loop { body: b, id }, ty, span)
            }
            K::For { pat, iter, body } => self.check_for(pat, iter, body, span),
            K::Break(v) => {
                let Some(idx) = self.loops.len().checked_sub(1) else {
                    self.err(Diagnostic::error(tyc::BREAK_OUTSIDE_LOOP, span, "`break` outside of a loop"));
                    return self.mk(ExprKind::Error, Ty::Never, span);
                };
                let id = self.loops[idx].id;
                self.loops[idx].saw_break = true;
                let value = match v {
                    Some(v) => {
                        if !self.loops[idx].is_loop {
                            self.err(Diagnostic::error(tyc::BREAK_OUTSIDE_LOOP, v.span, "`break` with a value is only allowed inside `loop`"));
                        }
                        let bt = self.loops[idx].break_ty.clone();
                        let x = self.check_expr(v, bt.as_ref());
                        let x = match bt {
                            Some(t) => self.coerce(x, &t, v.span),
                            None => {
                                self.loops[idx].break_ty = Some(x.ty.clone());
                                x
                            }
                        };
                        Some(Box::new(x))
                    }
                    None => {
                        if self.loops[idx].is_loop {
                            match self.loops[idx].break_ty.clone() {
                                Some(t) => {
                                    if !self.unify(&t, &Ty::unit()) {
                                        self.mismatch(&t, &Ty::unit(), span);
                                    }
                                }
                                None => self.loops[idx].break_ty = Some(Ty::unit()),
                            }
                        }
                        None
                    }
                };
                self.mk(ExprKind::Break { id, value }, Ty::Never, span)
            }
            K::Continue => {
                let Some(ctx) = self.loops.last() else {
                    self.err(Diagnostic::error(tyc::BREAK_OUTSIDE_LOOP, span, "`continue` outside of a loop"));
                    return self.mk(ExprKind::Error, Ty::Never, span);
                };
                let id = ctx.id;
                self.mk(ExprKind::Continue { id }, Ty::Never, span)
            }
            K::Return(v) => {
                let ret = self.ret_ty.clone();
                let value = match v {
                    Some(v) => Some(Box::new(self.check_expr_coerce(v, &ret))),
                    None => {
                        if !self.unify(&ret, &Ty::unit()) {
                            let r = self.ty_str(&ret);
                            self.err(Diagnostic::error(tyc::MISMATCH, span, format!("`return` without a value in a function returning `{r}`")));
                        }
                        None
                    }
                };
                self.mk(ExprKind::Return(value), Ty::Never, span)
            }
            K::Cast(x, t) => self.check_cast(x, t, span),
            K::Try(x) => self.check_try(x, span),
            K::Field(..) | K::TupleField(..) => unreachable!("handled as places"),
        }
    }

    // ===================================================================== literals

    pub(crate) fn check_lit(&mut self, l: &ast::Lit, expect: Option<&Ty>, span: Span) -> Expr {
        let exp = expect.map(|t| self.shallow(t));
        match l {
            ast::Lit::Int { value, suffix } => {
                let ty = match suffix.as_deref() {
                    Some(s) => crate::resolve::prim_type(s).unwrap_or(Ty::Error),
                    None => match &exp {
                        Some(t @ Ty::Int(_)) => t.clone(),
                        Some(t) if self.is_int_var(t) => t.clone(),
                        Some(Ty::Float(f)) => {
                            // `let x: f64 = 1;` is accepted as `1.0`.
                            return self.mk(ExprKind::Lit(ConstVal::Float(*value as f64)), Ty::Float(*f), span);
                        }
                        _ => self.int_var(span),
                    },
                };
                if let Ty::Float(_) = ty {
                    return self.mk(ExprKind::Lit(ConstVal::Float(*value as f64)), ty, span);
                }
                if *value > i128::MAX as u128 {
                    self.err(Diagnostic::error(tyc::LITERAL_RANGE, span, "integer literal is too large"));
                }
                self.lit_checks.push((ty.clone(), *value as i128, span));
                self.mk(ExprKind::Lit(ConstVal::Int(*value as i128)), ty, span)
            }
            ast::Lit::Float { value, suffix } => {
                let ty = match suffix.as_deref() {
                    Some(s) => crate::resolve::prim_type(s).unwrap_or(Ty::Error),
                    None => match &exp {
                        Some(t @ Ty::Float(_)) => t.clone(),
                        Some(t) if self.is_float_var(t) => t.clone(),
                        _ => self.float_var(span),
                    },
                };
                self.mk(ExprKind::Lit(ConstVal::Float(*value)), ty, span)
            }
            ast::Lit::Bool(b) => self.mk(ExprKind::Lit(ConstVal::Bool(*b)), Ty::Bool, span),
            ast::Lit::Char(c) => self.mk(ExprKind::Lit(ConstVal::Char(*c)), Ty::Char, span),
            ast::Lit::Str { value, fmt } => {
                if matches!(exp, Some(Ty::FmtArgs)) {
                    return self.check_fmt(fmt, span);
                }
                self.mk(ExprKind::Lit(ConstVal::Str(value.clone())), Ty::Str, span)
            }
        }
    }

    /// A string literal converted to `FmtArgs`.
    fn check_fmt(&mut self, fmt: &ast::FmtString, span: Span) -> Expr {
        let pieces = match fmt {
            ast::FmtString::Ok(p) => p,
            ast::FmtString::Err(msg, sp) => {
                self.err(Diagnostic::error(tyc::INTERPOLATION, *sp, msg.clone()));
                return self.mk(ExprKind::Error, Ty::FmtArgs, span);
            }
        };
        let display = lang_trait_ref(&self.s.p, "display");
        let mut out = Vec::new();
        for p in pieces {
            match p {
                ast::FmtPiece::Text(t) => out.push(FmtPiece::Text(t.clone())),
                ast::FmtPiece::Expr(e) => {
                    let v = self.check_expr_borrow(e, None);
                    let v = self.auto_read_bang(v);
                    if let Some(d) = &display {
                        self.oblige(v.ty.clone(), d.clone(), e.span);
                    }
                    out.push(FmtPiece::Arg(v));
                }
            }
        }
        self.mk(ExprKind::FmtArgs(out), Ty::FmtArgs, span)
    }

    fn auto_read_bang(&mut self, v: Expr) -> Expr {
        v
    }

    // ===================================================================== operators

    fn check_unary(&mut self, op: UnOp, x: &ast::Expr, expect: Option<&Ty>, span: Span) -> Expr {
        match op {
            UnOp::Deref => unreachable!("deref is a place"),
            UnOp::Neg => {
                // Fold `-literal` so `-128i8` range-checks correctly.
                if let ast::ExprKind::Lit(ast::Lit::Int { value, .. }) = &x.kind {
                    let mut lit = self.check_lit(
                        match &x.kind {
                            ast::ExprKind::Lit(l) => l,
                            _ => unreachable!(),
                        },
                        expect,
                        x.span,
                    );
                    if let ExprKind::Lit(ConstVal::Int(_)) = lit.kind {
                        lit.kind = ExprKind::Lit(ConstVal::Int(-(*value as i128)));
                        if let Some(last) = self.lit_checks.last_mut() {
                            last.1 = -(*value as i128);
                            last.2 = span;
                        }
                        if let Ty::Int(it) = self.shallow(&lit.ty)
                            && !it.signed()
                        {
                            self.err(Diagnostic::error(tyc::BAD_OPERAND, span, format!("cannot negate unsigned type `{}`", it.name())));
                        }
                        lit.span = span;
                        return lit;
                    }
                    if let ExprKind::Lit(ConstVal::Float(f)) = lit.kind {
                        lit.kind = ExprKind::Lit(ConstVal::Float(-f));
                        lit.span = span;
                        return lit;
                    }
                }
                let v = self.check_expr(x, expect);
                let t = self.shallow(&v.ty);
                let ok_prim = matches!(t, Ty::Int(i) if i.signed()) || matches!(t, Ty::Float(_)) || self.is_int_var(&t) || self.is_float_var(&t);
                if ok_prim {
                    let ty = v.ty.clone();
                    return self.mk(ExprKind::Unary(UnOp::Neg, Box::new(v)), ty, span);
                }
                self.op_trait_call("neg", "neg", v, None, span)
            }
            UnOp::Not => {
                let v = self.check_expr(x, expect);
                let t = self.shallow(&v.ty);
                if matches!(t, Ty::Bool | Ty::Int(_)) || self.is_int_var(&t) {
                    let ty = v.ty.clone();
                    return self.mk(ExprKind::Unary(UnOp::Not, Box::new(v)), ty, span);
                }
                self.op_trait_call("not", "not", v, None, span)
            }
        }
    }

    fn is_prim_operand(&self, t: &Ty) -> bool {
        let t = self.shallow(t);
        t.is_primitive_scalar() || self.is_int_var(&t) || self.is_float_var(&t) || matches!(t, Ty::Error | Ty::Never)
    }

    pub(crate) fn binop_trait(op: BinOp) -> (&'static str, &'static str) {
        match op {
            BinOp::Add => ("add", "add"),
            BinOp::Sub => ("sub", "sub"),
            BinOp::Mul => ("mul", "mul"),
            BinOp::Div => ("div", "div"),
            BinOp::Rem => ("rem", "rem"),
            BinOp::BitAnd => ("bitand", "bitand"),
            BinOp::BitOr => ("bitor", "bitor"),
            BinOp::BitXor => ("bitxor", "bitxor"),
            BinOp::Shl => ("shl", "shl"),
            BinOp::Shr => ("shr", "shr"),
            BinOp::Eq => ("eq", "eq"),
            BinOp::Ne => ("eq", "ne"),
            BinOp::Lt => ("ord", "lt"),
            BinOp::Le => ("ord", "le"),
            BinOp::Gt => ("ord", "gt"),
            BinOp::Ge => ("ord", "ge"),
            BinOp::And | BinOp::Or => ("", ""),
        }
    }

    fn check_binary(&mut self, op: BinOp, a: &ast::Expr, b: &ast::Expr, expect: Option<&Ty>, span: Span) -> Expr {
        if matches!(op, BinOp::And | BinOp::Or) {
            let l = self.check_expr_coerce(a, &Ty::Bool);
            let r = self.check_expr_coerce(b, &Ty::Bool);
            return self.mk(ExprKind::Logical { and: op == BinOp::And, lhs: Box::new(l), rhs: Box::new(r) }, Ty::Bool, span);
        }
        let lhs_expect = if op.is_comparison() { None } else { expect };
        let l = self.check_expr_borrow(a, lhs_expect);
        let lt = self.shallow(&l.ty);
        if self.is_prim_operand(&lt) && !matches!(lt, Ty::Str) {
            let r = self.check_expr_borrow(b, Some(&lt));
            let r = self.coerce(r, &lt, b.span);
            let t = self.shallow(&lt);
            let valid = match op {
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => t.is_numeric() || self.is_int_var(&t) || self.is_float_var(&t),
                BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => t.is_integral() || matches!(t, Ty::Bool) || self.is_int_var(&t),
                BinOp::Shl | BinOp::Shr => t.is_integral() || self.is_int_var(&t),
                BinOp::Eq | BinOp::Ne => true,
                BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => !matches!(t, Ty::Bool),
                _ => false,
            } || matches!(t, Ty::Error | Ty::Never);
            if !valid {
                self.err(Diagnostic::error(tyc::BAD_OPERAND, span, format!("operator `{}` cannot be applied to `{}`", op.as_str(), self.ty_str(&t))));
            }
            let ty = if op.is_comparison() { Ty::Bool } else { lt };
            return self.mk(ExprKind::Binary(op, Box::new(l), Box::new(r)), ty, span);
        }
        let (lang, method) = Self::binop_trait(op);
        if self.is_unbound_var(&lt) {
            self.annotations_needed(a.span);
            return self.error_expr(span);
        }
        let r = self.check_expr_borrow(b, Some(&lt));
        let r = self.coerce_arg(r, &lt, b.span);
        let call = self.op_trait_call(lang, method, l, Some(r), span);
        if op == BinOp::Ne {
            // `a != b` is `!(a == b)`.
            return self.mk(ExprKind::Unary(UnOp::Not, Box::new(call)), Ty::Bool, span);
        }
        call
    }

    /// Call an operator trait method on a non-primitive operand.
    fn op_trait_call(&mut self, lang: &str, method: &str, lhs: Expr, rhs: Option<Expr>, span: Span) -> Expr {
        let Some(tr) = lang_trait_ref(&self.s.p, lang) else {
            self.err(Diagnostic::error(tyc::BAD_OPERAND, span, format!("operator trait `{lang}` is not available (missing lang item)")));
            return self.error_expr(span);
        };
        let self_ty = self.resolve(&lhs.ty);
        if self_ty.is_error() {
            return self.error_expr(span);
        }
        if !self_ty.has_infer() && !self.s.p.implements(&self_ty, &tr, &self.env) {
            let ts = self.s.p.ty_str(&self_ty);
            let tn = self.s.p.def(tr.trait_id).name.clone();
            self.err(
                Diagnostic::error(tyc::BAD_OPERAND, span, format!("cannot apply this operator to `{ts}`"))
                    .with_note(format!("`{ts}` does not implement `{tn}`"))
                    .with_help(if matches!(self_ty, Ty::Param(..)) {
                        format!("add `{tn}` to the parameter's bounds")
                    } else {
                        format!("implement `{tn}` for `{ts}`")
                    }),
            );
            return self.error_expr(span);
        }
        self.oblige(self_ty.clone(), tr.clone(), span);
        let ret = match lang {
            "eq" | "ord" => Ty::Bool,
            _ => self_ty.clone(),
        };
        let mut args = vec![thir::Arg::Value(lhs)];
        if let Some(r) = rhs {
            args.push(thir::Arg::Value(r));
        }
        let callee = Callee::Trait { trait_ref: tr, self_ty, method: method.to_string(), method_args: Vec::new() };
        self.mk(ExprKind::Call { callee, args }, ret, span)
    }

    fn check_assign(&mut self, lhs: &ast::Expr, rhs: &ast::Expr, span: Span) -> Expr {
        let Some(place) = self.check_place(lhs) else {
            let _ = self.check_expr(lhs, None);
            self.err(
                Diagnostic::error(tyc::NOT_A_PLACE, lhs.span, "invalid left-hand side of assignment")
                    .with_note("only variables, fields, indexes and `*ptr` can be assigned"),
            );
            let _ = self.check_expr(rhs, None);
            return self.unit_expr(span);
        };
        if matches!(place.kind, ExprKind::Static(_)) {
            self.static_access(&place, lhs.span);
        }
        if !self.is_mutable_place(&place) && !place.ty.is_error() {
            self.immutable_err(&place, lhs.span, "assign to this place");
        }
        let pty = place.ty.clone();
        let pty = pty.strip_bang().clone();
        let v = self.check_expr(rhs, Some(&pty));
        let v = self.coerce(v, &pty, rhs.span);
        self.mk(ExprKind::Assign { place: Box::new(place), value: Box::new(v) }, Ty::unit(), span)
    }

    fn check_assign_op(&mut self, op: BinOp, lhs: &ast::Expr, rhs: &ast::Expr, span: Span) -> Expr {
        let Some(place) = self.check_place(lhs) else {
            self.err(Diagnostic::error(tyc::NOT_A_PLACE, lhs.span, "invalid left-hand side of compound assignment"));
            return self.unit_expr(span);
        };
        if matches!(place.kind, ExprKind::Static(_)) {
            self.static_access(&place, lhs.span);
        }
        if !self.is_mutable_place(&place) && !place.ty.is_error() {
            self.immutable_err(&place, lhs.span, "assign to this place");
        }
        let pty = self.shallow(&place.ty);
        if self.is_prim_operand(&pty) {
            let v = self.check_expr(rhs, Some(&pty));
            let v = self.coerce(v, &pty, rhs.span);
            return self.mk(ExprKind::AssignOp { op, place: Box::new(place), value: Box::new(v) }, Ty::unit(), span);
        }
        // `a op= b` on a non-primitive is `a = a.op(b)`.
        let (lang, method) = Self::binop_trait(op);
        let cur = self.read(place.clone(), ReadMode::Borrow);
        let r = self.check_expr_borrow(rhs, Some(&pty));
        let r = self.coerce_arg(r, &pty, rhs.span);
        let v = self.op_trait_call(lang, method, cur, Some(r), span);
        self.mk(ExprKind::Assign { place: Box::new(place), value: Box::new(v) }, Ty::unit(), span)
    }

    fn check_cast(&mut self, x: &ast::Expr, t: &ast::Ty, span: Span) -> Expr {
        let target = self.lower_local_ty(t);
        let v = self.check_expr(x, None);
        let from = self.default_var(&v.ty);
        let to = self.shallow(&target);
        let ok = match (&from, &to) {
            (Ty::Error, _) | (_, Ty::Error) => true,
            (Ty::Int(_) | Ty::Float(_), Ty::Int(_) | Ty::Float(_)) => true,
            (Ty::Bool, Ty::Int(_)) => true,
            (Ty::Char, Ty::Int(_)) => true,
            (Ty::Int(IntTy::U8), Ty::Char) => true,
            (Ty::Ptr(..), Ty::Ptr(..)) => true,
            (Ty::Ptr(..), Ty::Int(IntTy::Usize | IntTy::Isize)) => true,
            (Ty::Int(IntTy::Usize | IntTy::Isize), Ty::Ptr(..)) => true,
            (Ty::FnPtr(..), Ty::Ptr(..)) => true,
            (a, b) => a == b,
        };
        if !ok {
            let (f, tt) = (self.ty_str(&from), self.ty_str(&to));
            let mut d = Diagnostic::error(tyc::BAD_CAST, span, format!("cannot cast `{f}` as `{tt}`"));
            if matches!(to, Ty::Char) {
                d = d.with_help("only `u8` can be cast to `char`; use `char::from_u32` for other integers");
            }
            self.err(d);
        }
        self.mk(ExprKind::Cast(Box::new(v)), to, span)
    }

    // ===================================================================== arrays & ranges

    fn check_array(&mut self, elems: &[ast::Expr], expect: Option<&Ty>, span: Span) -> Expr {
        let exp = expect.map(|t| self.shallow(t));
        // `[]` is also a valid (empty) slice.
        if elems.is_empty()
            && let Some(Ty::Array(e, k)) = &exp
            && k.is_slice()
        {
            return self.mk(ExprKind::Array(Vec::new()), Ty::Array(e.clone(), *k), span);
        }
        let annotated = matches!(&exp, Some(Ty::Array(_, k)) if !k.is_slice());
        let (elem_ty, kind) = match &exp {
            Some(Ty::Array(e, k)) if !k.is_slice() => ((**e).clone(), *k),
            _ => (self.new_var(span), ArrKind::Fixed(elems.len() as u64)),
        };
        let mut out = Vec::new();
        for x in elems {
            let v = self.check_expr(x, Some(&elem_ty));
            out.push(self.coerce(v, &elem_ty, x.span));
        }
        let kind = match kind {
            ArrKind::Fixed(n) if n != elems.len() as u64 => {
                self.err(Diagnostic::error(tyc::ARRAY_LEN, span, format!("expected an array of {n} elements, found {}", elems.len())));
                ArrKind::Fixed(n)
            }
            ArrKind::FixedVar(v) => {
                let fixed = ArrKind::Fixed(elems.len() as u64);
                let ok = self.unify(&Ty::Array(Box::new(Ty::Error), ArrKind::FixedVar(v)), &Ty::Array(Box::new(Ty::Error), fixed));
                if !ok {
                    self.err(Diagnostic::error(tyc::ARRAY_LEN, span, "array length does not match the annotation"));
                }
                fixed
            }
            k => k,
        };
        if !annotated {
            self.dull_checks.push((elem_ty.clone(), span));
        }
        self.mk(ExprKind::Array(out), Ty::Array(Box::new(elem_ty), kind), span)
    }

    fn check_array_repeat(&mut self, v: &ast::Expr, n: &ast::Expr, expect: Option<&Ty>, span: Span) -> Expr {
        let exp = expect.map(|t| self.shallow(t));
        let (elem_ty, kind) = match &exp {
            Some(Ty::Array(e, k)) if !k.is_slice() => ((**e).clone(), Some(*k)),
            _ => (self.new_var(span), None),
        };
        let annotated = kind.is_some();
        let value = self.check_expr(v, Some(&elem_ty));
        let value = self.coerce(value, &elem_ty, v.span);
        let value = self.read_if_place(value);
        let kind = match kind {
            Some(k) if k.is_owner() => k,
            other => {
                // Fixed arrays need a constant count.
                let module = self.module();
                let count = match self.s.eval_const(module, n, &Ty::usize()) {
                    Some(ConstVal::Int(c)) if c >= 0 => c as u64,
                    _ => 0,
                };
                match other {
                    Some(ArrKind::Fixed(want)) if want != count => {
                        self.err(Diagnostic::error(tyc::ARRAY_LEN, span, format!("expected an array of {want} elements, found {count}")));
                    }
                    Some(ArrKind::FixedVar(var)) => {
                        let _ = self.unify(&Ty::Array(Box::new(Ty::Error), ArrKind::FixedVar(var)), &Ty::Array(Box::new(Ty::Error), ArrKind::Fixed(count)));
                    }
                    _ => {}
                }
                ArrKind::Fixed(count)
            }
        };
        let count = self.check_expr_coerce(n, &Ty::usize());
        if !annotated {
            self.dull_checks.push((elem_ty.clone(), span));
        }
        self.mk(ExprKind::ArrayRepeat { value: Box::new(value), count: Box::new(count) }, Ty::Array(Box::new(elem_ty), kind), span)
    }

    fn read_if_place(&mut self, e: Expr) -> Expr {
        if e.is_place() { self.read(e, ReadMode::Copy) } else { e }
    }

    /// `base[start..end]`
    fn check_slice(&mut self, base: &ast::Expr, start: Option<&ast::Expr>, end: Option<&ast::Expr>, inclusive: bool, expect: Option<&Ty>, span: Span) -> Expr {
        let b = self.check_place_or_temp(base);
        let bt = self.shallow(&b.ty);
        let bt = bt.strip_bang().clone();
        let want_mut = matches!(expect.map(|t| self.shallow(t)), Some(Ty::Array(_, ArrKind::SliceMut)));
        let ty = match &bt {
            Ty::Array(e, k) => {
                if want_mut {
                    let ok = match k {
                        ArrKind::SliceMut => true,
                        ArrKind::Slice => false,
                        _ => self.is_mutable_place(&b),
                    };
                    if !ok {
                        self.immutable_err(&b, base.span, "take a `[T; &mut]` slice of this array");
                    }
                    Ty::Array(e.clone(), ArrKind::SliceMut)
                } else {
                    Ty::Array(e.clone(), ArrKind::Slice)
                }
            }
            Ty::Str | Ty::String => Ty::Str,
            Ty::Error => Ty::Error,
            t => {
                if self.is_unbound_var(t) {
                    self.annotations_needed(base.span);
                } else {
                    self.err(Diagnostic::error(tyc::NOT_INDEXABLE, span, format!("cannot slice a value of type `{}`", self.ty_str(t))));
                }
                Ty::Error
            }
        };
        let s = start.map(|x| Box::new(self.check_expr_coerce(x, &Ty::usize())));
        let e = end.map(|x| Box::new(self.check_expr_coerce(x, &Ty::usize())));
        if inclusive && e.is_none() {
            self.err(Diagnostic::error(tyc::BAD_OPERAND, span, "inclusive ranges need an end: `a..=b`"));
        }
        let mutable = matches!(ty, Ty::Array(_, ArrKind::SliceMut));
        self.mk(ExprKind::Slice { base: Box::new(b), start: s, end: e, inclusive, mutable }, ty, span)
    }

    /// A range used as a value: `core::ops::Range::<T> { start, end }`.
    fn check_range_value(&mut self, start: Option<&ast::Expr>, end: Option<&ast::Expr>, inclusive: bool, expect: Option<&Ty>, span: Span) -> Expr {
        let Some(range) = self.s.p.lang_item("range") else {
            self.err(Diagnostic::error(tyc::BAD_OPERAND, span, "ranges are not available (missing lang item)"));
            return self.error_expr(span);
        };
        let (Some(s), Some(e)) = (start, end) else {
            self.err(Diagnostic::error(tyc::BAD_OPERAND, span, "open ranges are only allowed when slicing (`arr[a..]`)").with_help("give both ends: `a..b`"));
            return self.error_expr(span);
        };
        if inclusive {
            self.err(Diagnostic::error(tyc::BAD_OPERAND, span, "inclusive range values are not supported; use `a..b+1` or a `for` loop"));
        }
        let elem = match expect.map(|t| self.shallow(t)) {
            Some(Ty::Adt(d, args)) if d == range => args[0].clone(),
            _ => self.new_var(span),
        };
        let sv = self.check_expr(s, Some(&elem));
        let sv = self.coerce(sv, &elem, s.span);
        let ev = self.check_expr(e, Some(&elem));
        let ev = self.coerce(ev, &elem, e.span);
        let ty = Ty::Adt(range, vec![elem]);
        self.mk(
            ExprKind::Adt {
                def: range,
                args: match &ty {
                    Ty::Adt(_, a) => a.clone(),
                    _ => vec![],
                },
                variant: 0,
                fields: vec![(0, sv), (1, ev)],
            },
            ty,
            span,
        )
    }

    // ===================================================================== blocks

    /// Check a block and wrap it as an expression.
    pub(crate) fn check_block_expr(&mut self, b: &ast::Block, expect: Option<&Ty>, _fn_body: bool) -> Expr {
        let (blk, ty) = self.check_block_typed(b, expect);
        self.mk(ExprKind::Block(blk), ty, b.span)
    }

    /// Check a block whose value is discarded or unit.
    pub(crate) fn check_block(&mut self, b: &ast::Block, expect: Option<&Ty>) -> thir::Block {
        let (blk, ty) = self.check_block_typed(b, expect);
        if let Some(t) = expect
            && t.is_unit()
            && !matches!(self.shallow(&ty), Ty::Never | Ty::Error)
            && !self.unify(&ty, &Ty::unit())
        {
            let sp = b.tail.as_ref().map_or(b.span, |t| t.span);
            self.mismatch(&Ty::unit(), &ty, sp);
        }
        blk
    }

    fn check_block_typed(&mut self, b: &ast::Block, expect: Option<&Ty>) -> (thir::Block, Ty) {
        let module = self.s.block_scopes.get(&b.id).copied();
        self.push_scope(module);
        let first_local = self.locals.len();
        let mut stmts = Vec::new();
        let mut diverges = false;
        for s in &b.stmts {
            match &s.kind {
                ast::StmtKind::Item(_) => {}
                ast::StmtKind::Let { pat, ty, init } => {
                    let st = self.check_let(pat, ty.as_ref(), init.as_ref(), s.span);
                    if let thir::Stmt::Let { init: Some(i), .. } = &st
                        && matches!(self.shallow(&i.ty), Ty::Never)
                    {
                        diverges = true;
                    }
                    stmts.push(st);
                }
                ast::StmtKind::Expr(e) => {
                    let v = self.check_expr(e, None);
                    if matches!(self.shallow(&v.ty), Ty::Never) {
                        diverges = true;
                    }
                    stmts.push(thir::Stmt::Expr(v));
                }
            }
        }
        let tail = b.tail.as_ref().map(|t| {
            let v = self.check_expr(t, expect);
            Box::new(v)
        });
        let ty = match &tail {
            Some(t) => {
                if diverges && !matches!(self.shallow(&t.ty), Ty::Never) {
                    t.ty.clone()
                } else {
                    t.ty.clone()
                }
            }
            None if diverges => Ty::Never,
            None => Ty::unit(),
        };
        let depth = self.depth();
        let locals = (first_local..self.locals.len())
            .map(|i| thir::LocalId(i as u32))
            .filter(|l| self.locals[l.0 as usize].depth == depth && self.locals[l.0 as usize].kind == LocalKind::Let)
            .collect();
        self.pop_scope();
        (thir::Block { stmts, tail, locals, span: b.span }, ty)
    }

    fn check_let(&mut self, pat: &ast::Pat, ty: Option<&ast::Ty>, init: Option<&ast::Expr>, span: Span) -> thir::Stmt {
        let declared = ty.map(|t| self.lower_local_ty(t));
        let init = match init {
            Some(e) => {
                let v = self.check_expr(e, declared.as_ref());
                Some(match &declared {
                    Some(t) => self.coerce(v, t, e.span),
                    None => v,
                })
            }
            None => {
                self.err(
                    Diagnostic::error(tyc::ANNOTATIONS_NEEDED, span, "`let` bindings must be initialized").with_help("give it a value: `let x: T = ...;`"),
                );
                None
            }
        };
        let t = declared.or_else(|| init.as_ref().map(|i| i.ty.clone())).unwrap_or(Ty::Error);
        let p = self.check_pat(pat, &t, true);
        if !p.is_irrefutable(&|d| self.s.p.is_enum(d)) {
            self.err(
                Diagnostic::error(tyc::NON_EXHAUSTIVE, pat.span, "refutable pattern in `let`").with_help("use `if let` or `match` to handle the other cases"),
            );
        }
        thir::Stmt::Let { pat: p, init, span }
    }

    // ===================================================================== control flow

    fn check_if(&mut self, cond: &ast::Expr, then: &ast::Block, else_: Option<&ast::Expr>, expect: Option<&Ty>, span: Span) -> Expr {
        let c = self.check_expr_coerce(cond, &Ty::Bool);
        match else_ {
            None => {
                let t = self.check_block(then, Some(&Ty::unit()));
                let te = self.mk(ExprKind::Block(t), Ty::unit(), then.span);
                self.mk(ExprKind::If { cond: Box::new(c), then: Box::new(te), else_: None }, Ty::unit(), span)
            }
            Some(e) => {
                let t = self.check_block_expr(then, expect, false);
                let ev = self.check_expr(e, expect.or(Some(&t.ty)).cloned().as_ref());
                let (t, ev, ty) = self.join_branches(t, ev, expect, e.span);
                self.mk(ExprKind::If { cond: Box::new(c), then: Box::new(t), else_: Some(Box::new(ev)) }, ty, span)
            }
        }
    }

    /// Find the common type of two branches, coercing `!` branches.
    pub(crate) fn join_branches(&mut self, a: Expr, b: Expr, expect: Option<&Ty>, b_span: Span) -> (Expr, Expr, Ty) {
        let at = self.shallow(&a.ty);
        let bt = self.shallow(&b.ty);
        let target = match (&at, &bt) {
            (Ty::Never, Ty::Never) => return (a, b, Ty::Never),
            (Ty::Never, _) => bt.clone(),
            _ => expect.cloned().unwrap_or(at.clone()),
        };
        let a = self.coerce(a, &target, b_span);
        let b = self.coerce(b, &target, b_span);
        (a, b, target)
    }

    fn check_if_let(&mut self, pat: &ast::Pat, scrut: &ast::Expr, then: &ast::Block, else_: Option<&ast::Expr>, expect: Option<&Ty>, span: Span) -> Expr {
        let s = self.check_place_or_temp(scrut);
        let st = s.ty.clone();
        self.push_scope(None);
        let p = self.check_pat(pat, &st, false);
        let has_else = else_.is_some();
        let then_e = if has_else {
            self.check_block_expr(then, expect, false)
        } else {
            let b = self.check_block(then, Some(&Ty::unit()));
            self.mk(ExprKind::Block(b), Ty::unit(), then.span)
        };
        self.pop_scope();
        let else_e = match else_ {
            Some(e) => self.check_expr(e, Some(&then_e.ty.clone())),
            None => self.unit_expr(span),
        };
        let (then_e, else_e, ty) = if has_else { self.join_branches(then_e, else_e, expect, span) } else { (then_e, else_e, Ty::unit()) };
        let wild = thir::Pat { kind: PatKind::Wild, ty: st, span };
        let arms = vec![Arm { pat: p, guard: None, body: then_e }, Arm { pat: wild, guard: None, body: else_e }];
        self.mk(ExprKind::Match { scrut: Box::new(s), arms }, ty, span)
    }

    fn check_while_let(&mut self, pat: &ast::Pat, scrut: &ast::Expr, body: &ast::Block, span: Span) -> Expr {
        let id = self.new_loop(false);
        let s = self.check_place_or_temp(scrut);
        let st = s.ty.clone();
        self.push_scope(None);
        let p = self.check_pat(pat, &st, false);
        let b = self.check_block(body, Some(&Ty::unit()));
        self.pop_scope();
        self.loops.pop();
        let body_e = self.mk(ExprKind::Block(b), Ty::unit(), body.span);
        let brk = self.mk(ExprKind::Break { id, value: None }, Ty::Never, span);
        let brk = self.mk(ExprKind::Coerce { kind: Coercion::NeverToAny, expr: Box::new(brk) }, Ty::unit(), span);
        let wild = thir::Pat { kind: PatKind::Wild, ty: st, span };
        let m = self.mk(
            ExprKind::Match { scrut: Box::new(s), arms: vec![Arm { pat: p, guard: None, body: body_e }, Arm { pat: wild, guard: None, body: brk }] },
            Ty::unit(),
            span,
        );
        let blk = thir::Block { stmts: vec![thir::Stmt::Expr(m)], tail: None, locals: Vec::new(), span };
        self.mk(ExprKind::Loop { body: blk, id }, Ty::unit(), span)
    }

    fn check_match(&mut self, scrut: &ast::Expr, arms: &[ast::Arm], expect: Option<&Ty>, span: Span) -> Expr {
        let s = self.check_place_or_temp(scrut);
        let st = s.ty.clone();
        let mut out = Vec::new();
        let mut result_ty: Option<Ty> = expect.cloned();
        let mut all_never = true;
        for a in arms {
            self.push_scope(None);
            let p = self.check_pat(&a.pat, &st, false);
            let guard = a.guard.as_ref().map(|g| self.check_expr_coerce(g, &Ty::Bool));
            let body = self.check_expr(&a.body, result_ty.as_ref());
            self.pop_scope();
            let bt = self.shallow(&body.ty);
            let body = if matches!(bt, Ty::Never) {
                body
            } else {
                all_never = false;
                match &result_ty {
                    Some(t) => self.coerce(body, t, a.body.span),
                    None => {
                        result_ty = Some(body.ty.clone());
                        body
                    }
                }
            };
            out.push(Arm { pat: p, guard, body });
        }
        let ty = if all_never && !arms.is_empty() { Ty::Never } else { result_ty.unwrap_or(Ty::Never) };
        let ty = if arms.is_empty() { Ty::Never } else { ty };
        // Coerce diverging arms to the result type.
        let out: Vec<Arm> = out
            .into_iter()
            .map(|mut a| {
                if !matches!(ty, Ty::Never) && matches!(self.shallow(&a.body.ty), Ty::Never) {
                    let sp = a.body.span;
                    a.body = self.coerce(a.body, &ty, sp);
                }
                a
            })
            .collect();
        self.check_exhaustive(&st, &out, span);
        self.mk(ExprKind::Match { scrut: Box::new(s), arms: out }, ty, span)
    }

    fn check_for(&mut self, pat: &ast::Pat, iter: &ast::Expr, body: &ast::Block, span: Span) -> Expr {
        // Integer ranges.
        if let ast::ExprKind::Range { start: Some(s), end: Some(e), inclusive } = &iter.kind {
            let sv = self.check_expr(s, None);
            let st = sv.ty.clone();
            let ev = self.check_expr(e, Some(&st));
            let ev = self.coerce(ev, &st, e.span);
            let t = self.shallow(&st);
            if !(t.is_integral() || self.is_int_var(&t) || t.is_error()) {
                self.err(Diagnostic::error(tyc::NOT_ITERABLE, iter.span, format!("`for` ranges must be over integers, found `{}`", self.ty_str(&t))));
            }
            let id = self.new_loop(false);
            self.push_scope(None);
            let depth = self.depth();
            let (var, extra) = match &pat.kind {
                ast::PatKind::Binding { is_mut, name } => {
                    let l = self.new_local(&name.name, st.clone(), *is_mut, LocalKind::Let, name.span, depth);
                    self.bind_name(&name.name, l);
                    (l, None)
                }
                ast::PatKind::Wild => (self.new_local("_", st.clone(), false, LocalKind::Let, pat.span, depth), None),
                _ => {
                    let l = self.new_local("<i>", st.clone(), false, LocalKind::Temp, pat.span, depth);
                    let p = self.check_pat(pat, &st, true);
                    (l, Some(p))
                }
            };
            let _ = extra;
            let b = self.check_block(body, Some(&Ty::unit()));
            self.pop_scope();
            self.loops.pop();
            return self.mk(ExprKind::ForRange { var, start: Box::new(sv), end: Box::new(ev), inclusive: *inclusive, body: b, id }, Ty::unit(), span);
        }
        let it = self.check_place_or_temp(iter);
        let itt = self.shallow(&it.ty);
        let itt = itt.strip_bang().clone();
        match &itt {
            Ty::Array(elem, _) => {
                let elem = (**elem).clone();
                let id = self.new_loop(false);
                self.push_scope(None);
                let depth = self.depth();
                let index = self.new_local("<idx>", Ty::usize(), true, LocalKind::Temp, iter.span, depth);
                let p = self.check_pat(pat, &elem, true);
                let b = self.check_block(body, Some(&Ty::unit()));
                self.pop_scope();
                self.loops.pop();
                self.mk(ExprKind::ForArray { pat: p, array: Box::new(it), index, body: b, id }, Ty::unit(), span)
            }
            Ty::Error => self.error_expr(span),
            t => self.check_for_iterator(pat, it.clone(), t.clone(), body, iter.span, span),
        }
    }

    /// `for x in it` over an `Iterator::<T>` value: `loop { match it.next() { Some(x) => body, _ => break } }`.
    fn check_for_iterator(&mut self, pat: &ast::Pat, it: Expr, it_ty: Ty, body: &ast::Block, iter_span: Span, span: Span) -> Expr {
        let (Some(iter_trait), Some(option)) = (self.s.p.lang_item("iterator"), self.s.p.lang_item("option")) else {
            self.err(Diagnostic::error(tyc::NOT_ITERABLE, iter_span, "iteration is not available (missing lang items)"));
            return self.error_expr(span);
        };
        // Find the item type from an impl or bound.
        let item = self.iterator_item(&it_ty, iter_trait);
        let Some(item) = item else {
            let t = self.ty_str(&it_ty);
            self.err(
                Diagnostic::error(tyc::NOT_ITERABLE, iter_span, format!("`{t}` is not iterable"))
                    .with_note("`for` loops work over integer ranges, arrays and slices, and types implementing `Iterator`")
                    .with_help(if matches!(it_ty, Ty::Str | Ty::String) {
                        "iterate `s.chars()` or `s.bytes()`"
                    } else {
                        "implement `core::iter::Iterator` for it"
                    }),
            );
            return self.error_expr(span);
        };
        // Move the iterator into a mutable temporary.
        let depth = self.depth();
        let tmp = self.new_local("<iter>", it_ty.clone(), true, LocalKind::Let, iter_span, depth + 1);
        let id = self.new_loop(false);
        self.push_scope(None);
        let opt_ty = Ty::Adt(option, vec![item.clone()]);
        let some_idx = self.s.p.adt(option).variant_index("Some").unwrap_or(1);
        let bind = self.check_pat(pat, &item, true);
        let b = self.check_block(body, Some(&Ty::unit()));
        self.pop_scope();
        self.loops.pop();
        let tr = TraitRef { trait_id: iter_trait, args: vec![item.clone()] };
        let tmp_place = self.mk(ExprKind::Local(tmp), it_ty.clone(), iter_span);
        let next = self.mk(
            ExprKind::Call {
                callee: Callee::Trait { trait_ref: tr, self_ty: it_ty.clone(), method: "next".into(), method_args: vec![] },
                args: vec![thir::Arg::InOut(tmp_place)],
            },
            opt_ty.clone(),
            iter_span,
        );
        let next = self.expr_as_place(next);
        let some_pat = thir::Pat { kind: PatKind::Adt { def: option, variant: some_idx, fields: vec![(0, bind)] }, ty: opt_ty.clone(), span: pat.span };
        let body_e = self.mk(ExprKind::Block(b), Ty::unit(), body.span);
        let brk = self.mk(ExprKind::Break { id, value: None }, Ty::Never, span);
        let brk = self.mk(ExprKind::Coerce { kind: Coercion::NeverToAny, expr: Box::new(brk) }, Ty::unit(), span);
        let wild = thir::Pat { kind: PatKind::Wild, ty: opt_ty, span };
        let m = self.mk(
            ExprKind::Match { scrut: Box::new(next), arms: vec![Arm { pat: some_pat, guard: None, body: body_e }, Arm { pat: wild, guard: None, body: brk }] },
            Ty::unit(),
            span,
        );
        let lp = self.mk(ExprKind::Loop { body: thir::Block { stmts: vec![thir::Stmt::Expr(m)], tail: None, locals: vec![], span }, id }, Ty::unit(), span);
        let init = self.read(it, ReadMode::Move);
        let bind_tmp = thir::Pat { kind: PatKind::Binding { local: tmp, mode: ReadMode::Move, sub: None }, ty: it_ty, span: iter_span };
        let outer = thir::Block {
            stmts: vec![thir::Stmt::Let { pat: bind_tmp, init: Some(init), span: iter_span }, thir::Stmt::Expr(lp)],
            tail: None,
            locals: vec![tmp],
            span,
        };
        self.mk(ExprKind::Block(outer), Ty::unit(), span)
    }

    fn iterator_item(&mut self, t: &Ty, iter_trait: DefId) -> Option<Ty> {
        let t = self.resolve(t);
        for p in self.env.clone() {
            if p.ty == t && p.trait_ref.trait_id == iter_trait {
                return p.trait_ref.args.first().cloned();
            }
        }
        for &imp in &self.s.p.impls.clone() {
            let DefData::Impl(d) = &self.s.p.def(imp).data else { continue };
            let Some(tr) = &d.trait_ref else { continue };
            if tr.trait_id != iter_trait {
                continue;
            }
            let mut sub = vec![None; d.generics.count()];
            if crate::Program::match_ty(&d.self_ty, &t, &mut sub) {
                let args: Vec<Ty> = sub.into_iter().map(|x| x.unwrap_or(Ty::Error)).collect();
                return tr.args.first().map(|a| a.subst(&args));
            }
        }
        None
    }

    /// `expr?` on `Option` or `Result`.
    fn check_try(&mut self, x: &ast::Expr, span: Span) -> Expr {
        let v = self.check_place_or_temp(x);
        let vt = self.shallow(&v.ty);
        let (Some(option), result) = (self.s.p.lang_item("option"), self.s.p.lang_item("result")) else {
            return self.error_expr(span);
        };
        let ret = self.shallow(&self.ret_ty.clone());
        match (&vt, &ret) {
            (Ty::Adt(d, args), Ty::Adt(rd, rargs)) if *d == *rd && (*d == option || Some(*d) == result) => {
                let adt = self.s.p.adt(*d).clone();
                let is_opt = *d == option;
                let ok_name = if is_opt { "Some" } else { "Ok" };
                let ok_idx = adt.variant_index(ok_name).unwrap_or(0);
                let depth = self.depth();
                let val_ty = args[0].clone();
                let val = self.new_local("<ok>", val_ty.clone(), false, LocalKind::Let, span, depth + 1);
                let ok_pat = thir::Pat {
                    kind: PatKind::Adt {
                        def: *d,
                        variant: ok_idx,
                        fields: vec![(0, thir::Pat { kind: PatKind::Binding { local: val, mode: ReadMode::Move, sub: None }, ty: val_ty.clone(), span })],
                    },
                    ty: vt.clone(),
                    span,
                };
                let val_read = self.mk(ExprKind::Local(val), val_ty.clone(), span);
                let val_read = self.read(val_read, ReadMode::Move);
                // Failure arm: return None / Err(e).
                let fail = if is_opt {
                    let none_idx = adt.variant_index("None").unwrap_or(0);
                    let none = self.mk(ExprKind::Adt { def: *d, args: rargs.clone(), variant: none_idx, fields: vec![] }, ret.clone(), span);
                    (thir::Pat { kind: PatKind::Wild, ty: vt.clone(), span }, none)
                } else {
                    let err_idx = adt.variant_index("Err").unwrap_or(1);
                    let et = args[1].clone();
                    if !self.unify(&et, &rargs[1]) {
                        self.mismatch(&rargs[1], &et, span);
                    }
                    let e_local = self.new_local("<err>", et.clone(), false, LocalKind::Let, span, depth + 1);
                    let pat = thir::Pat {
                        kind: PatKind::Adt {
                            def: *d,
                            variant: err_idx,
                            fields: vec![(0, thir::Pat { kind: PatKind::Binding { local: e_local, mode: ReadMode::Move, sub: None }, ty: et.clone(), span })],
                        },
                        ty: vt.clone(),
                        span,
                    };
                    let er = self.mk(ExprKind::Local(e_local), et.clone(), span);
                    let er = self.read(er, ReadMode::Move);
                    let errv = self.mk(ExprKind::Adt { def: *d, args: rargs.clone(), variant: err_idx, fields: vec![(0, er)] }, ret.clone(), span);
                    (pat, errv)
                };
                let r = self.mk(ExprKind::Return(Some(Box::new(fail.1))), Ty::Never, span);
                let r = self.mk(ExprKind::Coerce { kind: Coercion::NeverToAny, expr: Box::new(r) }, val_ty.clone(), span);
                let arms = vec![Arm { pat: ok_pat, guard: None, body: val_read }, Arm { pat: fail.0, guard: None, body: r }];
                self.mk(ExprKind::Match { scrut: Box::new(v), arms }, val_ty, span)
            }
            (Ty::Error, _) | (_, Ty::Error) => self.error_expr(span),
            _ => {
                let (a, b) = (self.ty_str(&vt), self.ty_str(&ret));
                self.err(Diagnostic::error(tyc::MISMATCH, span, format!("`?` cannot be used on `{a}` in a function returning `{b}`")).with_note(
                    "`?` works on `Option` in functions returning `Option`, and on `Result` in functions returning `Result` with the same error type",
                ));
                self.error_expr(span)
            }
        }
    }
}
