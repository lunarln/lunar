//! Sizes and alignments of types, mirroring the layouts code generation
//! uses (LLVM's default data layout on 64-bit targets). For editor hovers.
//!
//! * integers, floats, `char` (4 bytes), `bool` (1 byte): natural sizes;
//! * pointers and function pointers: 8 bytes;
//! * slices, `str`, `FmtArgs`: `{ ptr, len }`, 16 bytes;
//! * owning arrays and `String`: `{ ptr, len, cap }`, 24 bytes (the
//!   elements live on the array's data stack, not inline);
//! * `[T; N]`: N elements inline;
//! * tuples and structs: fields in order, each aligned, total rounded up to
//!   the largest alignment;
//! * enums: `{ i32 tag, [W x i64] }` where W covers the largest variant.

use crate::Program;
use crate::defs::DefData;
use crate::ty::{ArrKind, FloatTy, IntTy, Ty};

/// Size and alignment in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub size: u64,
    pub align: u64,
}

impl Layout {
    const fn new(size: u64, align: u64) -> Self {
        Layout { size, align }
    }
}

/// The C-style layout of fields in order, with each field's offset.
pub fn struct_layout(fields: &[Layout]) -> (Layout, Vec<u64>) {
    let mut offset = 0u64;
    let mut align = 1u64;
    let mut offsets = Vec::with_capacity(fields.len());
    for f in fields {
        offset = offset.next_multiple_of(f.align);
        offsets.push(offset);
        offset += f.size;
        align = align.max(f.align);
    }
    (Layout::new(offset.next_multiple_of(align), align), offsets)
}

impl Program {
    /// Layout of `t`, or `None` if it depends on generic parameters or
    /// unresolved inference.
    pub fn layout_of(&self, t: &Ty) -> Option<Layout> {
        self.layout_depth(t, 0)
    }

    fn layout_depth(&self, t: &Ty, depth: u32) -> Option<Layout> {
        if depth > 64 {
            return None;
        }
        let rec = |x: &Ty| self.layout_depth(x, depth + 1);
        Some(match t {
            Ty::Int(i) => {
                let n = match i {
                    IntTy::I8 | IntTy::U8 => 1,
                    IntTy::I16 | IntTy::U16 => 2,
                    IntTy::I32 | IntTy::U32 => 4,
                    IntTy::I64 | IntTy::U64 | IntTy::Isize | IntTy::Usize => 8,
                };
                Layout::new(n, n)
            }
            Ty::Float(FloatTy::F32) => Layout::new(4, 4),
            Ty::Float(FloatTy::F64) => Layout::new(8, 8),
            Ty::Bool => Layout::new(1, 1),
            Ty::Char => Layout::new(4, 4),
            Ty::Str | Ty::FmtArgs => Layout::new(16, 8),
            Ty::String => Layout::new(24, 8),
            Ty::Ptr(..) | Ty::FnPtr(..) => Layout::new(8, 8),
            Ty::Bang(inner) => return rec(inner),
            Ty::Never => Layout::new(0, 1),
            Ty::Tuple(ts) => {
                let fs: Option<Vec<Layout>> = ts.iter().map(rec).collect();
                struct_layout(&fs?).0
            }
            Ty::Array(e, k) => match k {
                ArrKind::Fixed(n) => {
                    let el = rec(e)?;
                    Layout::new(el.size * n, el.align)
                }
                ArrKind::FixedVar(_) => return None,
                ArrKind::Slice | ArrKind::SliceMut => Layout::new(16, 8),
                ArrKind::Dyn | ArrKind::Grow | ArrKind::Shrink => Layout::new(24, 8),
            },
            Ty::Adt(d, args) => {
                let DefData::Adt(adt) = &self.def(*d).data else { return None };
                let variant = |v: usize| -> Option<Layout> {
                    let fs: Option<Vec<Layout>> = adt.variants[v].fields.iter().map(|f| rec(&f.ty.subst(args))).collect();
                    Some(struct_layout(&fs?).0)
                };
                if adt.is_enum {
                    let mut max = 0;
                    for v in 0..adt.variants.len() {
                        max = max.max(variant(v)?.size);
                    }
                    // Even an empty `[0 x i64]` payload aligns the enum to 8.
                    Layout::new(8 + 8 * max.div_ceil(8), 8)
                } else {
                    variant(0)?
                }
            }
            Ty::Closure(def, args) => {
                let fs: Option<Vec<Layout>> = self.closure_fields(*def, args).iter().map(rec).collect();
                struct_layout(&fs?).0
            }
            Ty::Opaque(..) => {
                let r = self.reveal(t);
                if r.has_error() {
                    return None;
                }
                return rec(&r);
            }
            Ty::Param(..) | Ty::Infer(_) | Ty::Error => return None,
        })
    }

    /// Byte offsets of a struct's fields (non-generic structs only).
    pub fn field_offsets(&self, adt: crate::defs::DefId, args: &[Ty]) -> Option<Vec<u64>> {
        let DefData::Adt(a) = &self.def(adt).data else { return None };
        let fs: Option<Vec<Layout>> = a.variants[0].fields.iter().map(|f| self.layout_of(&f.ty.subst(args))).collect();
        Some(struct_layout(&fs?).1)
    }
}
