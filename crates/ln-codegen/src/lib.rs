//! LLVM code generation for Lunar via inkwell.
//!
//! Generic functions are monomorphized on demand: starting from `main` (or
//! the test harness), every reached `(function, type arguments)` instance is
//! declared and queued, then lowered from THIR.
//!
//! Layouts:
//! * owning arrays and `String`: `{ ptr, len, cap }` handle; the elements live
//!   on a data stack managed by the runtime (`ds_grow` / `ds_free`).
//! * slices, `str`: `{ ptr, len }`; `FmtArgs`: `{ env, thunk }`.
//! * enums: `{ i32 tag, [N x i64] payload }`.
//!
//! Aggregates of more than [`MEM_BYTES`] bytes live in memory (see
//! [`Cg::in_memory`]); everything else is an LLVM value.

mod func;
mod init;
mod ops;

use inkwell::AddressSpace;
use inkwell::OptimizationLevel;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::passes::PassBuilderOptions;
use inkwell::targets::{CodeModel, FileType, RelocMode, Target, TargetData, TargetMachine};
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum, FunctionType, StructType};
use inkwell::values::{BasicValueEnum, FunctionValue, GlobalValue, PointerValue, StructValue};
use ln_diag::{SourceMap, Span};
use ln_sema::Program;
use ln_sema::defs::{Abi, ConstVal, DefData, DefId};
use ln_sema::ty::{ArrKind, FloatTy, IntTy, Ty};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Options {
    /// 0 = no optimization, 1..=3 = LLVM `default<On>`.
    pub opt_level: u32,
    /// Generate the `#[test]` harness instead of calling `main`.
    pub test: bool,
    /// Panic on integer overflow.
    pub overflow_checks: bool,
    /// Write the LLVM IR to this path as well.
    pub emit_llvm: Option<PathBuf>,
}

/// Compile `program` to a native object file at `out`.
pub fn compile(program: &Program, map: &SourceMap, opts: &Options, out: &Path) -> Result<(), String> {
    generate(program, map, opts, |module, tm| tm.write_to_file(module, FileType::Object, out).map_err(|e| format!("could not write object file: {e}")))
}

/// Generate and optimize `program`, returning its textual LLVM IR.
pub fn emit_ir(program: &Program, map: &SourceMap, opts: &Options) -> Result<String, String> {
    generate(program, map, opts, |module, _| Ok(module.print_to_string().to_string()))
}

/// Lower, verify and optimize `program`, then hand the module to `finish`.
/// `opts.emit_llvm` receives the optimized IR, or the unoptimized IR if
/// verification fails (so a compiler bug can be inspected).
fn generate<T>(program: &Program, map: &SourceMap, opts: &Options, finish: impl FnOnce(&Module, &TargetMachine) -> Result<T, String>) -> Result<T, String> {
    init::native();
    let triple = TargetMachine::get_default_triple();
    let target = Target::from_triple(&triple).map_err(|e| format!("unsupported target: {e}"))?;
    let level = match opts.opt_level {
        0 => OptimizationLevel::None,
        1 => OptimizationLevel::Less,
        2 => OptimizationLevel::Default,
        _ => OptimizationLevel::Aggressive,
    };
    let tm =
        target.create_target_machine(&triple, "generic", "", level, RelocMode::PIC, CodeModel::Default).ok_or("could not create an LLVM target machine")?;
    let ctx = Context::create();
    let module = ctx.create_module("lunar");
    module.set_triple(&triple);
    let td = tm.get_target_data();
    module.set_data_layout(&td.get_data_layout());
    let mut cg = Cg {
        ctx: &ctx,
        module,
        b: ctx.create_builder(),
        td,
        p: program,
        map,
        opts: opts.clone(),
        fns: HashMap::new(),
        queue: Vec::new(),
        adt_types: HashMap::new(),
        statics: HashMap::new(),
        strs: HashMap::new(),
        glue: HashMap::new(),
        externs: HashMap::new(),
        counter: 0,
    };
    cg.gen_entry()?;
    while let Some((def, args, f)) = cg.queue.pop() {
        cg.gen_fn(def, &args, f)?;
    }
    let write_ir = |m: &Module| match &opts.emit_llvm {
        Some(p) => m.print_to_file(p).map_err(|e| format!("could not write LLVM IR: {e}")),
        None => Ok(()),
    };
    if let Err(e) = cg.module.verify() {
        write_ir(&cg.module)?;
        return Err(format!("internal compiler error: invalid LLVM IR generated:\n{}", e.to_string_lossy()));
    }
    let pipeline = format!("default<O{}>", opts.opt_level.min(3));
    cg.module.run_passes(&pipeline, &tm, PassBuilderOptions::create()).map_err(|e| format!("LLVM optimization failed: {e}"))?;
    write_ir(&cg.module)?;
    finish(&cg.module, &tm)
}

/// Structs, enums, tuples, fixed arrays and closures bigger than this many
/// bytes live in memory.
pub(crate) const MEM_BYTES: u64 = 64;

pub(crate) struct Cg<'ctx, 'p> {
    pub ctx: &'ctx Context,
    pub module: Module<'ctx>,
    pub b: Builder<'ctx>,
    pub td: TargetData,
    pub p: &'p Program,
    pub map: &'p SourceMap,
    pub opts: Options,
    /// Declared function instances.
    pub fns: HashMap<(DefId, Vec<Ty>), FunctionValue<'ctx>>,
    /// Instances whose bodies still need to be generated.
    pub queue: Vec<(DefId, Vec<Ty>, FunctionValue<'ctx>)>,
    pub adt_types: HashMap<Ty, StructType<'ctx>>,
    pub statics: HashMap<DefId, GlobalValue<'ctx>>,
    pub strs: HashMap<String, GlobalValue<'ctx>>,
    /// Drop glue functions per type.
    pub glue: HashMap<Ty, FunctionValue<'ctx>>,
    /// libc helpers (memcmp, atoi, ...).
    pub externs: HashMap<&'static str, FunctionValue<'ctx>>,
    pub counter: u32,
}

impl<'ctx, 'p> Cg<'ctx, 'p> {
    // ===================================================================== types

    pub fn ptr_ty(&self) -> inkwell::types::PointerType<'ctx> {
        self.ctx.ptr_type(AddressSpace::default())
    }

    pub fn usize_ty(&self) -> inkwell::types::IntType<'ctx> {
        self.ctx.i64_type()
    }

    pub fn unit_ty(&self) -> StructType<'ctx> {
        self.ctx.struct_type(&[], false)
    }

    pub fn slice_ty(&self) -> StructType<'ctx> {
        self.ctx.struct_type(&[self.ptr_ty().into(), self.usize_ty().into()], false)
    }

    pub fn handle_ty(&self) -> StructType<'ctx> {
        self.ctx.struct_type(&[self.ptr_ty().into(), self.usize_ty().into(), self.usize_ty().into()], false)
    }

    pub fn int_ty(&self, i: IntTy) -> inkwell::types::IntType<'ctx> {
        match i.bits() {
            8 => self.ctx.i8_type(),
            16 => self.ctx.i16_type(),
            32 => self.ctx.i32_type(),
            _ => self.ctx.i64_type(),
        }
    }

    pub fn llty(&mut self, t: &Ty) -> BasicTypeEnum<'ctx> {
        match t {
            Ty::Int(i) => self.int_ty(*i).into(),
            Ty::Float(FloatTy::F32) => self.ctx.f32_type().into(),
            Ty::Float(FloatTy::F64) => self.ctx.f64_type().into(),
            Ty::Bool => self.ctx.bool_type().into(),
            Ty::Char => self.ctx.i32_type().into(),
            Ty::Str | Ty::FmtArgs => self.slice_ty().into(),
            Ty::String => self.handle_ty().into(),
            Ty::Tuple(ts) => {
                let fields: Vec<BasicTypeEnum> = ts.iter().map(|t| self.llty(t)).collect();
                self.ctx.struct_type(&fields, false).into()
            }
            Ty::Adt(..) => self.adt_ty(t).into(),
            Ty::Array(e, k) => match k {
                ArrKind::Fixed(n) => self.llty(e).array_type(*n as u32).into(),
                ArrKind::FixedVar(_) => self.llty(e).array_type(0).into(),
                ArrKind::Slice | ArrKind::SliceMut => self.slice_ty().into(),
                ArrKind::Dyn | ArrKind::Grow | ArrKind::Shrink => self.handle_ty().into(),
            },
            Ty::Grow(t) => self.llty(t),
            Ty::Ptr(..) | Ty::FnPtr(..) => self.ptr_ty().into(),
            // The captures: a pointer to each borrowed one, each taken value.
            Ty::Closure(def, args) => {
                let fields: Vec<BasicTypeEnum> = self.p.closure_fields(*def, args).iter().map(|t| self.llty(t)).collect();
                self.ctx.struct_type(&fields, false).into()
            }
            Ty::Opaque(..) => {
                let r = self.p.reveal(t);
                self.llty(&r)
            }
            Ty::Never | Ty::Error | Ty::Param(..) | Ty::Infer(_) => self.unit_ty().into(),
        }
    }

    fn adt_ty(&mut self, t: &Ty) -> StructType<'ctx> {
        if let Some(s) = self.adt_types.get(t) {
            return *s;
        }
        let Ty::Adt(d, args) = t else { unreachable!() };
        let name = self.p.ty_str(t);
        let st = self.ctx.opaque_struct_type(&name);
        self.adt_types.insert(t.clone(), st);
        let adt = self.p.adt(*d).clone();
        if adt.is_enum {
            let mut max = 0u64;
            for v in 0..adt.variants.len() {
                let vs = self.variant_struct(t, v);
                max = max.max(self.td.get_abi_size(&vs));
            }
            let words = max.div_ceil(8) as u32;
            st.set_body(&[self.ctx.i32_type().into(), self.ctx.i64_type().array_type(words).into()], false);
        } else {
            let tys: Vec<Ty> = adt.variants[0].fields.iter().map(|f| f.ty.subst(args)).collect();
            let fields: Vec<BasicTypeEnum> = tys.iter().map(|t| self.llty(t)).collect();
            st.set_body(&fields, false);
        }
        st
    }

    /// The payload layout of enum variant `v` (or the struct itself).
    pub fn variant_struct(&mut self, t: &Ty, v: usize) -> StructType<'ctx> {
        let Ty::Adt(d, args) = t else { unreachable!() };
        let adt = self.p.adt(*d);
        if !adt.is_enum {
            return self.adt_ty(t);
        }
        let tys: Vec<Ty> = adt.variants[v].fields.iter().map(|f| f.ty.subst(args)).collect();
        let fields: Vec<BasicTypeEnum> = tys.iter().map(|t| self.llty(t)).collect();
        self.ctx.struct_type(&fields, false)
    }

    pub fn size_of(&mut self, t: &Ty) -> u64 {
        let l = self.llty(t);
        self.td.get_abi_size(&l)
    }

    pub fn align_of(&mut self, t: &Ty) -> u32 {
        let l = self.llty(t);
        self.td.get_abi_alignment(&l)
    }

    pub fn is_void(t: &Ty) -> bool {
        t.is_unit() || matches!(t, Ty::Never)
    }

    /// Whether values of type `t` live in memory instead of being LLVM
    /// values: structs, enums, tuples, fixed arrays and closures of more
    /// than [`MEM_BYTES`] bytes. LLVM loads and stores a first-class
    /// aggregate one element at a time (and its instruction selection
    /// crashes on big ones), so such a value is always handled through its
    /// address: the value of an expression is a pointer to memory holding
    /// it, which its user may take over; copies are `llvm.memcpy`; a Lunar
    /// function takes one as a pointer to a copy it owns and returns one
    /// through a pointer to the caller's memory (the first parameter).
    /// (`extern` functions only take and return scalars.)
    pub fn in_memory(&mut self, t: &Ty) -> bool {
        let l = self.llty(t);
        matches!(l, BasicTypeEnum::StructType(_) | BasicTypeEnum::ArrayType(_)) && self.td.get_abi_size(&l) > MEM_BYTES
    }

    /// Copy the value of type `t` at `src` to `dst`.
    pub fn copy_mem(&mut self, dst: PointerValue<'ctx>, src: PointerValue<'ctx>, t: &Ty) {
        let size = self.size_of(t);
        let align = self.align_of(t);
        self.b.build_memcpy(dst, align, src, align, self.usize_ty().const_int(size, false)).unwrap();
    }

    /// An alloca in the entry block of `f`.
    pub fn entry_alloca(&self, f: FunctionValue<'ctx>, ty: BasicTypeEnum<'ctx>, name: &str) -> PointerValue<'ctx> {
        let entry = f.get_first_basic_block().unwrap();
        let b = self.ctx.create_builder();
        match entry.get_first_instruction() {
            Some(i) => b.position_before(&i),
            None => b.position_at_end(entry),
        }
        b.build_alloca(ty, name).unwrap()
    }

    /// LLVM function type for a Lunar signature: `(type, passed by pointer)`.
    /// Values that live in memory are passed by pointer too, and one
    /// returned is written through a pointer passed first.
    pub fn fn_ty(&mut self, params: &[(Ty, bool)], ret: &Ty, variadic: bool) -> FunctionType<'ctx> {
        let sret = self.in_memory(ret);
        let mut ps: Vec<BasicMetadataTypeEnum> = Vec::new();
        if sret {
            ps.push(self.ptr_ty().into());
        }
        for (t, by_ref) in params {
            let pt = if *by_ref || self.in_memory(t) { self.ptr_ty().into() } else { self.llty(t).into() };
            ps.push(pt);
        }
        if Self::is_void(ret) || sret { self.ctx.void_type().fn_type(&ps, variadic) } else { self.llty(ret).fn_type(&ps, variadic) }
    }

    // ===================================================================== functions

    fn mangle(&self, def: DefId, args: &[Ty]) -> String {
        let mut s = format!("_LN{}_{}", def.0, self.p.path_of(def));
        if !args.is_empty() {
            s.push('<');
            s.push_str(&args.iter().map(|a| self.p.ty_str(a)).collect::<Vec<_>>().join(","));
            s.push('>');
        }
        s.chars().map(|c| if c.is_ascii_alphanumeric() || "_.$".contains(c) { c } else { '_' }).collect()
    }

    /// Get (declaring and queueing if needed) the instance `def<args>`.
    pub fn instance(&mut self, def: DefId, args: &[Ty]) -> FunctionValue<'ctx> {
        let key = (def, args.to_vec());
        if let Some(f) = self.fns.get(&key) {
            return *f;
        }
        let fd = self.p.fn_data(def).clone();
        let params: Vec<(Ty, bool)> = fd.sig.params.iter().enumerate().map(|(i, p)| (self.p.reveal(&p.ty.subst(args)), self.p.param_by_ref(def, i))).collect();
        let ret = self.p.reveal(&fd.sig.ret.subst(args));
        let fty = self.fn_ty(&params, &ret, fd.variadic);
        let f = if fd.abi == Abi::C {
            let attrs = &self.p.def(def).attrs;
            let name = attrs.link_name.clone().unwrap_or_else(|| self.p.def(def).name.clone());
            match self.module.get_function(&name) {
                Some(f) => f,
                None => self.module.add_function(&name, fty, Some(Linkage::External)),
            }
        } else {
            let name = self.mangle(def, args);
            let f = self.module.add_function(&name, fty, Some(Linkage::Internal));
            if fd.has_body {
                self.queue.push((def, args.to_vec(), f));
            }
            f
        };
        if matches!(ret, Ty::Never) {
            let kind = inkwell::attributes::Attribute::get_named_enum_kind_id("noreturn");
            f.add_attribute(inkwell::attributes::AttributeLoc::Function, self.ctx.create_enum_attribute(kind, 0));
        }
        self.fns.insert(key, f);
        f
    }

    pub fn lang_fn(&mut self, name: &str) -> Result<FunctionValue<'ctx>, String> {
        let def = self.p.lang_item(name).ok_or_else(|| format!("missing lang item `{name}`"))?;
        Ok(self.instance(def, &[]))
    }

    pub fn libc(&mut self, name: &'static str) -> FunctionValue<'ctx> {
        if let Some(f) = self.externs.get(name) {
            return *f;
        }
        let i32t = self.ctx.i32_type();
        let ptr = self.ptr_ty();
        let fty = match name {
            "memcmp" => i32t.fn_type(&[ptr.into(), ptr.into(), self.usize_ty().into()], false),
            "atoi" => i32t.fn_type(&[ptr.into()], false),
            _ => unreachable!("unknown libc helper"),
        };
        let f = self.module.get_function(name).unwrap_or_else(|| self.module.add_function(name, fty, Some(Linkage::External)));
        self.externs.insert(name, f);
        f
    }

    fn gen_fn(&mut self, def: DefId, args: &[Ty], f: FunctionValue<'ctx>) -> Result<(), String> {
        let Some(body) = self.p.bodies.get(&def) else {
            return Err(format!("internal compiler error: no body for `{}`", self.p.path_of(def)));
        };
        func::FnCg::generate(self, f, body, args)
    }

    // ===================================================================== constants

    /// A private global holding the bytes of `s`.
    pub fn str_global(&mut self, s: &str) -> PointerValue<'ctx> {
        if let Some(g) = self.strs.get(s) {
            return g.as_pointer_value();
        }
        let data = self.ctx.const_string(s.as_bytes(), false);
        let g = self.module.add_global(data.get_type(), None, &format!(".str.{}", self.strs.len()));
        g.set_initializer(&data);
        g.set_constant(true);
        g.set_linkage(Linkage::Private);
        g.set_unnamed_addr(true);
        self.strs.insert(s.to_string(), g);
        g.as_pointer_value()
    }

    pub fn const_str(&mut self, s: &str) -> StructValue<'ctx> {
        let ptr = self.str_global(s);
        let len = self.usize_ty().const_int(s.len() as u64, false);
        self.slice_ty().const_named_struct(&[ptr.into(), len.into()])
    }

    pub fn const_val(&mut self, v: &ConstVal, t: &Ty) -> Result<BasicValueEnum<'ctx>, String> {
        Ok(match (v, t) {
            (ConstVal::Int(i), Ty::Int(it)) => self.int_ty(*it).const_int(*i as u64, it.signed()).into(),
            (ConstVal::Int(i), Ty::Float(_)) => self.llty(t).into_float_type().const_float(*i as f64).into(),
            (ConstVal::Float(f), Ty::Float(_)) => self.llty(t).into_float_type().const_float(*f).into(),
            (ConstVal::Int(i), Ty::Ptr(..)) => self.usize_ty().const_int(*i as u64, false).const_to_pointer(self.ptr_ty()).into(),
            (ConstVal::Int(i), Ty::Char) => self.ctx.i32_type().const_int(*i as u64, false).into(),
            (ConstVal::Bool(b), _) => self.ctx.bool_type().const_int(*b as u64, false).into(),
            (ConstVal::Char(c), _) => self.ctx.i32_type().const_int(*c as u64, false).into(),
            (ConstVal::Str(s), _) => self.const_str(s).into(),
            (ConstVal::Unit, _) => self.unit_ty().const_named_struct(&[]).into(),
            (ConstVal::Tuple(vs), Ty::Tuple(ts)) => {
                let vals: Vec<BasicValueEnum> = vs.iter().zip(ts).map(|(v, t)| self.const_val(v, t)).collect::<Result<_, _>>()?;
                let st = self.llty(t).into_struct_type();
                st.const_named_struct(&vals).into()
            }
            (ConstVal::Array(vs), Ty::Array(e, ArrKind::Fixed(_))) => {
                let et = self.llty(e);
                let vals: Vec<BasicValueEnum> = vs.iter().map(|v| self.const_val(v, e)).collect::<Result<_, _>>()?;
                const_array(et, &vals).into()
            }
            (v, t) => return Err(format!("internal compiler error: unsupported constant {v:?} of type {}", self.p.ty_str(t))),
        })
    }

    /// A private global holding constant `v` of type `t`, for a value that
    /// lives in memory.
    pub fn const_global(&mut self, v: &ConstVal, t: &Ty) -> Result<PointerValue<'ctx>, String> {
        let init = self.const_val(v, t)?;
        let g = self.module.add_global(init.get_type(), None, ".const");
        g.set_initializer(&init);
        g.set_constant(true);
        g.set_linkage(Linkage::Private);
        g.set_unnamed_addr(true);
        Ok(g.as_pointer_value())
    }

    pub fn static_ptr(&mut self, def: DefId) -> Result<PointerValue<'ctx>, String> {
        if let Some(g) = self.statics.get(&def) {
            return Ok(g.as_pointer_value());
        }
        let DefData::Static(s) = &self.p.def(def).data else { return Err("not a static".into()) };
        let (ty, mutable, value) = (s.ty.clone(), s.mutable, s.value.clone());
        let llty = self.llty(&ty);
        let g = self.module.add_global(llty, None, &self.mangle(def, &[]));
        let init = match &value {
            Some(v) => self.const_val(v, &ty)?,
            None => llty.const_zero(),
        };
        g.set_initializer(&init);
        g.set_constant(!mutable);
        g.set_linkage(Linkage::Internal);
        self.statics.insert(def, g);
        Ok(g.as_pointer_value())
    }

    pub fn location(&self, span: Span) -> (String, u32, u32) {
        self.map.location(span)
    }

    // ===================================================================== entry point

    fn gen_entry(&mut self) -> Result<(), String> {
        let i32t = self.ctx.i32_type();
        let fty = i32t.fn_type(&[i32t.into(), self.ptr_ty().into()], false);
        let main = self.module.add_function("main", fty, Some(Linkage::External));
        let entry = self.ctx.append_basic_block(main, "entry");
        self.b.position_at_end(entry);
        let argc = main.get_nth_param(0).unwrap().into_int_value();
        let argv = main.get_nth_param(1).unwrap().into_pointer_value();
        let rt_init = self.lang_fn("rt_init")?;
        let argv_int = self.b.build_ptr_to_int(argv, self.usize_ty(), "argv").unwrap();
        self.b.build_call(rt_init, &[argc.into(), argv_int.into()], "").unwrap();
        if self.opts.test {
            let tests = self.p.tests.clone();
            let run_all = self.ctx.append_basic_block(main, "run_all");
            let pick = self.ctx.append_basic_block(main, "pick");
            let has_arg = self.b.build_int_compare(inkwell::IntPredicate::SGE, argc, i32t.const_int(2, false), "has_arg").unwrap();
            self.b.build_conditional_branch(has_arg, pick, run_all).unwrap();
            self.b.position_at_end(run_all);
            for &t in &tests {
                let f = self.instance(t, &[]);
                self.b.build_call(f, &[], "").unwrap();
            }
            self.b.build_return(Some(&i32t.const_zero())).unwrap();
            self.b.position_at_end(pick);
            let arg1p = unsafe { self.b.build_gep(self.ptr_ty(), argv, &[self.usize_ty().const_int(1, false)], "arg1p").unwrap() };
            let arg1 = self.b.build_load(self.ptr_ty(), arg1p, "arg1").unwrap();
            let atoi = self.libc("atoi");
            let idx = self.b.build_call(atoi, &[arg1.into()], "idx").unwrap().try_as_basic_value().basic().unwrap().into_int_value();
            let done = self.ctx.append_basic_block(main, "done");
            let mut cases = Vec::new();
            for (i, &t) in tests.iter().enumerate() {
                let bb = self.ctx.append_basic_block(main, &format!("test{i}"));
                cases.push((i32t.const_int(i as u64, false), bb, t));
            }
            let switch_cases: Vec<_> = cases.iter().map(|(v, bb, _)| (*v, *bb)).collect();
            self.b.build_switch(idx, done, &switch_cases).unwrap();
            for (_, bb, t) in cases {
                self.b.position_at_end(bb);
                let f = self.instance(t, &[]);
                self.b.build_call(f, &[], "").unwrap();
                self.b.build_unconditional_branch(done).unwrap();
            }
            self.b.position_at_end(done);
            self.b.build_return(Some(&i32t.const_zero())).unwrap();
        } else {
            let user_main = self.p.main.ok_or("no `main` function")?;
            let f = self.instance(user_main, &[]);
            let r = self.b.build_call(f, &[], "").unwrap();
            let code = match r.try_as_basic_value().basic() {
                Some(v) => v.into_int_value(),
                None => i32t.const_zero(),
            };
            self.b.build_return(Some(&code)).unwrap();
        }
        Ok(())
    }
}

/// Build a constant array of `vals` with element type `et`.
pub(crate) fn const_array<'ctx>(et: BasicTypeEnum<'ctx>, vals: &[BasicValueEnum<'ctx>]) -> inkwell::values::ArrayValue<'ctx> {
    match et {
        BasicTypeEnum::IntType(t) => t.const_array(&vals.iter().map(|v| v.into_int_value()).collect::<Vec<_>>()),
        BasicTypeEnum::FloatType(t) => t.const_array(&vals.iter().map(|v| v.into_float_value()).collect::<Vec<_>>()),
        BasicTypeEnum::PointerType(t) => t.const_array(&vals.iter().map(|v| v.into_pointer_value()).collect::<Vec<_>>()),
        BasicTypeEnum::StructType(t) => t.const_array(&vals.iter().map(|v| v.into_struct_value()).collect::<Vec<_>>()),
        BasicTypeEnum::ArrayType(t) => t.const_array(&vals.iter().map(|v| v.into_array_value()).collect::<Vec<_>>()),
        BasicTypeEnum::VectorType(t) => t.const_array(&vals.iter().map(|v| v.into_vector_value()).collect::<Vec<_>>()),
        BasicTypeEnum::ScalableVectorType(t) => t.const_array(&vals.iter().map(|v| v.into_scalable_vector_value()).collect::<Vec<_>>()),
    }
}
