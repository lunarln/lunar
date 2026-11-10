//! Lowering of THIR function bodies.
//!
//! Every local lives in an entry-block `alloca` (LLVM promotes them to
//! registers). Owned values are tracked on a *cleanup stack*: locals and
//! statement temporaries that own data stacks are dropped when their scope
//! ends, including on `return`, `break` and `continue`. Moving out of a place
//! zeroes it, and dropping a zeroed handle is a no-op.
//!
//! The value of an expression whose type lives in memory
//! ([`Cg::in_memory`]) is a pointer to memory holding it that nothing else
//! uses, which its user may read, keep or hand on. `value_into` builds such
//! a value straight where it is going (a variable, the return slot, a field
//! of a bigger value), saving a copy and a temporary.

use crate::Cg;
use inkwell::IntPredicate;
use inkwell::basic_block::BasicBlock;
use inkwell::types::BasicTypeEnum;
use inkwell::values::{AsValueRef, BasicValueEnum, FunctionValue, IntValue, PointerValue};
use ln_diag::Span;
use ln_sema::defs::ConstVal;
use ln_sema::thir::*;
use ln_sema::ty::{ArrKind, IntTy, Ty};
use std::collections::HashMap;

pub(crate) type BV<'ctx> = BasicValueEnum<'ctx>;

/// Unwrap a lowered value or propagate divergence.
macro_rules! tri {
    ($e:expr) => {
        match $e {
            Some(v) => v,
            None => return None,
        }
    };
}
pub(crate) use tri;

pub(crate) struct LoopInfo<'ctx> {
    pub brk: BasicBlock<'ctx>,
    pub cont: BasicBlock<'ctx>,
    pub slot: Option<PointerValue<'ctx>>,
    pub mark: usize,
}

pub(crate) struct FnCg<'a, 'ctx, 'p> {
    pub cg: &'a mut Cg<'ctx, 'p>,
    pub f: FunctionValue<'ctx>,
    pub body: &'p Body,
    pub args: Vec<Ty>,
    pub locals: Vec<Option<PointerValue<'ctx>>>,
    pub cleanups: Vec<(PointerValue<'ctx>, Ty)>,
    pub marks: Vec<usize>,
    pub loops: HashMap<LoopId, LoopInfo<'ctx>>,
    pub error: Option<String>,
    /// Where the function writes its result, if it lives in memory.
    pub sret: Option<PointerValue<'ctx>>,
}

impl<'a, 'ctx, 'p> FnCg<'a, 'ctx, 'p> {
    pub fn generate(cg: &'a mut Cg<'ctx, 'p>, f: FunctionValue<'ctx>, body: &'p Body, args: &[Ty]) -> Result<(), String> {
        let entry = cg.ctx.append_basic_block(f, "entry");
        cg.b.position_at_end(entry);
        let mut fc = FnCg {
            cg,
            f,
            body,
            args: args.to_vec(),
            locals: vec![None; body.locals.len()],
            cleanups: Vec::new(),
            marks: Vec::new(),
            loops: HashMap::new(),
            error: None,
            sret: None,
        };
        let ret = fc.t(&fc.cg.p.fn_data(body.def).sig.ret);
        if fc.cg.in_memory(&ret) {
            fc.sret = Some(f.get_nth_param(0).unwrap().into_pointer_value());
        }
        let first = fc.sret.is_some() as u32;
        // Parameters.
        for (i, &l) in body.params.iter().enumerate() {
            let param = f.get_nth_param(i as u32 + first).unwrap();
            let local = body.local(l);
            // In-out and borrowed parameters arrive as pointers to the
            // caller's place.
            let by_ref = matches!(local.kind, LocalKind::Param { index, .. } if fc.cg.p.param_by_ref(body.def, index));
            if by_ref {
                fc.locals[l.0 as usize] = Some(param.into_pointer_value());
            } else {
                let ty = fc.t(&local.ty);
                let slot = if fc.cg.in_memory(&ty) {
                    // A pointer to a copy that is this function's.
                    param.into_pointer_value()
                } else {
                    let lt = fc.cg.llty(&ty);
                    let slot = fc.alloca(lt, &local.name);
                    fc.cg.b.build_store(slot, param).unwrap();
                    slot
                };
                fc.locals[l.0 as usize] = Some(slot);
                // A parameter that took ownership is this function's to free
                // (moving out of it zeroes the slot, making the drop a no-op).
                if let LocalKind::Param { owned: true, .. } = local.kind {
                    fc.register_cleanup(slot, ty.strip_grow());
                }
            }
        }
        // Other locals.
        for (i, local) in body.locals.iter().enumerate() {
            if fc.locals[i].is_some() {
                continue;
            }
            let ty = fc.t(&local.ty);
            let slot = fc.alloca_for(&ty, &local.name);
            fc.locals[i] = Some(slot);
        }
        let v = fc.block_into(&body.block, fc.sret);
        if let Some(v) = v
            && !fc.terminated()
        {
            if Cg::is_void(&ret) {
                if matches!(ret, ln_sema::ty::Ty::Never) {
                    fc.cg.b.build_unreachable().unwrap();
                } else {
                    fc.cg.b.build_return(None).unwrap();
                }
            } else if fc.sret.is_some() {
                fc.cg.b.build_return(None).unwrap();
            } else {
                fc.cg.b.build_return(Some(&v)).unwrap();
            }
        } else if !fc.terminated() {
            fc.cg.b.build_unreachable().unwrap();
        }
        match fc.error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    // ===================================================================== helpers

    pub fn ice(&mut self, msg: impl Into<String>) {
        if self.error.is_none() {
            self.error = Some(format!("internal compiler error in `{}`: {}", self.cg.p.path_of(self.body.def), msg.into()));
        }
    }

    /// Substitute the instance's generic arguments, and reveal the types
    /// behind `impl Trait`.
    pub fn t(&self, t: &Ty) -> Ty {
        self.cg.p.reveal(&t.subst(&self.args))
    }

    /// An alloca in the entry block.
    pub fn alloca(&mut self, ty: BasicTypeEnum<'ctx>, name: &str) -> PointerValue<'ctx> {
        self.cg.entry_alloca(self.f, ty, name)
    }

    /// An alloca for a value of type `t`, zero-initialized if it owns data.
    pub fn alloca_for(&mut self, t: &Ty, name: &str) -> PointerValue<'ctx> {
        let lt = self.cg.llty(t);
        let slot = self.alloca(lt, name);
        if self.cg.p.has_owner(t) {
            // Zero right after the alloca so drops of never-written slots are no-ops.
            let entry = self.f.get_first_basic_block().unwrap();
            let b = self.cg.ctx.create_builder();
            match slot.as_instruction().and_then(|i| i.get_next_instruction()) {
                Some(next) => b.position_before(&next),
                None => b.position_at_end(entry),
            }
            if self.cg.in_memory(t) {
                let size = self.cg.usize_ty().const_int(self.cg.size_of(t), false);
                let align = self.cg.align_of(t);
                b.build_memset(slot, align, self.cg.ctx.i8_type().const_zero(), size).unwrap();
            } else {
                b.build_store(slot, lt.const_zero()).unwrap();
            }
        }
        slot
    }

    pub fn unit(&self) -> BV<'ctx> {
        self.cg.unit_ty().const_named_struct(&[]).into()
    }

    pub fn terminated(&self) -> bool {
        self.cg.b.get_insert_block().and_then(|b| b.get_terminator()).is_some()
    }

    pub fn block_named(&self, name: &str) -> BasicBlock<'ctx> {
        self.cg.ctx.append_basic_block(self.f, name)
    }

    pub fn usize(&self, v: u64) -> IntValue<'ctx> {
        self.cg.usize_ty().const_int(v, false)
    }

    /// The value of type `t` at `ptr`: a copy, for one that lives in memory.
    pub fn load(&mut self, ptr: PointerValue<'ctx>, t: &Ty) -> BV<'ctx> {
        let lt = self.cg.llty(t);
        if self.cg.in_memory(t) {
            let tmp = self.alloca(lt, "copy");
            self.cg.copy_mem(tmp, ptr, t);
            return tmp.into();
        }
        self.cg.b.build_load(lt, ptr, "").unwrap()
    }

    /// Store value `v` of type `t` at `ptr` (copying it from where it is,
    /// if it lives in memory).
    pub fn store(&mut self, ptr: PointerValue<'ctx>, v: BV<'ctx>, t: &Ty) {
        if v.is_pointer_value() && self.cg.in_memory(t) {
            self.cg.copy_mem(ptr, v.into_pointer_value(), t);
        } else {
            self.cg.b.build_store(ptr, v).unwrap();
        }
    }

    /// Zero the value of type `t` at `ptr`.
    pub fn store_zero(&mut self, ptr: PointerValue<'ctx>, t: &Ty) {
        if self.cg.in_memory(t) {
            let size = self.cg.usize_ty().const_int(self.cg.size_of(t), false);
            let align = self.cg.align_of(t);
            self.cg.b.build_memset(ptr, align, self.cg.ctx.i8_type().const_zero(), size).unwrap();
        } else {
            let lt = self.cg.llty(t);
            self.cg.b.build_store(ptr, lt.const_zero()).unwrap();
        }
    }

    /// The address of value `v` of type `t`: where it is, if it lives in
    /// memory, or else a new slot holding it.
    pub fn spill(&mut self, v: BV<'ctx>, t: &Ty, name: &str) -> PointerValue<'ctx> {
        if v.is_pointer_value() && self.cg.in_memory(t) {
            return v.into_pointer_value();
        }
        let tmp = self.alloca_for(t, name);
        self.cg.b.build_store(tmp, v).unwrap();
        tmp
    }

    pub fn struct_field(&mut self, sty: BasicTypeEnum<'ctx>, ptr: PointerValue<'ctx>, idx: u32) -> PointerValue<'ctx> {
        self.cg.b.build_struct_gep(sty, ptr, idx, "").unwrap()
    }

    // ===================================================================== cleanups

    pub fn push_mark(&mut self) {
        self.marks.push(self.cleanups.len());
    }

    /// Pop the innermost mark, emitting drops if control reaches here.
    pub fn pop_mark(&mut self) {
        let m = self.marks.pop().unwrap();
        if !self.terminated() {
            self.emit_cleanups(m);
        }
        self.cleanups.truncate(m);
    }

    /// Emit drops for cleanup entries above `mark` (without popping them).
    pub fn emit_cleanups(&mut self, mark: usize) {
        let entries: Vec<(PointerValue<'ctx>, Ty)> = self.cleanups[mark..].iter().rev().cloned().collect();
        for (p, t) in entries {
            self.drop_in_place(p, &t);
            self.store_zero(p, &t);
        }
    }

    pub fn register_cleanup(&mut self, p: PointerValue<'ctx>, t: &Ty) {
        if self.cg.p.has_owner(t) {
            self.cleanups.push((p, t.clone()));
        }
    }

    // ===================================================================== blocks & statements

    pub fn block(&mut self, b: &Block) -> Option<BV<'ctx>> {
        self.block_into(b, None)
    }

    /// A block whose value, if `dest` is given, is built at `dest` (see
    /// `value_into`).
    pub fn block_into(&mut self, b: &Block, dest: Option<PointerValue<'ctx>>) -> Option<BV<'ctx>> {
        self.push_mark();
        for s in &b.stmts {
            if self.stmt(s).is_none() {
                self.marks.pop();
                return None;
            }
        }
        let v = match (&b.tail, dest) {
            (Some(t), Some(d)) => self.value_into(t, d).map(|()| d.into()),
            (Some(t), None) => self.value(t),
            (None, _) => Some(self.unit()),
        };
        match v {
            Some(v) => {
                self.pop_mark();
                Some(v)
            }
            None => {
                let m = self.marks.pop().unwrap();
                self.cleanups.truncate(m);
                None
            }
        }
    }

    fn stmt(&mut self, s: &Stmt) -> Option<()> {
        match s {
            Stmt::Let { pat, init, .. } => {
                let Some(init) = init else { return Some(()) };
                let ty = self.t(&pat.ty);
                if self.cg.in_memory(&ty) {
                    return self.let_in_memory(pat, init, &ty);
                }
                self.push_mark();
                let v = match self.value(init) {
                    Some(v) => v,
                    None => {
                        let m = self.marks.pop().unwrap();
                        self.cleanups.truncate(m);
                        return None;
                    }
                };
                self.pop_mark();
                match &pat.kind {
                    PatKind::Binding { local, sub: None, .. } => {
                        let slot = self.locals[local.0 as usize].unwrap();
                        self.cg.b.build_store(slot, v).unwrap();
                        self.register_cleanup(slot, &ty);
                    }
                    PatKind::Wild => {
                        let tmp = self.alloca_for(&ty, "let_tmp");
                        self.cg.b.build_store(tmp, v).unwrap();
                        self.drop_in_place(tmp, &ty);
                    }
                    _ => {
                        let tmp = self.alloca_for(&ty, "let_tmp");
                        self.cg.b.build_store(tmp, v).unwrap();
                        self.bind_pat(pat, tmp, &ty, true);
                        self.register_cleanup(tmp, &ty);
                    }
                }
                Some(())
            }
            Stmt::Expr(e) => {
                self.push_mark();
                match self.value(e) {
                    Some(v) => {
                        let ty = self.t(&e.ty);
                        if self.cg.p.has_owner(&ty) {
                            // A discarded owned value is dropped right away.
                            let tmp = self.spill(v, &ty, "discard");
                            self.drop_in_place(tmp, &ty);
                            self.store_zero(tmp, &ty);
                        }
                        self.pop_mark();
                        Some(())
                    }
                    None => {
                        let m = self.marks.pop().unwrap();
                        self.cleanups.truncate(m);
                        None
                    }
                }
            }
        }
    }

    /// `let pat = init;` for a value that lives in memory: built in the
    /// variable, or in a temporary the pattern binds from.
    fn let_in_memory(&mut self, pat: &Pat, init: &Expr, ty: &Ty) -> Option<()> {
        let dest = match &pat.kind {
            PatKind::Binding { local, sub: None, .. } => self.locals[local.0 as usize].unwrap(),
            _ => self.alloca_for(ty, "let_tmp"),
        };
        self.push_mark();
        if self.value_into(init, dest).is_none() {
            let m = self.marks.pop().unwrap();
            self.cleanups.truncate(m);
            return None;
        }
        self.pop_mark();
        match &pat.kind {
            PatKind::Binding { sub: None, .. } => self.register_cleanup(dest, ty),
            PatKind::Wild => self.drop_in_place(dest, ty),
            _ => {
                self.bind_pat(pat, dest, ty, true);
                self.register_cleanup(dest, ty);
            }
        }
        Some(())
    }

    /// Evaluate `e` inside its own temporary scope (conditions, guards).
    fn scoped_value(&mut self, e: &Expr) -> Option<BV<'ctx>> {
        self.push_mark();
        match self.value(e) {
            Some(v) => {
                self.pop_mark();
                Some(v)
            }
            None => {
                let m = self.marks.pop().unwrap();
                self.cleanups.truncate(m);
                None
            }
        }
    }

    // ===================================================================== places

    pub fn place(&mut self, e: &Expr) -> Option<PointerValue<'ctx>> {
        match &e.kind {
            ExprKind::Local(l) => Some(self.locals[l.0 as usize].unwrap()),
            ExprKind::Static(d) => match self.cg.static_ptr(*d) {
                Ok(p) => Some(p),
                Err(err) => {
                    self.ice(err);
                    None
                }
            },
            ExprKind::Field { base, variant, field } => {
                let bp = tri!(self.place(base));
                let bt = self.t(&base.ty).strip_grow().clone();
                Some(self.field_ptr(bp, &bt, *variant, *field))
            }
            ExprKind::Index { base, index } => {
                let bp = tri!(self.place(base));
                let i = tri!(self.value(index)).into_int_value();
                let bt = self.t(&base.ty).strip_grow().clone();
                let (data, len, elem) = self.array_parts(bp, &bt);
                self.check_bounds(i, len, e.span);
                let et = self.cg.llty(&elem);
                Some(unsafe { self.cg.b.build_gep(et, data, &[i], "elem").unwrap() })
            }
            ExprKind::Deref(p) => Some(tri!(self.value(p)).into_pointer_value()),
            ExprKind::Temp(v) => {
                let val = tri!(self.value(v));
                let ty = self.t(&v.ty);
                let tmp = self.spill(val, &ty, "tmp");
                self.register_cleanup(tmp, &ty);
                Some(tmp)
            }
            _ => {
                // A value used where a place is needed.
                let val = tri!(self.value(e));
                let ty = self.t(&e.ty);
                let tmp = self.spill(val, &ty, "tmp");
                self.register_cleanup(tmp, &ty);
                Some(tmp)
            }
        }
    }

    pub fn field_ptr(&mut self, bp: PointerValue<'ctx>, bt: &Ty, variant: usize, field: usize) -> PointerValue<'ctx> {
        match bt {
            Ty::Adt(d, _) if self.cg.p.is_enum(*d) => {
                let et = self.cg.llty(bt);
                let payload = self.struct_field(et, bp, 1);
                let vs = self.cg.variant_struct(bt, variant);
                self.struct_field(vs.into(), payload, field as u32)
            }
            _ => {
                let lt = self.cg.llty(bt);
                self.struct_field(lt, bp, field as u32)
            }
        }
    }

    /// Data pointer, length and element type of an array-like place.
    pub fn array_parts(&mut self, p: PointerValue<'ctx>, t: &Ty) -> (PointerValue<'ctx>, IntValue<'ctx>, Ty) {
        match t {
            Ty::Array(e, ArrKind::Fixed(n)) => (p, self.usize(*n), (**e).clone()),
            Ty::Array(e, k) => {
                let st: BasicTypeEnum = if k.is_slice() { self.cg.slice_ty().into() } else { self.cg.handle_ty().into() };
                let dp = self.struct_field(st, p, 0);
                let lp = self.struct_field(st, p, 1);
                let data = self.cg.b.build_load(self.cg.ptr_ty(), dp, "data").unwrap().into_pointer_value();
                let len = self.cg.b.build_load(self.cg.usize_ty(), lp, "len").unwrap().into_int_value();
                (data, len, (**e).clone())
            }
            Ty::Str | Ty::String => {
                let st: BasicTypeEnum = if matches!(t, Ty::Str) { self.cg.slice_ty().into() } else { self.cg.handle_ty().into() };
                let dp = self.struct_field(st, p, 0);
                let lp = self.struct_field(st, p, 1);
                let data = self.cg.b.build_load(self.cg.ptr_ty(), dp, "data").unwrap().into_pointer_value();
                let len = self.cg.b.build_load(self.cg.usize_ty(), lp, "len").unwrap().into_int_value();
                (data, len, Ty::Int(IntTy::U8))
            }
            other => {
                let s = self.cg.p.ty_str(other);
                self.ice(format!("not an array type: {s}"));
                (self.cg.ptr_ty().const_null(), self.usize(0), Ty::unit())
            }
        }
    }

    pub fn check_bounds(&mut self, i: IntValue<'ctx>, len: IntValue<'ctx>, span: Span) {
        let ok = self.cg.b.build_int_compare(IntPredicate::ULT, i, len, "inb").unwrap();
        let good = self.block_named("inbounds");
        let bad = self.block_named("oob");
        self.cg.b.build_conditional_branch(ok, good, bad).unwrap();
        self.cg.b.position_at_end(bad);
        self.panic_bounds(i, len, span);
        self.cg.b.position_at_end(good);
    }

    // ===================================================================== values

    pub fn value(&mut self, e: &Expr) -> Option<BV<'ctx>> {
        use ExprKind as K;
        let ty = self.t(&e.ty);
        let built = matches!(
            e.kind,
            K::Read { .. }
                | K::Lit(_)
                | K::Call { .. }
                | K::Tuple(_)
                | K::Adt { .. }
                | K::Array(_)
                | K::ArrayRepeat { .. }
                | K::If { .. }
                | K::Match { .. }
                | K::Loop { .. }
                | K::Closure { .. }
        );
        if built && self.cg.in_memory(&ty) {
            let lt = self.cg.llty(&ty);
            let tmp = self.alloca(lt, "val");
            tri!(self.value_into(e, tmp));
            return Some(tmp.into());
        }
        match &e.kind {
            K::Local(_) | K::Static(_) | K::Field { .. } | K::Index { .. } | K::Deref(_) => {
                let p = tri!(self.place(e));
                let t = ty.strip_grow().clone();
                Some(self.load(p, &t))
            }
            K::Temp(v) => self.value(v),
            K::Read { place, mode } => {
                let p = tri!(self.place(place));
                let pt = self.t(&place.ty).strip_grow().clone();
                let v = self.load(p, &pt);
                if *mode == ReadMode::Move && self.cg.p.has_owner(&pt) {
                    self.store_zero(p, &pt);
                }
                Some(v)
            }
            K::Lit(v) => match self.cg.const_val(v, &ty) {
                Ok(c) => Some(c),
                Err(err) => {
                    self.ice(err);
                    Some(self.unit())
                }
            },
            K::FnRef { def, args } => {
                let args: Vec<Ty> = args.iter().map(|a| self.t(a)).collect();
                let f = self.cg.instance(*def, &args);
                Some(f.as_global_value().as_pointer_value().into())
            }
            K::Call { callee, args } => self.call(callee, args, &ty, e.span, None),
            K::Unary(op, x) => {
                let v = tri!(self.value(x));
                Some(self.unary(*op, v, &ty, e.span))
            }
            K::Binary(op, a, b) => {
                let va = tri!(self.value(a));
                let vb = tri!(self.value(b));
                let at = self.t(&a.ty);
                Some(self.binop(*op, va, vb, &at, e.span))
            }
            K::Logical { and, lhs, rhs } => {
                let l = tri!(self.value(lhs)).into_int_value();
                let lhs_bb = self.cg.b.get_insert_block().unwrap();
                let rhs_bb = self.block_named("rhs");
                let merge = self.block_named("logic");
                if *and {
                    self.cg.b.build_conditional_branch(l, rhs_bb, merge).unwrap();
                } else {
                    self.cg.b.build_conditional_branch(l, merge, rhs_bb).unwrap();
                }
                self.cg.b.position_at_end(rhs_bb);
                let r = self.scoped_value(rhs);
                let r_end = self.cg.b.get_insert_block().unwrap();
                if r.is_some() {
                    self.cg.b.build_unconditional_branch(merge).unwrap();
                }
                self.cg.b.position_at_end(merge);
                let phi = self.cg.b.build_phi(self.cg.ctx.bool_type(), "logic").unwrap();
                let short = self.cg.ctx.bool_type().const_int(!*and as u64, false);
                phi.add_incoming(&[(&short, lhs_bb)]);
                if let Some(r) = r {
                    phi.add_incoming(&[(&r, r_end)]);
                }
                Some(phi.as_basic_value())
            }
            K::Cast(x) => {
                let v = tri!(self.value(x));
                let from = self.t(&x.ty);
                Some(self.cast(v, &from, &ty))
            }
            K::Assign { place, value } => {
                let v = tri!(self.value(value));
                let p = tri!(self.place(place));
                let pt = self.t(&place.ty).strip_grow().clone();
                self.drop_in_place(p, &pt);
                self.store(p, v, &pt);
                Some(self.unit())
            }
            K::AssignOp { op, place, value } => {
                let v = tri!(self.value(value));
                let p = tri!(self.place(place));
                let pt = self.t(&place.ty).strip_grow().clone();
                let cur = self.load(p, &pt);
                let new = self.binop(*op, cur, v, &pt, e.span);
                self.cg.b.build_store(p, new).unwrap();
                Some(self.unit())
            }
            K::Tuple(es) => {
                let lt = self.cg.llty(&ty).into_struct_type();
                let mut agg = lt.get_undef();
                for (i, x) in es.iter().enumerate() {
                    let v = tri!(self.value(x));
                    agg = self.cg.b.build_insert_value(agg, v, i as u32, "").unwrap().into_struct_value();
                }
                Some(agg.into())
            }
            K::Adt { variant, fields, .. } => {
                let mut vals = Vec::new();
                for (i, x) in fields {
                    vals.push((*i, tri!(self.value(x))));
                }
                Some(self.build_adt(&ty, *variant, vals))
            }
            K::Array(es) => {
                let Ty::Array(elem, kind) = &ty else {
                    self.ice("array literal of non-array type");
                    return None;
                };
                let mut vals = Vec::new();
                for x in es {
                    vals.push(tri!(self.value(x)));
                }
                match kind {
                    ArrKind::Fixed(_) | ArrKind::FixedVar(_) => {
                        let lt = self.cg.llty(&ty).into_array_type();
                        let mut agg = lt.get_undef();
                        for (i, v) in vals.into_iter().enumerate() {
                            agg = self.cg.b.build_insert_value(agg, v, i as u32, "").unwrap().into_array_value();
                        }
                        Some(agg.into())
                    }
                    ArrKind::Slice | ArrKind::SliceMut => Some(self.cg.slice_ty().const_zero().into()),
                    _ => Some(self.owner_from_values(elem, *kind, vals)),
                }
            }
            K::ArrayRepeat { value, count } => {
                let v = tri!(self.value(value));
                let n = tri!(self.value(count)).into_int_value();
                let Ty::Array(elem, kind) = &ty else { return None };
                Some(self.array_repeat(elem, *kind, &ty, v, n))
            }
            K::Slice { base, start, end, inclusive, .. } => {
                let bp = tri!(self.place(base));
                let bt = self.t(&base.ty).strip_grow().clone();
                let s = match start {
                    Some(x) => Some(tri!(self.value(x)).into_int_value()),
                    None => None,
                };
                let en = match end {
                    Some(x) => Some(tri!(self.value(x)).into_int_value()),
                    None => None,
                };
                Some(self.slice(bp, &bt, s, en, *inclusive, e.span))
            }
            K::Block(b) => self.block(b),
            K::If { cond, then, else_ } => self.if_expr(cond, then, else_.as_deref(), &ty, None),
            K::Match { scrut, arms } => self.match_expr(scrut, arms, &ty, None),
            K::Loop { body, id } => self.loop_expr(body, *id, &ty, None),
            K::While { cond, body, id } => {
                let head = self.block_named("while");
                let body_bb = self.block_named("while_body");
                let exit = self.block_named("while_end");
                let mark = self.cleanups.len();
                self.loops.insert(*id, LoopInfo { brk: exit, cont: head, slot: None, mark });
                self.cg.b.build_unconditional_branch(head).unwrap();
                self.cg.b.position_at_end(head);
                let c = self.scoped_value(cond);
                if let Some(c) = c {
                    self.cg.b.build_conditional_branch(c.into_int_value(), body_bb, exit).unwrap();
                }
                self.cg.b.position_at_end(body_bb);
                if self.block(body).is_some() && !self.terminated() {
                    self.cg.b.build_unconditional_branch(head).unwrap();
                }
                self.cg.b.position_at_end(exit);
                Some(self.unit())
            }
            K::ForRange { var, start, end, inclusive, body, id } => self.for_range(*var, start, end, *inclusive, body, *id),
            K::ForArray { pat, array, index, body, id } => self.for_array(pat, array, *index, body, *id),
            K::Break { id, value } => {
                let (brk, slot, mark) = {
                    let l = &self.loops[id];
                    (l.brk, l.slot, l.mark)
                };
                match (value, slot) {
                    (Some(x), Some(s)) => tri!(self.value_into(x, s)),
                    (Some(x), None) => {
                        tri!(self.value(x));
                    }
                    (None, _) => {}
                }
                self.emit_cleanups(mark);
                self.cg.b.build_unconditional_branch(brk).unwrap();
                None
            }
            K::Continue { id } => {
                let (cont, mark) = {
                    let l = &self.loops[id];
                    (l.cont, l.mark)
                };
                self.emit_cleanups(mark);
                self.cg.b.build_unconditional_branch(cont).unwrap();
                None
            }
            K::Return(v) => {
                let val = match (v, self.sret) {
                    (Some(x), Some(s)) => {
                        tri!(self.value_into(x, s));
                        self.unit()
                    }
                    (Some(x), None) => tri!(self.value(x)),
                    (None, _) => self.unit(),
                };
                self.emit_cleanups(0);
                let ret = self.t(&self.cg.p.fn_data(self.body.def).sig.ret);
                if Cg::is_void(&ret) || self.sret.is_some() {
                    self.cg.b.build_return(None).unwrap();
                } else {
                    self.cg.b.build_return(Some(&val)).unwrap();
                }
                None
            }
            K::Coerce { kind, expr } => match kind {
                Coercion::NeverToAny => {
                    let v = self.value(expr);
                    if v.is_some() && !self.terminated() {
                        self.cg.b.build_unreachable().unwrap();
                    }
                    None
                }
                Coercion::Weaken | Coercion::FnToPtr => self.value(expr),
                Coercion::ToSlice { .. } | Coercion::StringToStr => {
                    let p = tri!(self.place(expr));
                    let at = self.t(&expr.ty).strip_grow().clone();
                    let (data, len, _) = self.array_parts(p, &at);
                    Some(self.make_slice(data, len))
                }
            },
            K::FmtArgs(pieces) => self.fmt_args(pieces),
            // The captures: the address of each borrowed variable, the value
            // of each taken one.
            K::Closure { captures, .. } => {
                let lt = self.cg.llty(&ty).into_struct_type();
                let mut agg = lt.get_undef();
                for (i, a) in captures.iter().enumerate() {
                    let v: BV<'ctx> = match a {
                        Arg::InOut(p) => tri!(self.place(p)).into(),
                        Arg::Value(p) if p.is_place() => tri!(self.place(p)).into(),
                        Arg::Value(v) => tri!(self.value(v)),
                    };
                    agg = self.cg.b.build_insert_value(agg, v, i as u32, "").unwrap().into_struct_value();
                }
                Some(agg.into())
            }
            K::Error => {
                self.ice("error expression reached code generation");
                None
            }
        }
    }

    /// Evaluate `e` into `dest`. A value that lives in memory is built
    /// there, so `dest` must be memory nothing reads while `e` is evaluated:
    /// a new temporary, a variable being declared, a result slot.
    pub fn value_into(&mut self, e: &Expr, dest: PointerValue<'ctx>) -> Option<()> {
        use ExprKind as K;
        let ty = self.t(&e.ty);
        if !self.cg.in_memory(&ty) {
            let v = tri!(self.value(e));
            self.store(dest, v, &ty);
            return Some(());
        }
        match &e.kind {
            K::Local(_) | K::Static(_) | K::Field { .. } | K::Index { .. } | K::Deref(_) => {
                let p = tri!(self.place(e));
                self.cg.copy_mem(dest, p, &ty);
            }
            K::Temp(v) => return self.value_into(v, dest),
            K::Read { place, mode } => {
                let p = tri!(self.place(place));
                let pt = self.t(&place.ty).strip_grow().clone();
                self.cg.copy_mem(dest, p, &pt);
                if *mode == ReadMode::Move && self.cg.p.has_owner(&pt) {
                    self.store_zero(p, &pt);
                }
            }
            K::Lit(v) => match self.cg.const_global(v, &ty) {
                Ok(g) => self.cg.copy_mem(dest, g, &ty),
                Err(err) => self.ice(err),
            },
            K::Call { callee, args } => {
                let v = tri!(self.call(callee, args, &ty, e.span, Some(dest)));
                if v != BV::from(dest) {
                    self.store(dest, v, &ty);
                }
            }
            K::Tuple(es) => {
                let lt = self.cg.llty(&ty);
                for (i, x) in es.iter().enumerate() {
                    let fp = self.struct_field(lt, dest, i as u32);
                    tri!(self.value_into(x, fp));
                }
            }
            K::Adt { variant, fields, .. } => {
                if let Ty::Adt(d, _) = &ty
                    && self.cg.p.is_enum(*d)
                {
                    // Zeroed first, as `build_adt` does.
                    self.store_zero(dest, &ty);
                    let lt = self.cg.llty(&ty);
                    let tag = self.struct_field(lt, dest, 0);
                    self.cg.b.build_store(tag, self.cg.ctx.i32_type().const_int(*variant as u64, false)).unwrap();
                }
                for (i, x) in fields {
                    let fp = self.field_ptr(dest, &ty, *variant, *i);
                    tri!(self.value_into(x, fp));
                }
            }
            K::Array(es) => {
                let Ty::Array(elem, _) = &ty else {
                    self.ice("array literal of non-array type");
                    return None;
                };
                let et = self.cg.llty(elem);
                for (i, x) in es.iter().enumerate() {
                    let p = unsafe { self.cg.b.build_gep(et, dest, &[self.usize(i as u64)], "").unwrap() };
                    tri!(self.value_into(x, p));
                }
            }
            K::ArrayRepeat { value, count } => {
                let v = tri!(self.value(value));
                tri!(self.value(count));
                let Ty::Array(elem, ArrKind::Fixed(n)) = &ty else {
                    self.ice("repeat of a non-array type");
                    return None;
                };
                self.fill(dest, elem, *n, v);
            }
            K::Block(b) => {
                tri!(self.block_into(b, Some(dest)));
            }
            K::If { cond, then, else_ } => {
                tri!(self.if_expr(cond, then, else_.as_deref(), &ty, Some(dest)));
            }
            K::Match { scrut, arms } => {
                tri!(self.match_expr(scrut, arms, &ty, Some(dest)));
            }
            K::Loop { body, id } => {
                tri!(self.loop_expr(body, *id, &ty, Some(dest)));
            }
            // The captures: the address of each borrowed variable, the value
            // of each taken one.
            K::Closure { captures, .. } => {
                let lt = self.cg.llty(&ty);
                for (i, a) in captures.iter().enumerate() {
                    let fp = self.struct_field(lt, dest, i as u32);
                    match a {
                        Arg::InOut(p) => {
                            let pp = tri!(self.place(p));
                            self.cg.b.build_store(fp, pp).unwrap();
                        }
                        Arg::Value(p) if p.is_place() => {
                            let pp = tri!(self.place(p));
                            self.cg.b.build_store(fp, pp).unwrap();
                        }
                        Arg::Value(v) => tri!(self.value_into(v, fp)),
                    }
                }
            }
            _ => {
                let v = tri!(self.value(e));
                self.store(dest, v, &ty);
            }
        }
        Some(())
    }

    /// Fill the `n` elements of type `elem` at `dest` with `v`: a `memset`
    /// for bytes and zeros, else a loop.
    fn fill(&mut self, dest: PointerValue<'ctx>, elem: &Ty, n: u64, v: BV<'ctx>) {
        let et = self.cg.llty(elem);
        let byte = matches!(et, BasicTypeEnum::IntType(t) if t.get_bit_width() == 8);
        let zero = unsafe { llvm_sys_201::core::LLVMIsNull(v.as_value_ref()) } != 0;
        if byte || zero {
            let b = if byte { v.into_int_value() } else { self.cg.ctx.i8_type().const_zero() };
            let bytes = n * self.cg.size_of(elem);
            let align = self.cg.align_of(elem);
            self.cg.b.build_memset(dest, align, b, self.usize(bytes)).unwrap();
            return;
        }
        // for i in 0..n: dest[i] = v
        let idx = self.alloca(self.cg.usize_ty().into(), "i");
        self.cg.b.build_store(idx, self.usize(0)).unwrap();
        let head = self.block_named("fill_head");
        let body = self.block_named("fill_body");
        let exit = self.block_named("fill_end");
        self.cg.b.build_unconditional_branch(head).unwrap();
        self.cg.b.position_at_end(head);
        let i = self.cg.b.build_load(self.cg.usize_ty(), idx, "").unwrap().into_int_value();
        let c = self.cg.b.build_int_compare(IntPredicate::ULT, i, self.usize(n), "").unwrap();
        self.cg.b.build_conditional_branch(c, body, exit).unwrap();
        self.cg.b.position_at_end(body);
        let p = unsafe { self.cg.b.build_gep(et, dest, &[i], "").unwrap() };
        self.store(p, v, elem);
        let next = self.cg.b.build_int_add(i, self.usize(1), "").unwrap();
        self.cg.b.build_store(idx, next).unwrap();
        self.cg.b.build_unconditional_branch(head).unwrap();
        self.cg.b.position_at_end(exit);
    }

    pub fn make_slice(&mut self, data: PointerValue<'ctx>, len: IntValue<'ctx>) -> BV<'ctx> {
        let st = self.cg.slice_ty();
        let a = self.cg.b.build_insert_value(st.get_undef(), data, 0, "").unwrap();
        let a = self.cg.b.build_insert_value(a, len, 1, "").unwrap();
        a.into_struct_value().into()
    }

    pub fn build_adt(&mut self, ty: &Ty, variant: usize, vals: Vec<(usize, BV<'ctx>)>) -> BV<'ctx> {
        let Ty::Adt(d, _) = ty else { unreachable!() };
        if self.cg.p.is_enum(*d) {
            let lt = self.cg.llty(ty);
            let tmp = self.alloca(lt, "enum");
            self.cg.b.build_store(tmp, lt.const_zero()).unwrap();
            let tag = self.struct_field(lt, tmp, 0);
            self.cg.b.build_store(tag, self.cg.ctx.i32_type().const_int(variant as u64, false)).unwrap();
            for (i, v) in vals {
                let fp = self.field_ptr(tmp, ty, variant, i);
                self.cg.b.build_store(fp, v).unwrap();
            }
            return self.cg.b.build_load(lt, tmp, "").unwrap();
        }
        let st = self.cg.llty(ty).into_struct_type();
        let mut agg = st.get_undef();
        for (i, v) in vals {
            agg = self.cg.b.build_insert_value(agg, v, i as u32, "").unwrap().into_struct_value();
        }
        agg.into()
    }

    fn slice(&mut self, bp: PointerValue<'ctx>, bt: &Ty, start: Option<IntValue<'ctx>>, end: Option<IntValue<'ctx>>, inclusive: bool, span: Span) -> BV<'ctx> {
        let (data, len, elem) = self.array_parts(bp, bt);
        let s = start.unwrap_or(self.usize(0));
        let mut e = end.unwrap_or(len);
        if inclusive {
            e = self.cg.b.build_int_add(e, self.usize(1), "").unwrap();
        }
        let ok1 = self.cg.b.build_int_compare(IntPredicate::ULE, s, e, "").unwrap();
        let ok2 = self.cg.b.build_int_compare(IntPredicate::ULE, e, len, "").unwrap();
        let ok = self.cg.b.build_and(ok1, ok2, "").unwrap();
        let good = self.block_named("slice_ok");
        let bad = self.block_named("slice_bad");
        self.cg.b.build_conditional_branch(ok, good, bad).unwrap();
        self.cg.b.position_at_end(bad);
        self.panic_msg("slice range out of bounds", span);
        self.cg.b.position_at_end(good);
        if matches!(bt, Ty::Str | Ty::String) {
            self.check_char_boundary(data, len, s, span);
            self.check_char_boundary(data, len, e, span);
        }
        let et = self.cg.llty(&elem);
        let p = unsafe { self.cg.b.build_gep(et, data, &[s], "slice").unwrap() };
        let n = self.cg.b.build_int_sub(e, s, "").unwrap();
        self.make_slice(p, n)
    }

    /// Panic if byte index `i` (< len) falls inside a UTF-8 sequence.
    fn check_char_boundary(&mut self, data: PointerValue<'ctx>, len: IntValue<'ctx>, i: IntValue<'ctx>, span: Span) {
        let in_range = self.cg.b.build_int_compare(IntPredicate::ULT, i, len, "").unwrap();
        let check = self.block_named("utf8_check");
        let ok = self.block_named("utf8_ok");
        self.cg.b.build_conditional_branch(in_range, check, ok).unwrap();
        self.cg.b.position_at_end(check);
        let bp = unsafe { self.cg.b.build_gep(self.cg.ctx.i8_type(), data, &[i], "").unwrap() };
        let byte = self.cg.b.build_load(self.cg.ctx.i8_type(), bp, "").unwrap().into_int_value();
        let masked = self.cg.b.build_and(byte, self.cg.ctx.i8_type().const_int(0xC0, false), "").unwrap();
        let cont = self.cg.b.build_int_compare(IntPredicate::EQ, masked, self.cg.ctx.i8_type().const_int(0x80, false), "").unwrap();
        let bad = self.block_named("utf8_bad");
        self.cg.b.build_conditional_branch(cont, bad, ok).unwrap();
        self.cg.b.position_at_end(bad);
        self.panic_msg("string slice is not on a character boundary", span);
        self.cg.b.position_at_end(ok);
    }

    fn array_repeat(&mut self, elem: &Ty, kind: ArrKind, ty: &Ty, v: BV<'ctx>, n: IntValue<'ctx>) -> BV<'ctx> {
        let et = self.cg.llty(elem);
        let (slot, data, count) = if let ArrKind::Fixed(k) = kind {
            let lt = self.cg.llty(ty);
            let slot = self.alloca(lt, "rep");
            (slot, slot, self.usize(k))
        } else {
            let slot = self.alloca_for(ty, "rep");
            let ht = self.cg.handle_ty();
            self.cg.b.build_store(slot, ht.const_zero()).unwrap();
            self.size_frozen(slot, elem, kind, n);
            self.ensure_cap(slot, n, elem);
            let dp = self.struct_field(ht.into(), slot, 0);
            let data = self.cg.b.build_load(self.cg.ptr_ty(), dp, "").unwrap().into_pointer_value();
            let lp = self.struct_field(ht.into(), slot, 1);
            self.cg.b.build_store(lp, n).unwrap();
            (slot, data, n)
        };
        // for i in 0..count: data[i] = v
        let idx = self.alloca(self.cg.usize_ty().into(), "i");
        self.cg.b.build_store(idx, self.usize(0)).unwrap();
        let head = self.block_named("rep_head");
        let body = self.block_named("rep_body");
        let exit = self.block_named("rep_end");
        self.cg.b.build_unconditional_branch(head).unwrap();
        self.cg.b.position_at_end(head);
        let i = self.cg.b.build_load(self.cg.usize_ty(), idx, "").unwrap().into_int_value();
        let c = self.cg.b.build_int_compare(IntPredicate::ULT, i, count, "").unwrap();
        self.cg.b.build_conditional_branch(c, body, exit).unwrap();
        self.cg.b.position_at_end(body);
        let p = unsafe { self.cg.b.build_gep(et, data, &[i], "").unwrap() };
        self.store(p, v, elem);
        let next = self.cg.b.build_int_add(i, self.usize(1), "").unwrap();
        self.cg.b.build_store(idx, next).unwrap();
        self.cg.b.build_unconditional_branch(head).unwrap();
        self.cg.b.position_at_end(exit);
        self.load(slot, ty)
    }

    /// A new owning array holding `vals`.
    /// `[T]` never grows after it is built, so it gets a slot sized to fit
    /// instead of the default reservation.
    fn size_frozen(&mut self, slot: PointerValue<'ctx>, elem: &Ty, kind: ArrKind, n: IntValue<'ctx>) {
        let bytes = self.cg.size_of(elem);
        if kind == ArrKind::Dyn && bytes > 0 {
            let size = self.usize(bytes);
            self.emit_ds_cap(slot, size, n);
        }
    }

    fn owner_from_values(&mut self, elem: &Ty, kind: ArrKind, vals: Vec<BV<'ctx>>) -> BV<'ctx> {
        let ht = self.cg.handle_ty();
        let slot = self.alloca(ht.into(), "arr");
        self.cg.b.build_store(slot, ht.const_zero()).unwrap();
        if !vals.is_empty() {
            let n = self.usize(vals.len() as u64);
            self.size_frozen(slot, elem, kind, n);
            self.ensure_cap(slot, n, elem);
            let dp = self.struct_field(ht.into(), slot, 0);
            let data = self.cg.b.build_load(self.cg.ptr_ty(), dp, "").unwrap().into_pointer_value();
            let et = self.cg.llty(elem);
            for (i, v) in vals.into_iter().enumerate() {
                let p = unsafe { self.cg.b.build_gep(et, data, &[self.usize(i as u64)], "").unwrap() };
                self.store(p, v, elem);
            }
            let lp = self.struct_field(ht.into(), slot, 1);
            self.cg.b.build_store(lp, n).unwrap();
        }
        self.cg.b.build_load(ht, slot, "").unwrap()
    }

    // ===================================================================== control flow

    /// The slot where the branches of an `if`, `match` or `loop` of type
    /// `ty` leave its value: `dest` if given, else a new one (none for `()`).
    fn result_slot(&mut self, ty: &Ty, dest: Option<PointerValue<'ctx>>, name: &str) -> Option<PointerValue<'ctx>> {
        if Cg::is_void(ty) {
            return None;
        }
        Some(match dest {
            Some(d) => d,
            None => self.alloca_for(ty, name),
        })
    }

    /// Evaluate the branch `e` into the result slot, if there is one.
    fn value_to(&mut self, e: &Expr, slot: Option<PointerValue<'ctx>>) -> Option<()> {
        match slot {
            Some(s) => self.value_into(e, s),
            None => self.value(e).map(|_| ()),
        }
    }

    /// The value left in a result slot: the slot itself for a value that
    /// lives in memory.
    fn slot_value(&mut self, slot: Option<PointerValue<'ctx>>, ty: &Ty) -> BV<'ctx> {
        match slot {
            Some(s) if self.cg.in_memory(ty) => s.into(),
            Some(s) => self.load(s, ty),
            None => self.unit(),
        }
    }

    fn if_expr(&mut self, cond: &Expr, then: &Expr, else_: Option<&Expr>, ty: &Ty, dest: Option<PointerValue<'ctx>>) -> Option<BV<'ctx>> {
        let c = tri!(self.scoped_value(cond)).into_int_value();
        let then_bb = self.block_named("then");
        let else_bb = self.block_named("else");
        let merge = self.block_named("endif");
        self.cg.b.build_conditional_branch(c, then_bb, else_bb).unwrap();
        let slot = self.result_slot(ty, dest, "ifv");
        let mut reached = false;
        self.cg.b.position_at_end(then_bb);
        if self.value_to(then, slot).is_some() {
            self.cg.b.build_unconditional_branch(merge).unwrap();
            reached = true;
        }
        self.cg.b.position_at_end(else_bb);
        let ev = match else_ {
            Some(e) => self.value_to(e, slot),
            None => Some(()),
        };
        if ev.is_some() {
            self.cg.b.build_unconditional_branch(merge).unwrap();
            reached = true;
        }
        self.cg.b.position_at_end(merge);
        if !reached {
            self.cg.b.build_unreachable().unwrap();
            return None;
        }
        Some(self.slot_value(slot, ty))
    }

    fn loop_expr(&mut self, body: &Block, id: LoopId, ty: &Ty, dest: Option<PointerValue<'ctx>>) -> Option<BV<'ctx>> {
        let head = self.block_named("loop");
        let exit = self.block_named("loop_end");
        let slot = self.result_slot(ty, dest, "brk");
        let mark = self.cleanups.len();
        self.loops.insert(id, LoopInfo { brk: exit, cont: head, slot, mark });
        self.cg.b.build_unconditional_branch(head).unwrap();
        self.cg.b.position_at_end(head);
        if self.block(body).is_some() && !self.terminated() {
            self.cg.b.build_unconditional_branch(head).unwrap();
        }
        self.cg.b.position_at_end(exit);
        if exit.get_first_use().is_none() {
            self.cg.b.build_unreachable().unwrap();
            return None;
        }
        Some(self.slot_value(slot, ty))
    }

    fn match_expr(&mut self, scrut: &Expr, arms: &[Arm], ty: &Ty, dest: Option<PointerValue<'ctx>>) -> Option<BV<'ctx>> {
        let sp = tri!(self.place(scrut));
        let st = self.t(&scrut.ty).strip_grow().clone();
        let merge = self.block_named("match_end");
        let slot = self.result_slot(ty, dest, "matchv");
        let mut reached = false;
        for arm in arms {
            let next = self.block_named("arm_next");
            self.test_pat(&arm.pat, sp, &st, next);
            self.push_mark();
            if let Some(g) = &arm.guard {
                self.bind_pat(&arm.pat, sp, &st, false);
                let Some(gv) = self.scoped_value(g) else {
                    let m = self.marks.pop().unwrap();
                    self.cleanups.truncate(m);
                    self.cg.b.position_at_end(next);
                    continue;
                };
                let body_bb = self.block_named("guard_ok");
                // Guard failed: bindings were copies, nothing to undo.
                let m = *self.marks.last().unwrap();
                let saved: Vec<_> = self.cleanups[m..].to_vec();
                self.cleanups.truncate(m);
                self.cg.b.build_conditional_branch(gv.into_int_value(), body_bb, next).unwrap();
                self.cg.b.position_at_end(body_bb);
                self.cleanups.extend(saved);
                self.null_moves(&arm.pat, sp, &st);
            } else {
                self.bind_pat(&arm.pat, sp, &st, true);
            }
            match self.value_to(&arm.body, slot) {
                Some(()) => {
                    self.pop_mark();
                    self.cg.b.build_unconditional_branch(merge).unwrap();
                    reached = true;
                }
                None => {
                    let m = self.marks.pop().unwrap();
                    self.cleanups.truncate(m);
                }
            }
            self.cg.b.position_at_end(next);
        }
        // No arm matched: unreachable (matches are exhaustive).
        self.cg.b.build_unreachable().unwrap();
        self.cg.b.position_at_end(merge);
        if !reached {
            self.cg.b.build_unreachable().unwrap();
            return None;
        }
        Some(self.slot_value(slot, ty))
    }

    fn for_range(&mut self, var: LocalId, start: &Expr, end: &Expr, inclusive: bool, body: &Block, id: LoopId) -> Option<BV<'ctx>> {
        let s = tri!(self.value(start)).into_int_value();
        let e = tri!(self.value(end)).into_int_value();
        let ty = self.t(&start.ty);
        let signed = matches!(ty, Ty::Int(i) if i.signed());
        let it = s.get_type();
        let counter = self.alloca(it.into(), "ctr");
        self.cg.b.build_store(counter, s).unwrap();
        let head = self.block_named("for");
        let body_bb = self.block_named("for_body");
        let latch = self.block_named("for_next");
        let exit = self.block_named("for_end");
        let mark = self.cleanups.len();
        self.loops.insert(id, LoopInfo { brk: exit, cont: latch, slot: None, mark });
        self.cg.b.build_unconditional_branch(head).unwrap();
        self.cg.b.position_at_end(head);
        let cur = self.cg.b.build_load(it, counter, "i").unwrap().into_int_value();
        let pred = match (inclusive, signed) {
            (false, true) => IntPredicate::SLT,
            (false, false) => IntPredicate::ULT,
            (true, true) => IntPredicate::SLE,
            (true, false) => IntPredicate::ULE,
        };
        let c = self.cg.b.build_int_compare(pred, cur, e, "").unwrap();
        self.cg.b.build_conditional_branch(c, body_bb, exit).unwrap();
        self.cg.b.position_at_end(body_bb);
        let vslot = self.locals[var.0 as usize].unwrap();
        self.cg.b.build_store(vslot, cur).unwrap();
        if self.block(body).is_some() && !self.terminated() {
            self.cg.b.build_unconditional_branch(latch).unwrap();
        }
        self.cg.b.position_at_end(latch);
        let cur = self.cg.b.build_load(it, counter, "").unwrap().into_int_value();
        if inclusive {
            // Stop before incrementing past `end` (avoids overflow at the type's max).
            let at_end = self.cg.b.build_int_compare(IntPredicate::EQ, cur, e, "").unwrap();
            let inc = self.block_named("for_inc");
            self.cg.b.build_conditional_branch(at_end, exit, inc).unwrap();
            self.cg.b.position_at_end(inc);
        }
        let next = self.cg.b.build_int_add(cur, it.const_int(1, false), "").unwrap();
        self.cg.b.build_store(counter, next).unwrap();
        self.cg.b.build_unconditional_branch(head).unwrap();
        self.cg.b.position_at_end(exit);
        Some(self.unit())
    }

    fn for_array(&mut self, pat: &Pat, array: &Expr, index: LocalId, body: &Block, id: LoopId) -> Option<BV<'ctx>> {
        let ap = tri!(self.place(array));
        let at = self.t(&array.ty).strip_grow().clone();
        let idx = self.locals[index.0 as usize].unwrap();
        self.cg.b.build_store(idx, self.usize(0)).unwrap();
        let head = self.block_named("each");
        let body_bb = self.block_named("each_body");
        let latch = self.block_named("each_next");
        let exit = self.block_named("each_end");
        let mark = self.cleanups.len();
        self.loops.insert(id, LoopInfo { brk: exit, cont: latch, slot: None, mark });
        self.cg.b.build_unconditional_branch(head).unwrap();
        self.cg.b.position_at_end(head);
        let i = self.cg.b.build_load(self.cg.usize_ty(), idx, "i").unwrap().into_int_value();
        // Reload the length every iteration: the array may change in the body.
        let (data, len, elem) = self.array_parts(ap, &at);
        let c = self.cg.b.build_int_compare(IntPredicate::ULT, i, len, "").unwrap();
        self.cg.b.build_conditional_branch(c, body_bb, exit).unwrap();
        self.cg.b.position_at_end(body_bb);
        let et = self.cg.llty(&elem);
        let ep = unsafe { self.cg.b.build_gep(et, data, &[i], "").unwrap() };
        self.push_mark();
        self.bind_pat(pat, ep, &elem, false);
        let r = self.block(body);
        if r.is_some() {
            self.pop_mark();
            if !self.terminated() {
                self.cg.b.build_unconditional_branch(latch).unwrap();
            }
        } else {
            let m = self.marks.pop().unwrap();
            self.cleanups.truncate(m);
        }
        self.cg.b.position_at_end(latch);
        let i = self.cg.b.build_load(self.cg.usize_ty(), idx, "").unwrap().into_int_value();
        let next = self.cg.b.build_int_add(i, self.usize(1), "").unwrap();
        self.cg.b.build_store(idx, next).unwrap();
        self.cg.b.build_unconditional_branch(head).unwrap();
        self.cg.b.position_at_end(exit);
        Some(self.unit())
    }

    // ===================================================================== patterns

    /// Emit tests for `pat` against the value at `ptr`; on failure branch to
    /// `fail`, on success continue in the current block.
    pub fn test_pat(&mut self, pat: &Pat, ptr: PointerValue<'ctx>, ty: &Ty, fail: BasicBlock<'ctx>) {
        match &pat.kind {
            PatKind::Wild | PatKind::Binding { sub: None, .. } => {}
            PatKind::Binding { sub: Some(s), .. } => self.test_pat(s, ptr, ty, fail),
            PatKind::Lit(v) => {
                let val = self.load(ptr, ty);
                let c = match self.cg.const_val(v, ty) {
                    Ok(c) => c,
                    Err(e) => {
                        self.ice(e);
                        return;
                    }
                };
                let eq = self.values_eq(val, c, ty);
                self.branch_or(eq, fail);
            }
            PatKind::Range { lo, hi, inclusive } => {
                let val = self.load(ptr, ty).into_int_value();
                let (Ok(l), Ok(h)) = (self.cg.const_val(lo, ty), self.cg.const_val(hi, ty)) else { return };
                let signed = matches!(ty, Ty::Int(i) if i.signed());
                let (ge, le) = if signed {
                    (IntPredicate::SGE, if *inclusive { IntPredicate::SLE } else { IntPredicate::SLT })
                } else {
                    (IntPredicate::UGE, if *inclusive { IntPredicate::ULE } else { IntPredicate::ULT })
                };
                let a = self.cg.b.build_int_compare(ge, val, l.into_int_value(), "").unwrap();
                let b = self.cg.b.build_int_compare(le, val, h.into_int_value(), "").unwrap();
                let both = self.cg.b.build_and(a, b, "").unwrap();
                self.branch_or(both, fail);
            }
            PatKind::Tuple(ps) => {
                let Ty::Tuple(ts) = ty else { return };
                let lt = self.cg.llty(ty);
                for (i, (p, t)) in ps.iter().zip(ts.clone()).enumerate() {
                    let fp = self.struct_field(lt, ptr, i as u32);
                    self.test_pat(p, fp, &t, fail);
                }
            }
            PatKind::Adt { def, variant, fields } => {
                let Ty::Adt(_, args) = ty else { return };
                let args = args.clone();
                if self.cg.p.is_enum(*def) {
                    let lt = self.cg.llty(ty);
                    let tp = self.struct_field(lt, ptr, 0);
                    let tag = self.cg.b.build_load(self.cg.ctx.i32_type(), tp, "tag").unwrap().into_int_value();
                    let want = self.cg.ctx.i32_type().const_int(*variant as u64, false);
                    let eq = self.cg.b.build_int_compare(IntPredicate::EQ, tag, want, "").unwrap();
                    self.branch_or(eq, fail);
                }
                let fdefs = self.cg.p.adt(*def).variants[*variant].fields.clone();
                for (fi, p) in fields {
                    let ft = fdefs[*fi].ty.subst(&args);
                    let fp = self.field_ptr(ptr, ty, *variant, *fi);
                    self.test_pat(p, fp, &ft, fail);
                }
            }
            PatKind::Or(alts) => {
                let ok = self.block_named("or_ok");
                for (i, a) in alts.iter().enumerate() {
                    if i + 1 == alts.len() {
                        self.test_pat(a, ptr, ty, fail);
                        self.cg.b.build_unconditional_branch(ok).unwrap();
                    } else {
                        let next = self.block_named("or_next");
                        self.test_pat(a, ptr, ty, next);
                        self.cg.b.build_unconditional_branch(ok).unwrap();
                        self.cg.b.position_at_end(next);
                    }
                }
                self.cg.b.position_at_end(ok);
            }
        }
    }

    fn branch_or(&mut self, cond: IntValue<'ctx>, fail: BasicBlock<'ctx>) {
        let ok = self.block_named("pat_ok");
        self.cg.b.build_conditional_branch(cond, ok, fail).unwrap();
        self.cg.b.position_at_end(ok);
    }

    /// Copy matched parts into the pattern's bindings. With `null_moves`,
    /// moved owner values are zeroed in the source.
    pub fn bind_pat(&mut self, pat: &Pat, ptr: PointerValue<'ctx>, ty: &Ty, null_moves: bool) {
        match &pat.kind {
            PatKind::Wild | PatKind::Lit(_) | PatKind::Range { .. } => {}
            PatKind::Or(alts) => {
                if !binds(pat) {
                    return;
                }
                // Every alternative binds the same variables: their drops
                // are registered once, from the first.
                let mark = self.cleanups.len();
                let mut first = None;
                self.through_matching_alt(alts, ptr, ty, &mut |fc, a| {
                    fc.bind_pat(a, ptr, ty, null_moves);
                    let regs = fc.cleanups.split_off(mark);
                    first.get_or_insert(regs);
                });
                self.cleanups.extend(first.unwrap_or_default());
            }
            PatKind::Binding { local, mode, sub } => {
                let slot = self.locals[local.0 as usize].unwrap();
                if self.cg.in_memory(ty) {
                    self.cg.copy_mem(slot, ptr, ty);
                } else {
                    let v = self.load(ptr, ty);
                    self.cg.b.build_store(slot, v).unwrap();
                }
                if *mode == ReadMode::Move {
                    self.register_cleanup(slot, ty);
                    if null_moves && self.cg.p.has_owner(ty) {
                        self.store_zero(ptr, ty);
                    }
                }
                if let Some(s) = sub {
                    self.bind_pat(s, ptr, ty, null_moves);
                }
            }
            PatKind::Tuple(ps) => {
                let Ty::Tuple(ts) = ty else { return };
                let lt = self.cg.llty(ty);
                for (i, (p, t)) in ps.iter().zip(ts.clone()).enumerate() {
                    let fp = self.struct_field(lt, ptr, i as u32);
                    self.bind_pat(p, fp, &t, null_moves);
                }
            }
            PatKind::Adt { def, variant, fields } => {
                let Ty::Adt(_, args) = ty else { return };
                let args = args.clone();
                let fdefs = self.cg.p.adt(*def).variants[*variant].fields.clone();
                for (fi, p) in fields {
                    let ft = fdefs[*fi].ty.subst(&args);
                    let fp = self.field_ptr(ptr, ty, *variant, *fi);
                    self.bind_pat(p, fp, &ft, null_moves);
                }
            }
        }
    }

    /// Zero the sources of moved bindings (after a guard succeeded).
    fn null_moves(&mut self, pat: &Pat, ptr: PointerValue<'ctx>, ty: &Ty) {
        match &pat.kind {
            PatKind::Binding { mode: ReadMode::Move, .. } if self.cg.p.has_owner(ty) => self.store_zero(ptr, ty),
            PatKind::Tuple(ps) => {
                let Ty::Tuple(ts) = ty else { return };
                let lt = self.cg.llty(ty);
                for (i, (p, t)) in ps.iter().zip(ts.clone()).enumerate() {
                    let fp = self.struct_field(lt, ptr, i as u32);
                    self.null_moves(p, fp, &t);
                }
            }
            PatKind::Adt { def, variant, fields } => {
                let Ty::Adt(_, args) = ty else { return };
                let args = args.clone();
                let fdefs = self.cg.p.adt(*def).variants[*variant].fields.clone();
                for (fi, p) in fields {
                    let ft = fdefs[*fi].ty.subst(&args);
                    let fp = self.field_ptr(ptr, ty, *variant, *fi);
                    self.null_moves(p, fp, &ft);
                }
            }
            PatKind::Or(alts) if binds(pat) => {
                self.through_matching_alt(alts, ptr, ty, &mut |fc, a| fc.null_moves(a, ptr, ty));
            }
            _ => {}
        }
    }

    /// Run `f` on the alternative of a `|` pattern that the value at `ptr`
    /// matches (the first that does: the pattern is known to match).
    fn through_matching_alt(&mut self, alts: &[Pat], ptr: PointerValue<'ctx>, ty: &Ty, f: &mut dyn FnMut(&mut Self, &Pat)) {
        let done = self.block_named("or_bound");
        for (i, a) in alts.iter().enumerate() {
            let next = (i + 1 < alts.len()).then(|| self.block_named("or_bind_next"));
            if let Some(next) = next {
                self.test_pat(a, ptr, ty, next);
            }
            f(self, a);
            self.cg.b.build_unconditional_branch(done).unwrap();
            if let Some(next) = next {
                self.cg.b.position_at_end(next);
            }
        }
        self.cg.b.position_at_end(done);
    }

    #[allow(dead_code)]
    pub fn const_int_val(&mut self, v: &ConstVal) -> i128 {
        match v {
            ConstVal::Int(i) => *i,
            _ => 0,
        }
    }
}

/// Whether `p` binds a variable.
fn binds(p: &Pat) -> bool {
    match &p.kind {
        PatKind::Binding { .. } => true,
        PatKind::Tuple(ps) | PatKind::Or(ps) => ps.iter().any(binds),
        PatKind::Adt { fields, .. } => fields.iter().any(|(_, p)| binds(p)),
        PatKind::Wild | PatKind::Lit(_) | PatKind::Range { .. } => false,
    }
}
