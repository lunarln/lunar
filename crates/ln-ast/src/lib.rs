//! Abstract syntax tree for Lunar source files.
//!
//! Every expression, pattern and type node carries a [`NodeId`] unique within
//! one compilation, which later passes use to key side tables.

pub use ln_diag::Span;

pub mod pretty;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u32);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

impl Ident {
    pub fn new(name: impl Into<String>, span: Span) -> Self {
        Self { name: name.into(), span }
    }
}

// ===================================================================== items

/// One parsed source file.
#[derive(Clone, Debug)]
pub struct File {
    pub items: Vec<Item>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Attr {
    pub meta: Meta,
    pub span: Span,
}

/// Attribute contents: `name`, `name = "value"`, or `name(meta, ...)`.
#[derive(Clone, Debug)]
pub enum Meta {
    Word(Ident),
    NameValue(Ident, String),
    List(Ident, Vec<Meta>),
}

impl Meta {
    pub fn name(&self) -> &Ident {
        match self {
            Meta::Word(i) | Meta::NameValue(i, _) | Meta::List(i, _) => i,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Item {
    pub attrs: Vec<Attr>,
    pub is_pub: bool,
    pub kind: ItemKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum ItemKind {
    Fn(FnDecl),
    Struct(StructDecl),
    Enum(EnumDecl),
    Trait(TraitDecl),
    Impl(ImplDecl),
    Use(UseTree),
    Mod(ModDecl),
    Const(ConstDecl),
    Static(StaticDecl),
    Extern(ExternBlock),
    TypeAlias(TypeAlias),
}

#[derive(Clone, Debug, Default)]
pub struct Generics {
    pub params: Vec<GenericParam>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct GenericParam {
    pub name: Ident,
    pub bounds: Vec<Path>,
}

#[derive(Clone, Debug)]
pub struct FnDecl {
    pub name: Ident,
    pub generics: Generics,
    pub params: Vec<Param>,
    pub ret: Option<Ty>,
    pub body: Option<Block>,
    pub is_unsafe: bool,
    /// C-style variadic (`...` as the last parameter); extern fns only.
    pub variadic: bool,
    /// Span of the signature (name through return type).
    pub sig_span: Span,
}

#[derive(Clone, Debug)]
pub struct Param {
    /// `mut x: T` / `mut self`: an owned parameter the function may change.
    pub is_mut: bool,
    pub mode: ParamMode,
    pub kind: ParamKind,
    /// `None` only for a bare `self` (in any mode).
    pub ty: Option<Ty>,
    /// `&mut !self`: the receiver may be shrunk.
    pub self_bang: bool,
    pub span: Span,
}

/// How a parameter is passed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ParamMode {
    /// `x: T`, `self`: the function takes ownership (the caller's value is
    /// moved, or copied if it is Dull).
    Take,
    /// `x: &T`, `&self`: borrowed; the caller keeps ownership.
    Borrow,
    /// `x: &mut T`, `&mut self`: in-out; the caller passes a mutable place
    /// and sees the changes.
    InOut,
}

#[derive(Clone, Debug)]
pub enum ParamKind {
    SelfParam,
    Named(Ident),
    /// `_: T`
    Wild,
}

#[derive(Clone, Debug)]
pub struct StructDecl {
    pub name: Ident,
    pub generics: Generics,
    pub kind: VariantData,
}

#[derive(Clone, Debug)]
pub enum VariantData {
    Unit,
    Tuple(Vec<TupleField>),
    Named(Vec<FieldDecl>),
}

#[derive(Clone, Debug)]
pub struct TupleField {
    pub is_pub: bool,
    pub ty: Ty,
}

#[derive(Clone, Debug)]
pub struct FieldDecl {
    pub is_pub: bool,
    pub name: Ident,
    pub ty: Ty,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct EnumDecl {
    pub name: Ident,
    pub generics: Generics,
    pub variants: Vec<VariantDecl>,
}

#[derive(Clone, Debug)]
pub struct VariantDecl {
    pub name: Ident,
    pub data: VariantData,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct TraitDecl {
    pub name: Ident,
    pub generics: Generics,
    pub supertraits: Vec<Path>,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug)]
pub struct ImplDecl {
    pub generics: Generics,
    /// `impl !Trait for T`
    pub negative: bool,
    pub trait_ref: Option<Path>,
    pub self_ty: Ty,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug)]
pub struct UseTree {
    pub prefix: Vec<Ident>,
    pub kind: UseKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum UseKind {
    /// `use a::b;` or `use a::b as c;`
    Simple(Option<Ident>),
    /// `use a::*;`
    Glob,
    /// `use a::{b, c::d};`
    Nested(Vec<UseTree>),
}

#[derive(Clone, Debug)]
pub struct ModDecl {
    pub name: Ident,
    /// `None` for `mod foo;` (loaded from `foo.ln` / `foo/mod.ln`).
    pub items: Option<Vec<Item>>,
}

#[derive(Clone, Debug)]
pub struct ConstDecl {
    pub name: Ident,
    pub ty: Ty,
    pub value: Expr,
}

#[derive(Clone, Debug)]
pub struct StaticDecl {
    pub name: Ident,
    pub is_mut: bool,
    pub ty: Ty,
    pub value: Expr,
}

#[derive(Clone, Debug)]
pub struct ExternBlock {
    pub abi: Option<String>,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug)]
pub struct TypeAlias {
    pub name: Ident,
    pub generics: Generics,
    pub ty: Ty,
}

// ===================================================================== paths & types

#[derive(Clone, Debug)]
pub struct Path {
    pub segments: Vec<PathSegment>,
    pub span: Span,
    pub id: NodeId,
}

#[derive(Clone, Debug)]
pub struct PathSegment {
    /// May be `crate`, `self`, `super` or `Self`.
    pub ident: Ident,
    /// `::<T, U>` attached to this segment.
    pub args: Option<Vec<Ty>>,
}

impl Path {
    pub fn is_single(&self) -> bool {
        self.segments.len() == 1 && self.segments[0].args.is_none()
    }

    pub fn last(&self) -> &PathSegment {
        self.segments.last().unwrap()
    }

    pub fn to_string_lossy(&self) -> String {
        self.segments.iter().map(|s| s.ident.name.as_str()).collect::<Vec<_>>().join("::")
    }
}

#[derive(Clone, Debug)]
pub struct Ty {
    pub kind: TyKind,
    pub span: Span,
    pub id: NodeId,
}

#[derive(Clone, Debug)]
pub enum TyKind {
    Path(Path),
    /// `[T; size]`
    Array(Box<Ty>, ArraySize),
    /// `[T]`: dynamically sized at construction, then frozen.
    DynArray(Box<Ty>),
    /// `!T`: parameter-only marker that the argument may have been shrunk.
    Bang(Box<Ty>),
    /// `*T` / `*mut T`
    Ptr {
        is_mut: bool,
        inner: Box<Ty>,
    },
    /// `()` and `(A, B)`
    Tuple(Vec<Ty>),
    /// `fn(A, &B, &mut C): R`: parameter modes are part of the type.
    Fn {
        params: Vec<(ParamMode, Ty)>,
        ret: Option<Box<Ty>>,
    },
    /// `!` in return position.
    Never,
    /// `_` (only valid where inference is allowed).
    Infer,
}

#[derive(Clone, Debug)]
pub enum ArraySize {
    /// `[T; 4]` (any constant expression).
    Fixed(Box<Expr>),
    /// `[T; _]`
    Inferred,
    /// `[T; ..+]`: grow-only.
    Grow,
    /// `[T; -..+]`: grow and shrink.
    GrowShrink,
    /// `[T; &]`
    Slice,
    /// `[T; &mut]`
    SliceMut,
}

// ===================================================================== statements & expressions

#[derive(Clone, Debug)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    /// Final expression without a trailing `;`: the block's value.
    pub tail: Option<Box<Expr>>,
    pub span: Span,
    pub id: NodeId,
}

#[derive(Clone, Debug)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum StmtKind {
    Let {
        pat: Pat,
        ty: Option<Ty>,
        init: Option<Expr>,
    },
    /// Expression statement. Block-like expressions (`if`, `match`, loops,
    /// blocks) may omit the `;`.
    Expr(Expr),
    Item(Box<Item>),
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
    pub id: NodeId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

impl BinOp {
    pub fn as_str(self) -> &'static str {
        use BinOp::*;
        match self {
            Add => "+",
            Sub => "-",
            Mul => "*",
            Div => "/",
            Rem => "%",
            BitAnd => "&",
            BitOr => "|",
            BitXor => "^",
            Shl => "<<",
            Shr => ">>",
            Eq => "==",
            Ne => "!=",
            Lt => "<",
            Le => "<=",
            Gt => ">",
            Ge => ">=",
            And => "&&",
            Or => "||",
        }
    }

    pub fn is_comparison(self) -> bool {
        matches!(self, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnOp {
    Neg,
    Not,
    /// `*p` on a raw pointer.
    Deref,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Lit(Lit),
    Path(Path),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Assign(Box<Expr>, Box<Expr>),
    AssignOp(BinOp, Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    MethodCall {
        recv: Box<Expr>,
        name: Ident,
        generics: Option<Vec<Ty>>,
        args: Vec<Expr>,
    },
    Field(Box<Expr>, Ident),
    TupleField(Box<Expr>, u32, Span),
    Index(Box<Expr>, Box<Expr>),
    /// `a..b`, `a..`, `..b`, `..`, `a..=b`
    Range {
        start: Option<Box<Expr>>,
        end: Option<Box<Expr>>,
        inclusive: bool,
    },
    Array(Vec<Expr>),
    /// `[value; count]`
    ArrayRepeat(Box<Expr>, Box<Expr>),
    Tuple(Vec<Expr>),
    StructLit {
        path: Path,
        fields: Vec<FieldInit>,
    },
    Block(Block),
    Unsafe(Block),
    If {
        cond: Box<Expr>,
        then: Block,
        else_: Option<Box<Expr>>,
    },
    /// `if let pat = expr { } else { }`
    IfLet {
        pat: Pat,
        scrut: Box<Expr>,
        then: Block,
        else_: Option<Box<Expr>>,
    },
    Match {
        scrut: Box<Expr>,
        arms: Vec<Arm>,
    },
    While {
        cond: Box<Expr>,
        body: Block,
    },
    WhileLet {
        pat: Pat,
        scrut: Box<Expr>,
        body: Block,
    },
    Loop(Block),
    For {
        pat: Pat,
        iter: Box<Expr>,
        body: Block,
    },
    Break(Option<Box<Expr>>),
    Continue,
    Return(Option<Box<Expr>>),
    Cast(Box<Expr>, Ty),
    Paren(Box<Expr>),
    /// `expr?`
    Try(Box<Expr>),
    /// Placeholder produced after a parse error.
    Error,
}

impl ExprKind {
    /// Block-like expressions may stand as statements without a `;`.
    pub fn is_block_like(&self) -> bool {
        matches!(
            self,
            ExprKind::Block(_)
                | ExprKind::Unsafe(_)
                | ExprKind::If { .. }
                | ExprKind::IfLet { .. }
                | ExprKind::Match { .. }
                | ExprKind::While { .. }
                | ExprKind::WhileLet { .. }
                | ExprKind::Loop(_)
                | ExprKind::For { .. }
        )
    }
}

#[derive(Clone, Debug)]
pub struct FieldInit {
    pub name: Ident,
    /// `None` for shorthand `Point { x, y }`.
    pub value: Option<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Arm {
    pub pat: Pat,
    pub guard: Option<Expr>,
    pub body: Expr,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Lit {
    Int {
        value: u128,
        suffix: Option<String>,
    },
    Float {
        value: f64,
        suffix: Option<String>,
    },
    Bool(bool),
    Char(char),
    /// A string literal. `value` is the unescaped text. `fmt` holds the
    /// pre-parsed interpolation pieces, used only when the literal converts
    /// to `FmtArgs`.
    Str {
        value: String,
        fmt: FmtString,
    },
}

/// The interpretation of a string literal as a format string.
#[derive(Clone, Debug)]
pub enum FmtString {
    Ok(Vec<FmtPiece>),
    /// The literal is not a valid format string (message, span).
    Err(String, Span),
}

#[derive(Clone, Debug)]
pub enum FmtPiece {
    Text(String),
    Expr(Box<Expr>),
}

// ===================================================================== patterns

#[derive(Clone, Debug)]
pub struct Pat {
    pub kind: PatKind,
    pub span: Span,
    pub id: NodeId,
}

#[derive(Clone, Debug)]
pub enum PatKind {
    Wild,
    /// `x`, `mut x`. A single-segment path pattern that names a unit
    /// variant/struct or const is resolved later.
    Binding {
        is_mut: bool,
        name: Ident,
    },
    Lit(Lit, bool /* negated */),
    Range {
        lo: (Lit, bool),
        hi: (Lit, bool),
        inclusive: bool,
    },
    Tuple(Vec<Pat>),
    /// `Path(p, ..)`
    TupleStruct {
        path: Path,
        pats: Vec<Pat>,
    },
    /// `Path { f: p, g, .. }`
    Struct {
        path: Path,
        fields: Vec<FieldPat>,
        rest: bool,
    },
    /// Multi-segment path: unit variant / unit struct / const.
    Path(Path),
    Or(Vec<Pat>),
    /// `..` inside tuple patterns.
    Rest,
}

#[derive(Clone, Debug)]
pub struct FieldPat {
    pub name: Ident,
    pub pat: Pat,
}
