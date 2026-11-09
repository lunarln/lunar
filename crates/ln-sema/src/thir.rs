//! Typed intermediate representation produced by type checking.
//!
//! Places (locals, fields, indexes, derefs) are explicit and every use of a
//! place as a value goes through [`ExprKind::Read`], which records whether
//! the read copies, moves, or merely borrows (passes a handle to a callee).

use crate::defs::{ConstVal, DefId};
use crate::ty::{TraitRef, Ty};
use ln_ast::{BinOp, UnOp};
use ln_diag::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LocalId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LoopId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalKind {
    /// Function parameter number `index`.
    /// `owned`: the function took ownership (`x: T`), so the parameter is
    /// a local it owns, not a borrow of the caller's value.
    Param { index: usize, inout: bool, bang: bool, owned: bool },
    /// `let` binding or pattern binding.
    Let,
    /// Compiler temporary.
    Temp,
}

#[derive(Clone, Debug)]
pub struct Local {
    pub name: String,
    pub ty: Ty,
    pub mutable: bool,
    pub kind: LocalKind,
    pub span: Span,
    /// Lexical block depth (params are 0, the body block is 1).
    pub depth: u32,
}

#[derive(Clone, Debug)]
pub struct Body {
    pub def: DefId,
    pub locals: Vec<Local>,
    pub params: Vec<LocalId>,
    pub block: Block,
    pub span: Span,
}

impl Body {
    pub fn local(&self, id: LocalId) -> &Local {
        &self.locals[id.0 as usize]
    }
}

#[derive(Clone, Debug)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub tail: Option<Box<Expr>>,
    /// Locals introduced directly in this block, in declaration order
    /// (dropped in reverse at block exit).
    pub locals: Vec<LocalId>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Let { pat: Pat, init: Option<Expr>, span: Span },
    Expr(Expr),
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub ty: Ty,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadMode {
    /// Bitwise copy of a Dull (or otherwise copyable) value.
    Copy,
    /// Ownership transfer; the source becomes invalid.
    Move,
    /// Pass an owner's handle to a callee without transferring ownership.
    Borrow,
}

#[derive(Clone, Debug)]
pub enum Callee {
    /// A known function instantiated with `args` (all generic params, parent first).
    Fn { def: DefId, args: Vec<Ty> },
    /// A trait method resolved at monomorphization time from `self_ty`.
    Trait { trait_ref: TraitRef, self_ty: Ty, method: String, method_args: Vec<Ty> },
    /// A compiler intrinsic (`def` is its `#[intrinsic]` declaration).
    Intrinsic { def: DefId, name: String, args: Vec<Ty> },
    /// Call through a function pointer.
    Indirect(Box<Expr>),
}

#[derive(Clone, Debug)]
pub enum Arg {
    /// By-value (or by-handle for owners) argument.
    Value(Expr),
    /// `mut` parameter: a mutable place passed by reference.
    InOut(Expr),
}

impl Arg {
    pub fn expr(&self) -> &Expr {
        match self {
            Arg::Value(e) | Arg::InOut(e) => e,
        }
    }
}

#[derive(Clone, Debug)]
pub enum FmtPiece {
    Text(String),
    Arg(Expr),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coercion {
    /// `!` to any type.
    NeverToAny,
    /// Whole array place (fixed or owner) viewed as a slice.
    ToSlice { mutable: bool },
    /// `[T; &mut]` → `[T; &]`, `*mut T` → `*T` (no-op at runtime).
    Weaken,
    /// `String` → `str` view.
    StringToStr,
    /// Function item to function pointer.
    FnToPtr,
}

#[derive(Clone, Debug)]
pub struct Arm {
    pub pat: Pat,
    pub guard: Option<Expr>,
    pub body: Expr,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    // ----- places -----
    Local(LocalId),
    Static(DefId),
    /// Field of a struct, tuple, or (inside a matched pattern) enum variant.
    Field {
        base: Box<Expr>,
        variant: usize,
        field: usize,
    },
    /// Bounds-checked element of an array, slice or owner.
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
    },
    /// `*p` on a raw pointer.
    Deref(Box<Expr>),
    /// A value materialized into a temporary so it can be used as a place.
    Temp(Box<Expr>),

    // ----- values -----
    Read {
        place: Box<Expr>,
        mode: ReadMode,
    },
    Lit(ConstVal),
    FnRef {
        def: DefId,
        args: Vec<Ty>,
    },
    Call {
        callee: Callee,
        args: Vec<Arg>,
    },
    Unary(UnOp, Box<Expr>),
    /// Primitive arithmetic/comparison; operands have the same primitive type.
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// Short-circuiting `&&` (true) / `||` (false).
    Logical {
        and: bool,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Cast(Box<Expr>),
    Assign {
        place: Box<Expr>,
        value: Box<Expr>,
    },
    /// Primitive compound assignment.
    AssignOp {
        op: BinOp,
        place: Box<Expr>,
        value: Box<Expr>,
    },
    Tuple(Vec<Expr>),
    Adt {
        def: DefId,
        args: Vec<Ty>,
        variant: usize,
        fields: Vec<(usize, Expr)>,
    },
    /// Array literal; the array kind comes from `ty`.
    Array(Vec<Expr>),
    /// `[value; count]`; `count` is a usize expression.
    ArrayRepeat {
        value: Box<Expr>,
        count: Box<Expr>,
    },
    /// `base[start..end]` producing a slice / str view.
    Slice {
        base: Box<Expr>,
        start: Option<Box<Expr>>,
        end: Option<Box<Expr>>,
        inclusive: bool,
        mutable: bool,
    },
    Block(Block),
    If {
        cond: Box<Expr>,
        then: Box<Expr>,
        else_: Option<Box<Expr>>,
    },
    Match {
        scrut: Box<Expr>,
        arms: Vec<Arm>,
    },
    Loop {
        body: Block,
        id: LoopId,
    },
    While {
        cond: Box<Expr>,
        body: Block,
        id: LoopId,
    },
    /// `for var in start..end` over integers.
    ForRange {
        var: LocalId,
        start: Box<Expr>,
        end: Box<Expr>,
        inclusive: bool,
        body: Block,
        id: LoopId,
    },
    /// `for pat in array` (by-value elements of a Dull array place).
    ForArray {
        pat: Pat,
        array: Box<Expr>,
        index: LocalId,
        body: Block,
        id: LoopId,
    },
    Break {
        id: LoopId,
        value: Option<Box<Expr>>,
    },
    Continue {
        id: LoopId,
    },
    Return(Option<Box<Expr>>),
    Coerce {
        kind: Coercion,
        expr: Box<Expr>,
    },
    /// A string literal converted to `FmtArgs`.
    FmtArgs(Vec<FmtPiece>),
    /// Placeholder for an expression that failed to type-check.
    Error,
}

impl Expr {
    pub fn is_place(&self) -> bool {
        matches!(self.kind, ExprKind::Local(_) | ExprKind::Static(_) | ExprKind::Field { .. } | ExprKind::Index { .. } | ExprKind::Deref(_) | ExprKind::Temp(_))
    }
}

#[derive(Clone, Debug)]
pub struct Pat {
    pub kind: PatKind,
    pub ty: Ty,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum PatKind {
    Wild,
    Binding {
        local: LocalId,
        mode: ReadMode,
        sub: Option<Box<Pat>>,
    },
    Lit(ConstVal),
    Range {
        lo: ConstVal,
        hi: ConstVal,
        inclusive: bool,
    },
    Tuple(Vec<Pat>),
    /// Struct or enum variant with field sub-patterns.
    Adt {
        def: DefId,
        variant: usize,
        fields: Vec<(usize, Pat)>,
    },
    Or(Vec<Pat>),
}

impl Pat {
    /// Whether the pattern matches every value of its type.
    pub fn is_irrefutable(&self, is_enum: &dyn Fn(DefId) -> bool) -> bool {
        match &self.kind {
            PatKind::Wild => true,
            PatKind::Binding { sub, .. } => sub.as_ref().is_none_or(|p| p.is_irrefutable(is_enum)),
            PatKind::Lit(_) | PatKind::Range { .. } => false,
            PatKind::Tuple(ps) => ps.iter().all(|p| p.is_irrefutable(is_enum)),
            PatKind::Adt { def, fields, .. } => !is_enum(*def) && fields.iter().all(|(_, p)| p.is_irrefutable(is_enum)),
            PatKind::Or(ps) => ps.iter().any(|p| p.is_irrefutable(is_enum)),
        }
    }
}
