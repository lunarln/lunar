//! Semantic types.

use crate::defs::DefId;
pub use ln_ast::ParamMode;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IntTy {
    I8,
    I16,
    I32,
    I64,
    Isize,
    U8,
    U16,
    U32,
    U64,
    Usize,
}

impl IntTy {
    pub const ALL: [IntTy; 10] = [IntTy::I8, IntTy::I16, IntTy::I32, IntTy::I64, IntTy::Isize, IntTy::U8, IntTy::U16, IntTy::U32, IntTy::U64, IntTy::Usize];

    pub fn name(self) -> &'static str {
        match self {
            IntTy::I8 => "i8",
            IntTy::I16 => "i16",
            IntTy::I32 => "i32",
            IntTy::I64 => "i64",
            IntTy::Isize => "isize",
            IntTy::U8 => "u8",
            IntTy::U16 => "u16",
            IntTy::U32 => "u32",
            IntTy::U64 => "u64",
            IntTy::Usize => "usize",
        }
    }

    pub fn from_name(s: &str) -> Option<IntTy> {
        IntTy::ALL.into_iter().find(|t| t.name() == s)
    }

    pub fn signed(self) -> bool {
        matches!(self, IntTy::I8 | IntTy::I16 | IntTy::I32 | IntTy::I64 | IntTy::Isize)
    }

    /// Width in bits (pointer-sized types are 64-bit on every supported target).
    pub fn bits(self) -> u32 {
        match self {
            IntTy::I8 | IntTy::U8 => 8,
            IntTy::I16 | IntTy::U16 => 16,
            IntTy::I32 | IntTy::U32 => 32,
            IntTy::I64 | IntTy::U64 | IntTy::Isize | IntTy::Usize => 64,
        }
    }

    pub fn max(self) -> u128 {
        if self.signed() {
            (1u128 << (self.bits() - 1)) - 1
        } else if self.bits() == 128 {
            u128::MAX
        } else {
            (1u128 << self.bits()) - 1
        }
    }

    /// Magnitude of the minimum value (0 for unsigned).
    pub fn min_magnitude(self) -> u128 {
        if self.signed() { 1u128 << (self.bits() - 1) } else { 0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FloatTy {
    F32,
    F64,
}

impl FloatTy {
    pub fn name(self) -> &'static str {
        match self {
            FloatTy::F32 => "f32",
            FloatTy::F64 => "f64",
        }
    }
}

/// The kind of an array type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ArrKind {
    /// `[T; N]`: inline, fixed length.
    Fixed(u64),
    /// `[T; _]` whose length is still being inferred (inference variable id).
    FixedVar(u32),
    /// `[T]`: sized at construction, then frozen. Owns a data stack.
    Dyn,
    /// `[T; ..+]`: grow-only. Owns a data stack.
    Grow,
    /// `[T; -..+]`: grows and shrinks. Owns a data stack.
    Shrink,
    /// `[T; &]`
    Slice,
    /// `[T; &mut]`
    SliceMut,
}

impl ArrKind {
    /// Owning arrays live on their own data stack and are move-only.
    pub fn is_owner(self) -> bool {
        matches!(self, ArrKind::Dyn | ArrKind::Grow | ArrKind::Shrink)
    }

    pub fn is_slice(self) -> bool {
        matches!(self, ArrKind::Slice | ArrKind::SliceMut)
    }

    pub fn is_fixed(self) -> bool {
        matches!(self, ArrKind::Fixed(_) | ArrKind::FixedVar(_))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Ty {
    Int(IntTy),
    Float(FloatTy),
    Bool,
    Char,
    /// UTF-8 view (pointer + length).
    Str,
    /// Owned UTF-8 text; laid out exactly like `[u8; -..+]`.
    String,
    /// Implicitly built from a string literal; parameter-only.
    FmtArgs,
    /// `()` is the empty tuple.
    Tuple(Vec<Ty>),
    /// Struct or enum instance.
    Adt(DefId, Vec<Ty>),
    Array(Box<Ty>, ArrKind),
    /// `!T` parameter marker.
    Bang(Box<Ty>),
    Ptr(Box<Ty>, bool /* mutable */),
    /// `fn(A, &B, &mut C): R`: each parameter's mode is part of the type.
    FnPtr(Vec<(ParamMode, Ty)>, Box<Ty>),
    /// The type of one closure expression (its function definition), with
    /// the generic arguments of the function it is written in.
    Closure(DefId, Vec<Ty>),
    /// `impl Trait` number `idx` in the return type of function `def`, with
    /// that function's generic arguments. Callers only know its bounds; code
    /// generation uses the type the function's body returns.
    Opaque(DefId, u32, Vec<Ty>),
    /// Generic parameter number `idx` of the enclosing item (impl params come first).
    Param(u32, String),
    /// Inference variable.
    Infer(u32),
    Never,
    /// Produced after an error; unifies with anything.
    Error,
}

impl Ty {
    pub fn unit() -> Ty {
        Ty::Tuple(Vec::new())
    }

    pub fn is_unit(&self) -> bool {
        matches!(self, Ty::Tuple(v) if v.is_empty())
    }

    pub fn usize() -> Ty {
        Ty::Int(IntTy::Usize)
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Ty::Error)
    }

    pub fn is_integral(&self) -> bool {
        matches!(self, Ty::Int(_))
    }

    pub fn is_numeric(&self) -> bool {
        matches!(self, Ty::Int(_) | Ty::Float(_))
    }

    pub fn is_primitive_scalar(&self) -> bool {
        matches!(self, Ty::Int(_) | Ty::Float(_) | Ty::Bool | Ty::Char)
    }

    pub fn strip_bang(&self) -> &Ty {
        match self {
            Ty::Bang(t) => t,
            t => t,
        }
    }

    /// Element type and kind, for array types.
    pub fn as_array(&self) -> Option<(&Ty, ArrKind)> {
        match self {
            Ty::Array(e, k) => Some((e, *k)),
            _ => None,
        }
    }

    /// Visit every type contained in `self` (including itself).
    pub fn walk(&self, f: &mut impl FnMut(&Ty)) {
        f(self);
        match self {
            Ty::Tuple(ts) | Ty::Adt(_, ts) | Ty::Closure(_, ts) | Ty::Opaque(_, _, ts) => ts.iter().for_each(|t| t.walk(f)),
            Ty::Array(t, _) | Ty::Bang(t) | Ty::Ptr(t, _) => t.walk(f),
            Ty::FnPtr(ps, r) => {
                ps.iter().for_each(|(_, t)| t.walk(f));
                r.walk(f);
            }
            _ => {}
        }
    }

    pub fn has_params(&self) -> bool {
        let mut any = false;
        self.walk(&mut |t| any |= matches!(t, Ty::Param(..)));
        any
    }

    pub fn has_infer(&self) -> bool {
        let mut any = false;
        self.walk(&mut |t| any |= matches!(t, Ty::Infer(_) | Ty::Array(_, ArrKind::FixedVar(_))));
        any
    }

    pub fn has_error(&self) -> bool {
        let mut any = false;
        self.walk(&mut |t| any |= matches!(t, Ty::Error));
        any
    }

    /// Apply `f` bottom-up to every type node.
    pub fn map(&self, f: &mut impl FnMut(Ty) -> Ty) -> Ty {
        let t = match self {
            Ty::Tuple(ts) => Ty::Tuple(ts.iter().map(|t| t.map(f)).collect()),
            Ty::Adt(d, ts) => Ty::Adt(*d, ts.iter().map(|t| t.map(f)).collect()),
            Ty::Closure(d, ts) => Ty::Closure(*d, ts.iter().map(|t| t.map(f)).collect()),
            Ty::Opaque(d, i, ts) => Ty::Opaque(*d, *i, ts.iter().map(|t| t.map(f)).collect()),
            Ty::Array(t, k) => Ty::Array(Box::new(t.map(f)), *k),
            Ty::Bang(t) => Ty::Bang(Box::new(t.map(f))),
            Ty::Ptr(t, m) => Ty::Ptr(Box::new(t.map(f)), *m),
            Ty::FnPtr(ps, r) => Ty::FnPtr(ps.iter().map(|(m, t)| (*m, t.map(f))).collect(), Box::new(r.map(f))),
            t => t.clone(),
        };
        f(t)
    }

    /// Substitute generic parameters by index.
    pub fn subst(&self, args: &[Ty]) -> Ty {
        if args.is_empty() {
            return self.clone();
        }
        self.map(&mut |t| match t {
            Ty::Param(i, n) => args.get(i as usize).cloned().unwrap_or(Ty::Param(i, n)),
            t => t,
        })
    }
}

/// A reference to a trait with its generic arguments (excluding `Self`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TraitRef {
    pub trait_id: DefId,
    pub args: Vec<Ty>,
}

impl TraitRef {
    pub fn subst(&self, args: &[Ty]) -> TraitRef {
        TraitRef { trait_id: self.trait_id, args: self.args.iter().map(|t| t.subst(args)).collect() }
    }
}

/// Pretty-printing needs item names, so it goes through this helper.
pub struct TyDisplay<'a> {
    pub ty: &'a Ty,
    pub names: &'a dyn Fn(DefId) -> String,
    /// How to show `impl Trait` (its bounds).
    pub opaque: &'a dyn Fn(DefId, u32, &[Ty]) -> String,
}

impl fmt::Display for TyDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let d = |t: &Ty| TyDisplay { ty: t, names: self.names, opaque: self.opaque }.to_string();
        let list = |ts: &[Ty]| ts.iter().map(d).collect::<Vec<_>>().join(", ");
        match self.ty {
            Ty::Int(i) => write!(f, "{}", i.name()),
            Ty::Float(x) => write!(f, "{}", x.name()),
            Ty::Bool => write!(f, "bool"),
            Ty::Char => write!(f, "char"),
            Ty::Str => write!(f, "str"),
            Ty::String => write!(f, "String"),
            Ty::FmtArgs => write!(f, "FmtArgs"),
            Ty::Tuple(ts) if ts.len() == 1 => write!(f, "({},)", d(&ts[0])),
            Ty::Tuple(ts) => write!(f, "({})", list(ts)),
            Ty::Adt(id, args) if args.is_empty() => write!(f, "{}", (self.names)(*id)),
            Ty::Adt(id, args) => write!(f, "{}::<{}>", (self.names)(*id), list(args)),
            Ty::Array(t, k) => match k {
                ArrKind::Fixed(n) => write!(f, "[{}; {n}]", d(t)),
                ArrKind::FixedVar(_) => write!(f, "[{}; _]", d(t)),
                ArrKind::Dyn => write!(f, "[{}]", d(t)),
                ArrKind::Grow => write!(f, "[{}; ..+]", d(t)),
                ArrKind::Shrink => write!(f, "[{}; -..+]", d(t)),
                ArrKind::Slice => write!(f, "[{}; &]", d(t)),
                ArrKind::SliceMut => write!(f, "[{}; &mut]", d(t)),
            },
            Ty::Bang(t) => write!(f, "!{}", d(t)),
            Ty::Ptr(t, m) => write!(f, "*{}{}", if *m { "mut " } else { "" }, d(t)),
            Ty::FnPtr(ps, r) => {
                let params: Vec<String> = ps
                    .iter()
                    .map(|(m, t)| {
                        let prefix = match m {
                            ParamMode::Take => "",
                            ParamMode::Borrow => "&",
                            ParamMode::InOut => "&mut ",
                        };
                        format!("{prefix}{}", d(t))
                    })
                    .collect();
                write!(f, "fn({})", params.join(", "))?;
                if !r.is_unit() {
                    write!(f, ": {}", d(r))?;
                }
                Ok(())
            }
            Ty::Closure(..) => write!(f, "{{closure}}"),
            Ty::Opaque(def, i, args) => write!(f, "{}", (self.opaque)(*def, *i, args)),
            Ty::Param(_, n) => write!(f, "{n}"),
            Ty::Infer(_) => write!(f, "_"),
            Ty::Never => write!(f, "!"),
            Ty::Error => write!(f, "{{error}}"),
        }
    }
}
