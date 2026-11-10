//! Definition tables: every item in every crate gets a [`DefId`].

use crate::ty::{TraitRef, Ty};
pub use ln_ast::ParamMode;
use ln_diag::Span;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DefId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefKind {
    Mod,
    Fn,
    Struct,
    Enum,
    Variant,
    Trait,
    Impl,
    Const,
    Static,
    TypeAlias,
    Macro,
}

impl DefKind {
    pub fn describe(self) -> &'static str {
        match self {
            DefKind::Macro => "macro",
            DefKind::Mod => "module",
            DefKind::Fn => "function",
            DefKind::Struct => "struct",
            DefKind::Enum => "enum",
            DefKind::Variant => "enum variant",
            DefKind::Trait => "trait",
            DefKind::Impl => "impl",
            DefKind::Const => "constant",
            DefKind::Static => "static",
            DefKind::TypeAlias => "type alias",
        }
    }
}

/// Recognized item attributes.
#[derive(Clone, Debug, Default)]
pub struct ItemAttrs {
    pub lang: Option<String>,
    pub intrinsic: bool,
    pub test: bool,
    pub inline: bool,
    pub link_name: Option<String>,
    /// `#[derive(A, B)]`: derive macros to run on this struct or enum.
    pub derive: Vec<ln_ast::Ident>,
    /// `#[derive_macro]`: this macro is a derive.
    pub derive_macro: bool,
    /// `#[export]`: an item a macro defines is visible where it is invoked.
    pub export: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ns {
    Type,
    Value,
    /// Macros (`vec!`, derives); a trait and its derive share a path.
    Macro,
}

/// A name bound in a module (an item or an import).
#[derive(Clone, Copy, Debug)]
pub struct Binding {
    pub def: DefId,
    pub is_pub: bool,
    pub span: Span,
}

#[derive(Clone, Debug, Default)]
pub struct ModData {
    pub types: HashMap<String, Binding>,
    pub values: HashMap<String, Binding>,
    pub macros: HashMap<String, Binding>,
    /// `use path::*;` targets (module or enum), with visibility.
    pub globs: Vec<(DefId, bool)>,
    /// Parent module (`super`); `None` for crate roots.
    pub parent: Option<DefId>,
    /// The crate root this module belongs to.
    pub krate: DefId,
    /// Anonymous block scope inside a function body.
    pub is_block: bool,
}

impl ModData {
    pub fn ns(&self, ns: Ns) -> &HashMap<String, Binding> {
        match ns {
            Ns::Type => &self.types,
            Ns::Value => &self.values,
            Ns::Macro => &self.macros,
        }
    }

    pub fn ns_mut(&mut self, ns: Ns) -> &mut HashMap<String, Binding> {
        match ns {
            Ns::Type => &mut self.types,
            Ns::Value => &mut self.values,
            Ns::Macro => &mut self.macros,
        }
    }
}

/// A trait bound on a type: `ty: trait_ref`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Pred {
    pub ty: Ty,
    pub trait_ref: TraitRef,
    pub span: Span,
}

#[derive(Clone, Debug, Default)]
pub struct Generics {
    /// Number of parameters inherited from the parent (impl or trait).
    pub parent_count: u32,
    /// Names of this item's own parameters.
    pub names: Vec<String>,
    /// Bounds on own parameters (parents' bounds live in the parent).
    pub preds: Vec<Pred>,
    pub span: Span,
}

impl Generics {
    pub fn count(&self) -> usize {
        self.parent_count as usize + self.names.len()
    }

    pub fn own_count(&self) -> usize {
        self.names.len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Abi {
    Lunar,
    C,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    /// The name's hygiene context (it may come from a macro's body).
    pub ctx: ln_ast::SyntaxCtx,
    pub ty: Ty,
    pub mode: ParamMode,
    /// `x: &mut T`: passed by reference (in-out). Same as `mode == InOut`.
    pub inout: bool,
    /// `mut x: T`: an owned parameter the function may change.
    pub mutable: bool,
    pub is_self: bool,
    /// `kept x: T`: views from this argument may be stored into the
    /// function's in-out arguments.
    pub kept: bool,
    pub span: Span,
}

/// Which parameters a function's result may point into, from its
/// signature: bit `i` of `set` for parameter `i` (parameters from the 64th
/// on always count). `span` covers the names of the `from` clause, and is
/// dummy when there is none and the set follows from the parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetFrom {
    pub set: u64,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct FnSig {
    pub params: Vec<Param>,
    pub ret: Ty,
    pub ret_span: Span,
}

#[derive(Clone, Debug)]
pub struct FnData {
    pub generics: Generics,
    pub sig: FnSig,
    pub is_unsafe: bool,
    pub abi: Abi,
    pub variadic: bool,
    pub has_body: bool,
    /// Enclosing impl or trait, for methods.
    pub owner: Option<DefId>,
    pub has_self: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CtorKind {
    Unit,
    Tuple,
    Named,
}

#[derive(Clone, Debug)]
pub struct FieldDef {
    pub name: String,
    pub ty: Ty,
    pub is_pub: bool,
    /// The field's name (its type, for tuple fields).
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct VariantDef {
    pub name: String,
    /// The variant's own DefId (enums) or the struct's DefId.
    pub def: DefId,
    pub ctor: CtorKind,
    pub fields: Vec<FieldDef>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct AdtData {
    pub generics: Generics,
    pub variants: Vec<VariantDef>,
    pub is_enum: bool,
}

impl AdtData {
    pub fn struct_fields(&self) -> &[FieldDef] {
        &self.variants[0].fields
    }

    pub fn variant_index(&self, name: &str) -> Option<usize> {
        self.variants.iter().position(|v| v.name == name)
    }
}

#[derive(Clone, Debug)]
pub struct TraitMethod {
    pub name: String,
    pub def: DefId,
    pub has_default: bool,
}

#[derive(Clone, Debug)]
pub struct TraitData {
    /// Param 0 is `Self`; the trait's own params follow.
    pub generics: Generics,
    pub supertraits: Vec<TraitRef>,
    pub methods: Vec<TraitMethod>,
    /// Auto traits (only `Dull`) are implemented by the compiler.
    pub auto: bool,
}

#[derive(Clone, Debug)]
pub struct ImplData {
    pub generics: Generics,
    pub trait_ref: Option<TraitRef>,
    pub self_ty: Ty,
    pub methods: HashMap<String, DefId>,
    pub negative: bool,
}

/// A compile-time constant value.
#[derive(Clone, Debug, PartialEq)]
pub enum ConstVal {
    Int(i128),
    Float(f64),
    Bool(bool),
    Char(char),
    Str(String),
    Unit,
    Tuple(Vec<ConstVal>),
    /// Struct / enum variant: (variant index, fields).
    Adt(usize, Vec<ConstVal>),
    Array(Vec<ConstVal>),
}

#[derive(Clone, Debug)]
pub struct ConstData {
    pub ty: Ty,
    pub value: Option<ConstVal>,
}

#[derive(Clone, Debug)]
pub struct StaticData {
    pub ty: Ty,
    pub mutable: bool,
    pub value: Option<ConstVal>,
}

#[derive(Clone, Debug)]
pub struct AliasData {
    pub generics: Generics,
    pub ty: Option<Ty>,
}

#[derive(Clone, Debug)]
pub enum DefData {
    None,
    Mod(ModData),
    Fn(Box<FnData>),
    Adt(Box<AdtData>),
    Variant { enum_id: DefId, index: usize },
    Trait(Box<TraitData>),
    Impl(Box<ImplData>),
    Const(ConstData),
    Static(StaticData),
    Alias(AliasData),
}

#[derive(Clone, Debug)]
pub struct DefInfo {
    pub name: String,
    pub kind: DefKind,
    /// Enclosing module (for privacy and name lookup).
    pub module: DefId,
    /// Enclosing impl/trait/enum for associated items and variants.
    pub parent: Option<DefId>,
    pub is_pub: bool,
    pub span: Span,
    pub attrs: ItemAttrs,
    pub data: DefData,
}
