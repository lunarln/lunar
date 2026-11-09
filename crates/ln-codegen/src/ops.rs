//! Calls, intrinsics, builtin trait impls, arithmetic, casts, format
//! strings, drop glue and runtime panics.

use crate::Cg;
use crate::func::{BV, FnCg, tri};
use inkwell::intrinsics::Intrinsic;
use inkwell::types::BasicMetadataTypeEnum;
use inkwell::values::{BasicMetadataValueEnum, FunctionValue, IntValue, PointerValue};
use inkwell::{FloatPredicate, IntPredicate};
use ln_ast::{BinOp, FmtSpec, UnOp};
use ln_diag::Span;
use ln_sema::defs::{DefId, ParamMode};
use ln_sema::thir::*;
use ln_sema::traits::MethodImpl;
use ln_sema::ty::{ArrKind, FloatTy, IntTy, TraitRef, Ty};

impl<'a, 'ctx, 'p> FnCg<'a, 'ctx, 'p> {
    // ===================================================================== calls

    /// Whether an argument expression only borrows an existing place.
    fn is_borrowed(e: &Expr) -> bool {
        match &e.kind {
            ExprKind::Read { mode, .. } => *mode != ReadMode::Move,
            ExprKind::Coerce { expr, kind: Coercion::Weaken } => Self::is_borrowed(expr),
            _ => false,
        }
    }

    /// The place a borrowing argument reads, if it reads one in place.
    fn borrowed_place(e: &Expr) -> Option<&Expr> {
        match &e.kind {
            ExprKind::Read { place, mode } if *mode != ReadMode::Move => Some(place),
            // Owner kinds and `[T; &mut]` -> `[T; &]` share a layout.
            ExprKind::Coerce { expr, kind: Coercion::Weaken } => Self::borrowed_place(expr),
            _ => None,
        }
    }

    /// The declaration whose signature decides how arguments are passed
    /// (`None` for calls through function pointers, which take values).
    fn callee_def(&self, callee: &Callee) -> Option<DefId> {
        match callee {
            Callee::Fn { def, .. } | Callee::Intrinsic { def, .. } => Some(*def),
            Callee::Trait { trait_ref, method, .. } => self.cg.p.trait_data(trait_ref.trait_id).methods.iter().find(|m| &m.name == method).map(|m| m.def),
            Callee::Indirect(_) | Callee::Value { .. } => None,
        }
    }

    /// The parameter modes of a callable value of type `t` (after
    /// instantiation): a closure's own parameters, or a function pointer's.
    fn value_modes(&self, t: &Ty) -> Option<Vec<ParamMode>> {
        match t {
            Ty::Closure(def, args) => match self.cg.p.closure_sig(*def, args) {
                Ty::FnPtr(ps, _) => Some(ps.iter().map(|(m, _)| *m).collect()),
                _ => None,
            },
            Ty::FnPtr(ps, _) => Some(ps.iter().map(|(m, _)| *m).collect()),
            _ => None,
        }
    }

    /// Call a closure: its function takes the captures first, each from the
    /// closure's value at `env` (through the pointer for a borrowed one,
    /// in place for a held one), then the arguments.
    fn call_closure(&mut self, def: DefId, targs: &[Ty], env: PointerValue<'ctx>, vals: &[BV<'ctx>]) -> Option<BV<'ctx>> {
        let info = self.cg.p.closures[&def].clone();
        let cty = Ty::Closure(def, targs.to_vec());
        let lt = self.cg.llty(&cty);
        let fields = self.cg.p.closure_fields(def, targs);
        let mut all: Vec<BV<'ctx>> = Vec::new();
        for (i, cap) in info.captures.iter().enumerate() {
            let fp = self.struct_field(lt, env, i as u32);
            let at = if cap.by_ref { self.cg.b.build_load(self.cg.ptr_ty(), fp, "").unwrap().into_pointer_value() } else { fp };
            if self.cg.p.param_by_ref(def, i) {
                all.push(at.into());
            } else {
                let t = cap.ty.subst(targs);
                let v = self.load(at, &t);
                if cap.mode == ParamMode::Take && self.cg.p.has_owner(&t) {
                    // Taken by the call: the closure no longer owns it.
                    let ft = self.cg.llty(&fields[i]);
                    self.cg.b.build_store(at, ft.const_zero()).unwrap();
                }
                all.push(v);
            }
        }
        all.extend_from_slice(vals);
        let f = self.cg.instance(def, targs);
        self.emit_call(f, &all)
    }

    pub fn call(&mut self, callee: &Callee, args: &[Arg], ret: &Ty, span: Span) -> Option<BV<'ctx>> {
        let indirect = match callee {
            Callee::Indirect(f) => Some(tri!(self.value(f)).into_pointer_value()),
            _ => None,
        };
        let def = self.callee_def(callee);
        // How each argument is passed: from the callee's declaration, or for
        // a call through a pointer, from the modes in the pointer's type.
        let ptr_modes: Option<Vec<ParamMode>> = match callee {
            Callee::Indirect(f) | Callee::Value { callee: f, .. } => {
                let t = self.t(&f.ty);
                self.value_modes(&t)
            }
            _ => None,
        };
        // The callable value: a closure's place (or a temporary holding a
        // consumed one), or a function pointer.
        let value_callee = match callee {
            Callee::Value { callee: f, .. } => {
                let t = self.t(&f.ty);
                Some(match &t {
                    Ty::FnPtr(..) => tri!(self.value(f)).into_pointer_value(),
                    _ if f.is_place() => tri!(self.place(f)),
                    _ => {
                        // A consumed closure: what it still holds after the
                        // call is dropped with the statement's temporaries.
                        let v = tri!(self.value(f));
                        let tmp = self.alloca_for(&t, "closure");
                        self.cg.b.build_store(tmp, v).unwrap();
                        self.register_cleanup(tmp, &t);
                        tmp
                    }
                })
            }
            _ => None,
        };
        let n = args.len();
        let by_ref_at: Vec<bool> = (0..n)
            .map(|i| match &ptr_modes {
                Some(ms) => ms.get(i).is_some_and(|m| *m != ParamMode::Take),
                None => def.is_some_and(|d| self.cg.p.param_by_ref(d, i)),
            })
            .collect();
        let taken_at: Vec<bool> = (0..n)
            .map(|i| match &ptr_modes {
                Some(ms) => ms.get(i) == Some(&ParamMode::Take),
                None => def.is_some_and(|d| self.cg.p.fn_data(d).sig.params.get(i).is_some_and(|p| p.mode == ParamMode::Take)),
            })
            .collect();
        let mut vals: Vec<BV<'ctx>> = Vec::new();
        // Types of arguments passed by pointer, for built-in implementations
        // that need the values.
        let mut by_ref_tys: Vec<Option<Ty>> = Vec::new();
        for (i, a) in args.iter().enumerate() {
            match a {
                Arg::Value(e) => {
                    let by_ref = by_ref_at[i];
                    if by_ref {
                        // A borrowed parameter gets the address of the
                        // caller's place, or of a temporary that lives until
                        // the end of the statement.
                        by_ref_tys.push(Some(self.t(&e.ty)));
                        if let Some(place) = Self::borrowed_place(e) {
                            vals.push(tri!(self.place(place)).into());
                            continue;
                        }
                        let v = tri!(self.value(e));
                        let ty = self.t(&e.ty);
                        let tmp = self.alloca_for(&ty, "argtmp");
                        self.cg.b.build_store(tmp, v).unwrap();
                        self.register_cleanup(tmp, &ty);
                        vals.push(tmp.into());
                        continue;
                    }
                    by_ref_tys.push(None);
                    let v = tri!(self.value(e));
                    let ty = self.t(&e.ty);
                    let taken = taken_at[i];
                    if self.cg.p.has_owner(&ty) && !Self::is_borrowed(e) && !taken {
                        // An owned temporary lent to a borrowing parameter:
                        // dropped after the statement. (A parameter that takes
                        // ownership frees it itself.)
                        let tmp = self.alloca_for(&ty, "argtmp");
                        self.cg.b.build_store(tmp, v).unwrap();
                        self.register_cleanup(tmp, &ty);
                    }
                    vals.push(v);
                }
                Arg::InOut(p) => {
                    by_ref_tys.push(None);
                    vals.push(tri!(self.place(p)).into());
                }
            }
        }
        let result = match callee {
            Callee::Fn { def, args: targs } => {
                let targs: Vec<Ty> = targs.iter().map(|t| self.t(t)).collect();
                let f = self.cg.instance(*def, &targs);
                self.emit_call(f, &vals)
            }
            Callee::Trait { trait_ref, self_ty, method, method_args } => {
                let tr = TraitRef { trait_id: trait_ref.trait_id, args: trait_ref.args.iter().map(|t| self.t(t)).collect() };
                let st = self.t(self_ty);
                let margs: Vec<Ty> = method_args.iter().map(|t| self.t(t)).collect();
                match self.cg.p.resolve_method(&tr, &st, method, &margs) {
                    Some(MethodImpl::Fn { def, args }) => {
                        let f = self.cg.instance(def, &args);
                        self.emit_call(f, &vals)
                    }
                    Some(MethodImpl::Builtin { lang, method }) => {
                        // Built-in implementations work on values.
                        let mut vs = vals.clone();
                        for (v, t) in vs.iter_mut().zip(&by_ref_tys) {
                            if let Some(t) = t {
                                let lt = self.cg.llty(t);
                                *v = self.cg.b.build_load(lt, v.into_pointer_value(), "").unwrap();
                            }
                        }
                        Some(self.builtin(&lang, &method, &vs, &st, ret, span))
                    }
                    None => {
                        let (s, tn) = (self.cg.p.ty_str(&st), self.cg.p.def(tr.trait_id).name.clone());
                        self.ice(format!("no implementation of `{tn}::{method}` for `{s}`"));
                        return None;
                    }
                }
            }
            Callee::Intrinsic { name, args: targs, .. } => {
                let targs: Vec<Ty> = targs.iter().map(|t| self.t(t)).collect();
                return self.intrinsic(name, &targs, &vals, ret, span);
            }
            Callee::Indirect(f) => {
                let Ty::FnPtr(params, r) = self.t(&f.ty) else {
                    self.ice("indirect call through a non-function");
                    return None;
                };
                let ps: Vec<(Ty, bool)> = params.iter().map(|(m, t)| (t.clone(), *m != ParamMode::Take)).collect();
                let fty = self.cg.fn_ty(&ps, &r, false);
                let args: Vec<BasicMetadataValueEnum> = vals.iter().map(|v| (*v).into()).collect();
                let cs = self.cg.b.build_indirect_call(fty, indirect.unwrap(), &args, "").unwrap();
                cs.try_as_basic_value().basic()
            }
            Callee::Value { callee: f, .. } => {
                let at = value_callee.unwrap();
                match self.t(&f.ty) {
                    Ty::Closure(def, targs) => self.call_closure(def, &targs, at, &vals),
                    Ty::FnPtr(params, r) => {
                        let ps: Vec<(Ty, bool)> = params.iter().map(|(m, t)| (t.clone(), *m != ParamMode::Take)).collect();
                        let fty = self.cg.fn_ty(&ps, &r, false);
                        let args: Vec<BasicMetadataValueEnum> = vals.iter().map(|v| (*v).into()).collect();
                        self.cg.b.build_indirect_call(fty, at, &args, "").unwrap().try_as_basic_value().basic()
                    }
                    other => {
                        let s = self.cg.p.ty_str(&other);
                        self.ice(format!("call of a value of type `{s}`"));
                        return None;
                    }
                }
            }
        };
        if matches!(ret, Ty::Never) {
            if !self.terminated() {
                self.cg.b.build_unreachable().unwrap();
            }
            return None;
        }
        Some(result.unwrap_or_else(|| self.unit()))
    }

    fn emit_call(&mut self, f: FunctionValue<'ctx>, vals: &[BV<'ctx>]) -> Option<BV<'ctx>> {
        let args: Vec<BasicMetadataValueEnum> = vals.iter().map(|v| (*v).into()).collect();
        let cs = self.cg.b.build_call(f, &args, "").unwrap();
        cs.try_as_basic_value().basic()
    }

    // ===================================================================== intrinsics

    fn intrinsic(&mut self, name: &str, targs: &[Ty], vals: &[BV<'ctx>], ret: &Ty, span: Span) -> Option<BV<'ctx>> {
        let b = &self.cg.b;
        let ptr = |v: BV<'ctx>| v.into_pointer_value();
        let v: BV<'ctx> = match name {
            "size_of" => {
                let n = self.cg.size_of(&targs[0]);
                self.usize(n).into()
            }
            "align_of" => {
                let n = self.cg.align_of(&targs[0]);
                self.usize(n as u64).into()
            }
            "slice_len" | "str_len" => b.build_extract_value(vals[0].into_struct_value(), 1, "len").unwrap(),
            "slice_ptr" | "slice_mut_ptr" => b.build_extract_value(vals[0].into_struct_value(), 0, "ptr").unwrap(),
            "slice_from_raw" | "slice_mut_from_raw" => self.make_slice(ptr(vals[0]), vals[1].into_int_value()),
            "str_bytes" | "str_from_bytes" | "char_from_u32" => vals[0],
            "arr_push" => {
                self.emit_push(ptr(vals[0]), vals[1], &targs[0]);
                self.unit()
            }
            "arr_extend" => {
                self.emit_extend(ptr(vals[0]), vals[1], &targs[0]);
                self.unit()
            }
            "string_push_str" => {
                self.emit_extend(ptr(vals[0]), vals[1], &Ty::Int(IntTy::U8));
                self.unit()
            }
            "arr_reserve" => {
                let h = ptr(vals[0]);
                self.ensure_cap(h, vals[1].into_int_value(), &targs[0]);
                self.unit()
            }
            "arr_cap" | "string_cap" => {
                let h = ptr(vals[0]);
                let size = if name == "arr_cap" { self.cg.size_of(&targs[0]) } else { 1 };
                self.emit_ds_cap(h, self.usize(size), vals[1].into_int_value());
                self.unit()
            }
            "arr_truncate" | "string_truncate" => {
                let h = ptr(vals[0]);
                let ht = self.cg.handle_ty();
                let lp = self.struct_field(ht.into(), h, 1);
                let len = self.cg.b.build_load(self.cg.usize_ty(), lp, "").unwrap().into_int_value();
                let n = vals[1].into_int_value();
                let shorter = self.cg.b.build_int_compare(IntPredicate::ULT, n, len, "").unwrap();
                let new = self.cg.b.build_select(shorter, n, len, "").unwrap();
                self.cg.b.build_store(lp, new).unwrap();
                self.unit()
            }
            "arr_from_slice" => {
                let ht = self.cg.handle_ty();
                let slot = self.alloca(ht.into(), "dyn");
                self.cg.b.build_store(slot, ht.const_zero()).unwrap();
                self.emit_extend(slot, vals[0], &targs[0]);
                self.cg.b.build_load(ht, slot, "").unwrap()
            }
            "string_new" => self.cg.handle_ty().const_zero().into(),
            "string_as_str" => {
                let h = vals[0].into_struct_value();
                let data = self.cg.b.build_extract_value(h, 0, "").unwrap().into_pointer_value();
                let len = self.cg.b.build_extract_value(h, 1, "").unwrap().into_int_value();
                self.make_slice(data, len)
            }
            "fmt_write" => {
                let fa = vals[0].into_struct_value();
                let env = self.cg.b.build_extract_value(fa, 0, "env").unwrap();
                let thunk_i = self.cg.b.build_extract_value(fa, 1, "thunk").unwrap().into_int_value();
                let thunk = self.cg.b.build_int_to_ptr(thunk_i, self.cg.ptr_ty(), "").unwrap();
                let fty = self.cg.ctx.void_type().fn_type(&[self.cg.ptr_ty().into(), self.cg.ptr_ty().into()], false);
                self.cg.b.build_indirect_call(fty, thunk, &[env.into(), vals[1].into()], "").unwrap();
                self.unit()
            }
            "ptr_read" => {
                let lt = self.cg.llty(&targs[0]);
                self.cg.b.build_load(lt, ptr(vals[0]), "").unwrap()
            }
            "ptr_write" => {
                self.cg.b.build_store(ptr(vals[0]), vals[1]).unwrap();
                self.unit()
            }
            "ptr_offset" | "ptr_offset_mut" => {
                let lt = self.cg.llty(&targs[0]);
                unsafe { self.cg.b.build_gep(lt, ptr(vals[0]), &[vals[1].into_int_value()], "").unwrap().into() }
            }
            "ptr_null" | "ptr_null_mut" => self.cg.ptr_ty().const_null().into(),
            "addr_of_mut" => vals[0],
            "wrapping_add" => self.cg.b.build_int_add(vals[0].into_int_value(), vals[1].into_int_value(), "").unwrap().into(),
            "wrapping_sub" => self.cg.b.build_int_sub(vals[0].into_int_value(), vals[1].into_int_value(), "").unwrap().into(),
            "wrapping_mul" => self.cg.b.build_int_mul(vals[0].into_int_value(), vals[1].into_int_value(), "").unwrap().into(),
            "abort" => {
                let trap = Intrinsic::find("llvm.trap").unwrap().get_declaration(&self.cg.module, &[]).unwrap();
                self.cg.b.build_call(trap, &[], "").unwrap();
                self.cg.b.build_unreachable().unwrap();
                return None;
            }
            "unreachable" => {
                self.cg.b.build_unreachable().unwrap();
                return None;
            }
            other => {
                self.ice(format!("unknown intrinsic `{other}`"));
                let _ = span;
                return None;
            }
        };
        if matches!(ret, Ty::Never) {
            self.cg.b.build_unreachable().unwrap();
            return None;
        }
        Some(v)
    }

    /// Give the empty array behind handle `h` a slot for exactly `count`
    /// elements of `size` bytes.
    pub fn emit_ds_cap(&mut self, h: PointerValue<'ctx>, size: IntValue<'ctx>, count: IntValue<'ctx>) {
        match self.cg.lang_fn("ds_cap") {
            Ok(f) => {
                self.cg.b.build_call(f, &[h.into(), size.into(), count.into()], "").unwrap();
            }
            Err(e) => self.ice(e),
        }
    }

    /// Make sure the array behind handle `h` has room for `extra` more elements.
    pub fn ensure_cap(&mut self, h: PointerValue<'ctx>, extra: IntValue<'ctx>, elem: &Ty) {
        let size = self.cg.size_of(elem);
        if size == 0 {
            return;
        }
        let ht = self.cg.handle_ty();
        let lp = self.struct_field(ht.into(), h, 1);
        let cp = self.struct_field(ht.into(), h, 2);
        let len = self.cg.b.build_load(self.cg.usize_ty(), lp, "len").unwrap().into_int_value();
        let cap = self.cg.b.build_load(self.cg.usize_ty(), cp, "cap").unwrap().into_int_value();
        // `extra > cap - len` rather than `len + extra > cap`: `extra` may be
        // user-controlled and the addition could wrap (`len <= cap` always).
        let room = self.cg.b.build_int_sub(cap, len, "room").unwrap();
        let full = self.cg.b.build_int_compare(IntPredicate::UGT, extra, room, "").unwrap();
        let grow = self.block_named("grow");
        let done = self.block_named("grown");
        self.cg.b.build_conditional_branch(full, grow, done).unwrap();
        self.cg.b.position_at_end(grow);
        match self.cg.lang_fn("ds_grow") {
            Ok(f) => {
                self.cg.b.build_call(f, &[h.into(), self.usize(size).into(), extra.into()], "").unwrap();
            }
            Err(e) => self.ice(e),
        }
        self.cg.b.build_unconditional_branch(done).unwrap();
        self.cg.b.position_at_end(done);
    }

    fn emit_push(&mut self, h: PointerValue<'ctx>, v: BV<'ctx>, elem: &Ty) {
        self.ensure_cap(h, self.usize(1), elem);
        let ht = self.cg.handle_ty();
        let dp = self.struct_field(ht.into(), h, 0);
        let lp = self.struct_field(ht.into(), h, 1);
        let data = self.cg.b.build_load(self.cg.ptr_ty(), dp, "").unwrap().into_pointer_value();
        let len = self.cg.b.build_load(self.cg.usize_ty(), lp, "").unwrap().into_int_value();
        let et = self.cg.llty(elem);
        let slot = unsafe { self.cg.b.build_gep(et, data, &[len], "").unwrap() };
        self.cg.b.build_store(slot, v).unwrap();
        let n = self.cg.b.build_int_add(len, self.usize(1), "").unwrap();
        self.cg.b.build_store(lp, n).unwrap();
    }

    fn emit_extend(&mut self, h: PointerValue<'ctx>, slice: BV<'ctx>, elem: &Ty) {
        let s = slice.into_struct_value();
        let src = self.cg.b.build_extract_value(s, 0, "").unwrap().into_pointer_value();
        let n = self.cg.b.build_extract_value(s, 1, "").unwrap().into_int_value();
        let nonzero = self.cg.b.build_int_compare(IntPredicate::NE, n, self.usize(0), "").unwrap();
        let copy = self.block_named("extend");
        let done = self.block_named("extended");
        self.cg.b.build_conditional_branch(nonzero, copy, done).unwrap();
        self.cg.b.position_at_end(copy);
        self.ensure_cap(h, n, elem);
        let ht = self.cg.handle_ty();
        let dp = self.struct_field(ht.into(), h, 0);
        let lp = self.struct_field(ht.into(), h, 1);
        let data = self.cg.b.build_load(self.cg.ptr_ty(), dp, "").unwrap().into_pointer_value();
        let len = self.cg.b.build_load(self.cg.usize_ty(), lp, "").unwrap().into_int_value();
        let et = self.cg.llty(elem);
        let dst = unsafe { self.cg.b.build_gep(et, data, &[len], "").unwrap() };
        let size = self.cg.size_of(elem);
        let bytes = self.cg.b.build_int_mul(n, self.usize(size), "").unwrap();
        let align = self.cg.align_of(elem);
        self.cg.b.build_memmove(dst, align, src, align, bytes).unwrap();
        let new_len = self.cg.b.build_int_add(len, n, "").unwrap();
        self.cg.b.build_store(lp, new_len).unwrap();
        self.cg.b.build_unconditional_branch(done).unwrap();
        self.cg.b.position_at_end(done);
    }

    // ===================================================================== builtin impls

    fn builtin(&mut self, lang: &str, method: &str, vals: &[BV<'ctx>], self_ty: &Ty, ret: &Ty, span: Span) -> BV<'ctx> {
        let op = match method {
            "add" => Some(BinOp::Add),
            "sub" => Some(BinOp::Sub),
            "mul" => Some(BinOp::Mul),
            "div" => Some(BinOp::Div),
            "rem" => Some(BinOp::Rem),
            "bitand" => Some(BinOp::BitAnd),
            "bitor" => Some(BinOp::BitOr),
            "bitxor" => Some(BinOp::BitXor),
            "shl" => Some(BinOp::Shl),
            "shr" => Some(BinOp::Shr),
            "lt" => Some(BinOp::Lt),
            "le" => Some(BinOp::Le),
            "gt" => Some(BinOp::Gt),
            "ge" => Some(BinOp::Ge),
            _ => None,
        };
        match (lang, method) {
            ("neg", _) => self.unary(UnOp::Neg, vals[0], self_ty, span),
            ("not", _) => self.unary(UnOp::Not, vals[0], self_ty, span),
            ("eq", "eq") => self.values_eq(vals[0], vals[1], self_ty).into(),
            ("ord", "cmp") => self.ordering(vals[0], vals[1], self_ty),
            ("ord", _) if matches!(self_ty, Ty::Str) => {
                let o = self.ordering(vals[0], vals[1], self_ty);
                let tag = self.ordering_tag(o);
                let (pred, val) = match method {
                    "lt" => (IntPredicate::EQ, 0),
                    "gt" => (IntPredicate::EQ, 2),
                    "le" => (IntPredicate::NE, 2),
                    _ => (IntPredicate::NE, 0),
                };
                self.cg.b.build_int_compare(pred, tag, self.cg.ctx.i32_type().const_int(val, false), "").unwrap().into()
            }
            ("clone", _) => vals[0],
            ("default", _) => {
                let lt = self.cg.llty(ret);
                lt.const_zero()
            }
            _ => match op {
                Some(op) => self.binop(op, vals[0], vals[1], self_ty, span),
                None => {
                    self.ice(format!("unknown builtin `{lang}::{method}`"));
                    self.unit()
                }
            },
        }
    }

    fn ordering_ty(&mut self) -> Ty {
        Ty::Adt(self.cg.p.lang_item("ordering").expect("ordering lang item"), vec![])
    }

    fn ordering_tag(&mut self, v: BV<'ctx>) -> IntValue<'ctx> {
        self.cg.b.build_extract_value(v.into_struct_value(), 0, "").unwrap().into_int_value()
    }

    /// `a.cmp(b)` for primitives and `str`, as an `Ordering` value.
    fn ordering(&mut self, a: BV<'ctx>, b: BV<'ctx>, ty: &Ty) -> BV<'ctx> {
        let i32t = self.cg.ctx.i32_type();
        let (lt, gt) = if matches!(ty, Ty::Str) {
            let r = self.str_cmp(a, b);
            (
                self.cg.b.build_int_compare(IntPredicate::SLT, r, i32t.const_zero(), "").unwrap(),
                self.cg.b.build_int_compare(IntPredicate::SGT, r, i32t.const_zero(), "").unwrap(),
            )
        } else {
            (self.binop(BinOp::Lt, a, b, ty, Span::DUMMY).into_int_value(), self.binop(BinOp::Gt, a, b, ty, Span::DUMMY).into_int_value())
        };
        let one = self.cg.b.build_select(gt, i32t.const_int(2, false), i32t.const_int(1, false), "").unwrap().into_int_value();
        let tag = self.cg.b.build_select(lt, i32t.const_zero(), one, "").unwrap().into_int_value();
        let ot = self.ordering_ty();
        let lt_ty = self.cg.llty(&ot);
        let st = lt_ty.into_struct_type();
        let agg = self.cg.b.build_insert_value(st.const_zero(), tag, 0, "").unwrap();
        agg.into_struct_value().into()
    }

    /// Three-way comparison of two `str`s: negative, zero or positive.
    fn str_cmp(&mut self, a: BV<'ctx>, b: BV<'ctx>) -> IntValue<'ctx> {
        let (pa, la) = self.split_slice(a);
        let (pb, lb) = self.split_slice(b);
        let shorter = self.cg.b.build_int_compare(IntPredicate::ULT, la, lb, "").unwrap();
        let m = self.cg.b.build_select(shorter, la, lb, "").unwrap().into_int_value();
        let memcmp = self.cg.libc("memcmp");
        let r = self.cg.b.build_call(memcmp, &[pa.into(), pb.into(), m.into()], "").unwrap().try_as_basic_value().basic().unwrap().into_int_value();
        let i32t = self.cg.ctx.i32_type();
        let is_zero = self.cg.b.build_int_compare(IntPredicate::EQ, r, i32t.const_zero(), "").unwrap();
        let len_lt = self.cg.b.build_int_compare(IntPredicate::ULT, la, lb, "").unwrap();
        let len_gt = self.cg.b.build_int_compare(IntPredicate::UGT, la, lb, "").unwrap();
        let by_len = self.cg.b.build_select(len_gt, i32t.const_int(1, false), i32t.const_zero(), "").unwrap().into_int_value();
        let by_len = self.cg.b.build_select(len_lt, i32t.const_int(u64::MAX, true), by_len, "").unwrap().into_int_value();
        self.cg.b.build_select(is_zero, by_len, r, "").unwrap().into_int_value()
    }

    fn split_slice(&mut self, v: BV<'ctx>) -> (PointerValue<'ctx>, IntValue<'ctx>) {
        let s = v.into_struct_value();
        let p = self.cg.b.build_extract_value(s, 0, "").unwrap().into_pointer_value();
        let l = self.cg.b.build_extract_value(s, 1, "").unwrap().into_int_value();
        (p, l)
    }

    /// Equality for primitives and `str`.
    pub fn values_eq(&mut self, a: BV<'ctx>, b: BV<'ctx>, ty: &Ty) -> IntValue<'ctx> {
        match ty {
            Ty::Str => {
                let (pa, la) = self.split_slice(a);
                let (pb, lb) = self.split_slice(b);
                let same_len = self.cg.b.build_int_compare(IntPredicate::EQ, la, lb, "").unwrap();
                let cmp_bb = self.block_named("streq");
                let done = self.block_named("streq_done");
                let start = self.cg.b.get_insert_block().unwrap();
                self.cg.b.build_conditional_branch(same_len, cmp_bb, done).unwrap();
                self.cg.b.position_at_end(cmp_bb);
                let memcmp = self.cg.libc("memcmp");
                let r = self.cg.b.build_call(memcmp, &[pa.into(), pb.into(), la.into()], "").unwrap().try_as_basic_value().basic().unwrap().into_int_value();
                let eq = self.cg.b.build_int_compare(IntPredicate::EQ, r, self.cg.ctx.i32_type().const_zero(), "").unwrap();
                let cmp_end = self.cg.b.get_insert_block().unwrap();
                self.cg.b.build_unconditional_branch(done).unwrap();
                self.cg.b.position_at_end(done);
                let phi = self.cg.b.build_phi(self.cg.ctx.bool_type(), "").unwrap();
                phi.add_incoming(&[(&self.cg.ctx.bool_type().const_zero(), start), (&eq, cmp_end)]);
                phi.as_basic_value().into_int_value()
            }
            t if t.is_unit() => self.cg.ctx.bool_type().const_int(1, false),
            _ => self.binop(BinOp::Eq, a, b, ty, Span::DUMMY).into_int_value(),
        }
    }

    // ===================================================================== arithmetic

    pub fn unary(&mut self, op: UnOp, v: BV<'ctx>, ty: &Ty, span: Span) -> BV<'ctx> {
        match (op, ty) {
            (UnOp::Neg, Ty::Float(_)) => self.cg.b.build_float_neg(v.into_float_value(), "").unwrap().into(),
            (UnOp::Neg, Ty::Int(_)) => {
                let zero = v.into_int_value().get_type().const_zero();
                self.binop(BinOp::Sub, zero.into(), v, ty, span)
            }
            (UnOp::Not, _) => self.cg.b.build_not(v.into_int_value(), "").unwrap().into(),
            _ => {
                self.ice("bad unary operand");
                v
            }
        }
    }

    pub fn binop(&mut self, op: BinOp, a: BV<'ctx>, b: BV<'ctx>, ty: &Ty, span: Span) -> BV<'ctx> {
        if let Ty::Float(_) = ty {
            let (x, y) = (a.into_float_value(), b.into_float_value());
            let bd = &self.cg.b;
            return match op {
                BinOp::Add => bd.build_float_add(x, y, "").unwrap().into(),
                BinOp::Sub => bd.build_float_sub(x, y, "").unwrap().into(),
                BinOp::Mul => bd.build_float_mul(x, y, "").unwrap().into(),
                BinOp::Div => bd.build_float_div(x, y, "").unwrap().into(),
                BinOp::Rem => bd.build_float_rem(x, y, "").unwrap().into(),
                BinOp::Eq => bd.build_float_compare(FloatPredicate::OEQ, x, y, "").unwrap().into(),
                BinOp::Ne => bd.build_float_compare(FloatPredicate::UNE, x, y, "").unwrap().into(),
                BinOp::Lt => bd.build_float_compare(FloatPredicate::OLT, x, y, "").unwrap().into(),
                BinOp::Le => bd.build_float_compare(FloatPredicate::OLE, x, y, "").unwrap().into(),
                BinOp::Gt => bd.build_float_compare(FloatPredicate::OGT, x, y, "").unwrap().into(),
                BinOp::Ge => bd.build_float_compare(FloatPredicate::OGE, x, y, "").unwrap().into(),
                _ => {
                    self.ice("bad float operator");
                    a
                }
            };
        }
        let signed = matches!(ty, Ty::Int(i) if i.signed());
        let (x, y) = (a.into_int_value(), b.into_int_value());
        let cmp = |p_s: IntPredicate, p_u: IntPredicate| if signed { p_s } else { p_u };
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul if self.cg.opts.overflow_checks && matches!(ty, Ty::Int(_)) => {
                let (name, what) = match (op, signed) {
                    (BinOp::Add, true) => ("llvm.sadd.with.overflow", "add"),
                    (BinOp::Add, false) => ("llvm.uadd.with.overflow", "add"),
                    (BinOp::Sub, true) => ("llvm.ssub.with.overflow", "subtract"),
                    (BinOp::Sub, false) => ("llvm.usub.with.overflow", "subtract"),
                    (BinOp::Mul, true) => ("llvm.smul.with.overflow", "multiply"),
                    _ => ("llvm.umul.with.overflow", "multiply"),
                };
                let f = Intrinsic::find(name).unwrap().get_declaration(&self.cg.module, &[x.get_type().into()]).unwrap();
                let r = self.cg.b.build_call(f, &[x.into(), y.into()], "").unwrap().try_as_basic_value().basic().unwrap().into_struct_value();
                let val = self.cg.b.build_extract_value(r, 0, "").unwrap();
                let ov = self.cg.b.build_extract_value(r, 1, "").unwrap().into_int_value();
                let bad = self.block_named("overflow");
                let ok = self.block_named("no_overflow");
                self.cg.b.build_conditional_branch(ov, bad, ok).unwrap();
                self.cg.b.position_at_end(bad);
                self.panic_msg(&format!("attempt to {what} with overflow"), span);
                self.cg.b.position_at_end(ok);
                val
            }
            BinOp::Add => self.cg.b.build_int_add(x, y, "").unwrap().into(),
            BinOp::Sub => self.cg.b.build_int_sub(x, y, "").unwrap().into(),
            BinOp::Mul => self.cg.b.build_int_mul(x, y, "").unwrap().into(),
            BinOp::Div | BinOp::Rem => {
                let it = x.get_type();
                let is_zero = self.cg.b.build_int_compare(IntPredicate::EQ, y, it.const_zero(), "").unwrap();
                let bad = self.block_named("div_zero");
                let ok = self.block_named("div_ok");
                self.cg.b.build_conditional_branch(is_zero, bad, ok).unwrap();
                self.cg.b.position_at_end(bad);
                let what = if op == BinOp::Div { "divide" } else { "calculate the remainder" };
                self.panic_msg(&format!("attempt to {what} by zero"), span);
                self.cg.b.position_at_end(ok);
                if signed {
                    // MIN / -1 overflows.
                    let bits = it.get_bit_width();
                    let min = it.const_int(1u64 << (bits - 1), false);
                    let is_min = self.cg.b.build_int_compare(IntPredicate::EQ, x, min, "").unwrap();
                    let neg1 = self.cg.b.build_int_compare(IntPredicate::EQ, y, it.const_all_ones(), "").unwrap();
                    let both = self.cg.b.build_and(is_min, neg1, "").unwrap();
                    let bad = self.block_named("div_overflow");
                    let ok2 = self.block_named("div_ok2");
                    self.cg.b.build_conditional_branch(both, bad, ok2).unwrap();
                    self.cg.b.position_at_end(bad);
                    self.panic_msg(&format!("attempt to {what} with overflow"), span);
                    self.cg.b.position_at_end(ok2);
                    if op == BinOp::Div {
                        self.cg.b.build_int_signed_div(x, y, "").unwrap().into()
                    } else {
                        self.cg.b.build_int_signed_rem(x, y, "").unwrap().into()
                    }
                } else if op == BinOp::Div {
                    self.cg.b.build_int_unsigned_div(x, y, "").unwrap().into()
                } else {
                    self.cg.b.build_int_unsigned_rem(x, y, "").unwrap().into()
                }
            }
            BinOp::BitAnd | BinOp::And => self.cg.b.build_and(x, y, "").unwrap().into(),
            BinOp::BitOr | BinOp::Or => self.cg.b.build_or(x, y, "").unwrap().into(),
            BinOp::BitXor => self.cg.b.build_xor(x, y, "").unwrap().into(),
            BinOp::Shl | BinOp::Shr => {
                let bits = x.get_type().get_bit_width() as u64;
                let amt = self.cg.b.build_and(y, y.get_type().const_int(bits - 1, false), "").unwrap();
                if op == BinOp::Shl {
                    self.cg.b.build_left_shift(x, amt, "").unwrap().into()
                } else {
                    self.cg.b.build_right_shift(x, amt, signed, "").unwrap().into()
                }
            }
            BinOp::Eq => self.cg.b.build_int_compare(IntPredicate::EQ, x, y, "").unwrap().into(),
            BinOp::Ne => self.cg.b.build_int_compare(IntPredicate::NE, x, y, "").unwrap().into(),
            BinOp::Lt => self.cg.b.build_int_compare(cmp(IntPredicate::SLT, IntPredicate::ULT), x, y, "").unwrap().into(),
            BinOp::Le => self.cg.b.build_int_compare(cmp(IntPredicate::SLE, IntPredicate::ULE), x, y, "").unwrap().into(),
            BinOp::Gt => self.cg.b.build_int_compare(cmp(IntPredicate::SGT, IntPredicate::UGT), x, y, "").unwrap().into(),
            BinOp::Ge => self.cg.b.build_int_compare(cmp(IntPredicate::SGE, IntPredicate::UGE), x, y, "").unwrap().into(),
        }
    }

    pub fn cast(&mut self, v: BV<'ctx>, from: &Ty, to: &Ty) -> BV<'ctx> {
        let b = &self.cg.b;
        match (from, to) {
            (a, b2) if a == b2 => v,
            (Ty::Int(fi), Ty::Int(ti)) => b.build_int_cast_sign_flag(v.into_int_value(), self.cg.int_ty(*ti), fi.signed(), "").unwrap().into(),
            (Ty::Bool | Ty::Char, Ty::Int(ti)) => b.build_int_cast_sign_flag(v.into_int_value(), self.cg.int_ty(*ti), false, "").unwrap().into(),
            (Ty::Int(_), Ty::Char) => b.build_int_cast_sign_flag(v.into_int_value(), self.cg.ctx.i32_type(), false, "").unwrap().into(),
            (Ty::Int(fi), Ty::Float(ft)) => {
                let t = if *ft == FloatTy::F32 { self.cg.ctx.f32_type() } else { self.cg.ctx.f64_type() };
                if fi.signed() {
                    b.build_signed_int_to_float(v.into_int_value(), t, "").unwrap().into()
                } else {
                    b.build_unsigned_int_to_float(v.into_int_value(), t, "").unwrap().into()
                }
            }
            (Ty::Float(_), Ty::Int(ti)) => {
                // Saturating, like Rust.
                let name = if ti.signed() { "llvm.fptosi.sat" } else { "llvm.fptoui.sat" };
                let it = self.cg.int_ty(*ti);
                let f = Intrinsic::find(name).unwrap().get_declaration(&self.cg.module, &[it.into(), v.get_type()]).unwrap();
                self.cg.b.build_call(f, &[v.into()], "").unwrap().try_as_basic_value().basic().unwrap()
            }
            (Ty::Float(_), Ty::Float(ft)) => {
                let t = if *ft == FloatTy::F32 { self.cg.ctx.f32_type() } else { self.cg.ctx.f64_type() };
                b.build_float_cast(v.into_float_value(), t, "").unwrap().into()
            }
            (Ty::Ptr(..) | Ty::FnPtr(..), Ty::Int(ti)) => b.build_ptr_to_int(v.into_pointer_value(), self.cg.int_ty(*ti), "").unwrap().into(),
            (Ty::Int(_), Ty::Ptr(..)) => b.build_int_to_ptr(v.into_int_value(), self.cg.ptr_ty(), "").unwrap().into(),
            (Ty::Ptr(..) | Ty::FnPtr(..), Ty::Ptr(..)) => v,
            _ => {
                let (f, t) = (self.cg.p.ty_str(from), self.cg.p.ty_str(to));
                self.ice(format!("unsupported cast from {f} to {t}"));
                v
            }
        }
    }

    // ===================================================================== format strings

    /// Build a `FmtArgs` value: an environment holding the interpolated
    /// values and a generated thunk that writes them into a `String`.
    pub fn fmt_args(&mut self, pieces: &[FmtPiece]) -> Option<BV<'ctx>> {
        let mut arg_vals = Vec::new();
        let mut arg_tys = Vec::new();
        for p in pieces {
            if let FmtPiece::Arg(e, _) = p {
                arg_vals.push(tri!(self.value(e)));
                arg_tys.push(self.t(&e.ty));
            }
        }
        let env_fields: Vec<_> = arg_tys.iter().map(|t| self.cg.llty(t)).collect();
        let env_ty = self.cg.ctx.struct_type(&env_fields, false);
        let env = self.alloca(env_ty.into(), "fmt_env");
        for (i, v) in arg_vals.iter().enumerate() {
            let fp = self.struct_field(env_ty.into(), env, i as u32);
            self.cg.b.build_store(fp, *v).unwrap();
        }
        let thunk = match self.fmt_thunk(pieces, &arg_tys, env_ty) {
            Ok(f) => f,
            Err(e) => {
                self.ice(e);
                return None;
            }
        };
        let thunk_i = self.cg.b.build_ptr_to_int(thunk.as_global_value().as_pointer_value(), self.cg.usize_ty(), "").unwrap();
        let st = self.cg.slice_ty();
        let a = self.cg.b.build_insert_value(st.get_undef(), env, 0, "").unwrap();
        let a = self.cg.b.build_insert_value(a, thunk_i, 1, "").unwrap();
        Some(a.into_struct_value().into())
    }

    fn fmt_thunk(&mut self, pieces: &[FmtPiece], arg_tys: &[Ty], env_ty: inkwell::types::StructType<'ctx>) -> Result<FunctionValue<'ctx>, String> {
        let cg = &mut *self.cg;
        let ptr = cg.ptr_ty();
        let fty = cg.ctx.void_type().fn_type(&[ptr.into(), ptr.into()], false);
        cg.counter += 1;
        let f = cg.module.add_function(&format!("_LN_fmt{}", cg.counter), fty, Some(inkwell::module::Linkage::Internal));
        let saved = cg.b.get_insert_block();
        let entry = cg.ctx.append_basic_block(f, "entry");
        cg.b.position_at_end(entry);
        let env = f.get_nth_param(0).unwrap().into_pointer_value();
        let out = f.get_nth_param(1).unwrap().into_pointer_value();
        let mut ai = 0;
        for p in pieces {
            match p {
                FmtPiece::Text(s) => {
                    let sv = cg.const_str(s);
                    thunk_extend(cg, f, out, sv.into())?;
                }
                FmtPiece::Arg(_, spec) => {
                    let (lang, method, name) = match spec {
                        FmtSpec::Display => ("display", "fmt", "Display"),
                        FmtSpec::Debug => ("debug", "debug", "Debug"),
                    };
                    let trait_id = cg.p.lang_item(lang).ok_or_else(|| format!("missing lang item `{lang}`"))?;
                    let t = &arg_tys[ai];
                    let fp = cg.b.build_struct_gep(env_ty, env, ai as u32, "").unwrap();
                    let lt = cg.llty(t);
                    let v = cg.b.build_load(lt, fp, "").unwrap();
                    let tr = TraitRef { trait_id, args: vec![] };
                    match cg.p.resolve_method(&tr, t, method, &[]) {
                        Some(MethodImpl::Fn { def, args }) => {
                            let callee = cg.instance(def, &args);
                            // `fmt(&self, ..)` takes the value's address.
                            let recv = if cg.p.param_by_ref(def, 0) { fp.into() } else { v.into() };
                            cg.b.build_call(callee, &[recv, out.into()], "").unwrap();
                        }
                        _ => return Err(format!("no `{name}` implementation for `{}`", cg.p.ty_str(t))),
                    }
                    ai += 1;
                }
            }
        }
        cg.b.build_return(None).unwrap();
        if let Some(bb) = saved {
            cg.b.position_at_end(bb);
        }
        Ok(f)
    }

    // ===================================================================== drops

    /// Drop the owned contents of the value at `p` (no-op for zeroed handles).
    pub fn drop_in_place(&mut self, p: PointerValue<'ctx>, t: &Ty) {
        if !self.cg.p.has_owner(t) {
            return;
        }
        match glue(self.cg, t) {
            Ok(f) => {
                self.cg.b.build_call(f, &[p.into()], "").unwrap();
            }
            Err(e) => self.ice(e),
        }
    }

    // ===================================================================== panics

    pub fn panic_msg(&mut self, msg: &str, span: Span) {
        let (file, line, col) = self.cg.location(span);
        let f = match self.cg.lang_fn("panic_str") {
            Ok(f) => f,
            Err(e) => {
                self.ice(e);
                self.cg.b.build_unreachable().unwrap();
                return;
            }
        };
        let m = self.cg.const_str(msg);
        let fl = self.cg.const_str(&file);
        let i32t = self.cg.ctx.i32_type();
        self.cg.b.build_call(f, &[m.into(), fl.into(), i32t.const_int(line as u64, false).into(), i32t.const_int(col as u64, false).into()], "").unwrap();
        self.cg.b.build_unreachable().unwrap();
    }

    pub fn panic_bounds(&mut self, i: IntValue<'ctx>, len: IntValue<'ctx>, span: Span) {
        let (file, line, col) = self.cg.location(span);
        let f = match self.cg.lang_fn("panic_bounds") {
            Ok(f) => f,
            Err(e) => {
                self.ice(e);
                self.cg.b.build_unreachable().unwrap();
                return;
            }
        };
        let fl = self.cg.const_str(&file);
        let i32t = self.cg.ctx.i32_type();
        self.cg
            .b
            .build_call(f, &[i.into(), len.into(), fl.into(), i32t.const_int(line as u64, false).into(), i32t.const_int(col as u64, false).into()], "")
            .unwrap();
        self.cg.b.build_unreachable().unwrap();
    }
}

/// Append the `str` value `s` to the `String` behind `out` (inside a thunk).
fn thunk_extend<'ctx>(cg: &mut Cg<'ctx, '_>, f: FunctionValue<'ctx>, out: PointerValue<'ctx>, s: inkwell::values::BasicValueEnum<'ctx>) -> Result<(), String> {
    let sv = s.into_struct_value();
    let src = cg.b.build_extract_value(sv, 0, "").unwrap().into_pointer_value();
    let n = cg.b.build_extract_value(sv, 1, "").unwrap().into_int_value();
    if n.get_zero_extended_constant() == Some(0) {
        return Ok(());
    }
    let ht = cg.handle_ty();
    let lp = cg.b.build_struct_gep(ht, out, 1, "").unwrap();
    let cp = cg.b.build_struct_gep(ht, out, 2, "").unwrap();
    let len = cg.b.build_load(cg.usize_ty(), lp, "").unwrap().into_int_value();
    let cap = cg.b.build_load(cg.usize_ty(), cp, "").unwrap().into_int_value();
    let room = cg.b.build_int_sub(cap, len, "").unwrap();
    let full = cg.b.build_int_compare(IntPredicate::UGT, n, room, "").unwrap();
    let grow = cg.ctx.append_basic_block(f, "grow");
    let done = cg.ctx.append_basic_block(f, "grown");
    cg.b.build_conditional_branch(full, grow, done).unwrap();
    cg.b.position_at_end(grow);
    let ds_grow = cg.lang_fn("ds_grow")?;
    cg.b.build_call(ds_grow, &[out.into(), cg.usize_ty().const_int(1, false).into(), n.into()], "").unwrap();
    cg.b.build_unconditional_branch(done).unwrap();
    cg.b.position_at_end(done);
    let dp = cg.b.build_struct_gep(ht, out, 0, "").unwrap();
    let data = cg.b.build_load(cg.ptr_ty(), dp, "").unwrap().into_pointer_value();
    let len = cg.b.build_load(cg.usize_ty(), lp, "").unwrap().into_int_value();
    let dst = unsafe { cg.b.build_gep(cg.ctx.i8_type(), data, &[len], "").unwrap() };
    cg.b.build_memcpy(dst, 1, src, 1, n).unwrap();
    let nl = cg.b.build_int_add(len, n, "").unwrap();
    cg.b.build_store(lp, nl).unwrap();
    Ok(())
}

/// The drop glue function for `t`: `void drop(ptr)`.
fn glue<'ctx>(cg: &mut Cg<'ctx, '_>, t: &Ty) -> Result<FunctionValue<'ctx>, String> {
    if let Some(f) = cg.glue.get(t) {
        return Ok(*f);
    }
    let ptr = cg.ptr_ty();
    let fty = cg.ctx.void_type().fn_type(&[BasicMetadataTypeEnum::from(ptr)], false);
    cg.counter += 1;
    let f = cg.module.add_function(&format!("_LN_drop{}", cg.counter), fty, Some(inkwell::module::Linkage::Internal));
    cg.glue.insert(t.clone(), f);
    let saved = cg.b.get_insert_block();
    let entry = cg.ctx.append_basic_block(f, "entry");
    cg.b.position_at_end(entry);
    let p = f.get_nth_param(0).unwrap().into_pointer_value();
    match t {
        Ty::String | Ty::Array(_, ArrKind::Dyn | ArrKind::Grow | ArrKind::Shrink) => {
            let ht = cg.handle_ty();
            let dp = cg.b.build_struct_gep(ht, p, 0, "").unwrap();
            let data = cg.b.build_load(ptr, dp, "").unwrap();
            let free = cg.lang_fn("ds_free")?;
            cg.b.build_call(free, &[data.into()], "").unwrap();
        }
        Ty::Bang(inner) => {
            let g = glue(cg, inner)?;
            cg.b.position_at_end(entry);
            cg.b.build_call(g, &[p.into()], "").unwrap();
        }
        Ty::Tuple(_) | Ty::Closure(..) => {
            let ts = match t {
                Ty::Tuple(ts) => ts.clone(),
                Ty::Closure(def, args) => cg.p.closure_fields(*def, args),
                _ => unreachable!(),
            };
            let lt = cg.llty(t);
            for (i, ft) in ts.iter().enumerate() {
                if cg.p.has_owner(ft) {
                    let fp = cg.b.build_struct_gep(lt, p, i as u32, "").unwrap();
                    let g = glue(cg, ft)?;
                    cg.b.build_call(g, &[fp.into()], "").unwrap();
                }
            }
        }
        Ty::Adt(d, args) => {
            let adt = cg.p.adt(*d).clone();
            let lt = cg.llty(t);
            if adt.is_enum {
                let tp = cg.b.build_struct_gep(lt, p, 0, "").unwrap();
                let tag = cg.b.build_load(cg.ctx.i32_type(), tp, "").unwrap().into_int_value();
                let payload = cg.b.build_struct_gep(lt, p, 1, "").unwrap();
                let done = cg.ctx.append_basic_block(f, "done");
                let mut cases = Vec::new();
                for (vi, v) in adt.variants.iter().enumerate() {
                    let ftys: Vec<Ty> = v.fields.iter().map(|x| x.ty.subst(args)).collect();
                    if !ftys.iter().any(|ft| cg.p.has_owner(ft)) {
                        continue;
                    }
                    let bb = cg.ctx.append_basic_block(f, "variant");
                    cases.push((cg.ctx.i32_type().const_int(vi as u64, false), bb, vi, ftys));
                }
                let sw: Vec<_> = cases.iter().map(|(c, bb, _, _)| (*c, *bb)).collect();
                cg.b.build_switch(tag, done, &sw).unwrap();
                for (_, bb, vi, ftys) in cases {
                    cg.b.position_at_end(bb);
                    let vs = cg.variant_struct(t, vi);
                    for (fi, ft) in ftys.iter().enumerate() {
                        if cg.p.has_owner(ft) {
                            let fp = cg.b.build_struct_gep(vs, payload, fi as u32, "").unwrap();
                            let g = glue(cg, ft)?;
                            cg.b.position_at_end(bb);
                            cg.b.build_call(g, &[fp.into()], "").unwrap();
                        }
                    }
                    cg.b.build_unconditional_branch(done).unwrap();
                }
                cg.b.position_at_end(done);
            } else {
                let ftys: Vec<Ty> = adt.variants[0].fields.iter().map(|x| x.ty.subst(args)).collect();
                for (i, ft) in ftys.iter().enumerate() {
                    if cg.p.has_owner(ft) {
                        let cur = cg.b.get_insert_block().unwrap();
                        let fp = cg.b.build_struct_gep(lt, p, i as u32, "").unwrap();
                        let g = glue(cg, ft)?;
                        cg.b.position_at_end(cur);
                        cg.b.build_call(g, &[fp.into()], "").unwrap();
                    }
                }
            }
        }
        _ => {}
    }
    // Nested glue generation restores the builder to our current block.
    if let Some(cur) = cg.b.get_insert_block()
        && cur.get_terminator().is_none()
    {
        cg.b.build_return(None).unwrap();
    }
    if let Some(bb) = saved {
        cg.b.position_at_end(bb);
    }
    Ok(f)
}
