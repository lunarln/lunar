//! Editor queries over an [`Analysis`]: hover, go-to-definition, and
//! positions. Protocol-independent so it can be tested directly.

use crate::Analysis;
use ln_diag::{FileId, SourceMap, Span};
use ln_sema::defs::{DefData, DefId, DefKind};
use ln_sema::thir::{self, Block, Expr, ExprKind, FmtPiece, Pat, PatKind, Stmt};
use ln_sema::{Program, RefTarget};
use std::path::Path;

/// The file in `map` loaded from `path`, if any.
pub fn file_for_path(map: &SourceMap, path: &Path) -> Option<FileId> {
    let key = crate::path_key(path);
    map.files().find(|(_, f)| f.path.as_deref().is_some_and(|p| crate::path_key(p) == key)).map(|(id, _)| id)
}

/// Byte offset of a 0-based (line, UTF-16 column) position, clamped to the text.
pub fn offset_of(text: &str, line: u32, col_utf16: u32) -> u32 {
    let mut start = 0usize;
    for _ in 0..line {
        match text[start..].find('\n') {
            Some(i) => start += i + 1,
            None => return text.len() as u32,
        }
    }
    let mut units = 0u32;
    for (i, c) in text[start..].char_indices() {
        if units >= col_utf16 || c == '\n' {
            return (start + i) as u32;
        }
        units += c.len_utf16() as u32;
    }
    text.len() as u32
}

/// 0-based (line, UTF-16 column) of a byte offset.
pub fn position_of(text: &str, offset: u32) -> (u32, u32) {
    let offset = (offset as usize).min(text.len());
    let before = &text[..offset];
    let line = before.matches('\n').count() as u32;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let col = before[line_start..].chars().map(|c| c.len_utf16() as u32).sum();
    (line, col)
}

/// The innermost recorded name at `offset` in `file`: a use of a name, or
/// the name in an item or field declaration.
fn ref_at(p: &Program, file: FileId, offset: u32) -> Option<(Span, RefTarget)> {
    let mut best: Option<(Span, RefTarget)> = None;
    let mut consider = |s: Span, t: RefTarget| {
        if s.file == file && s.start <= offset && offset <= s.end && best.is_none_or(|(b, _)| s.len() <= b.len()) {
            best = Some((s, t));
        }
    };
    for r in &p.refs {
        consider(r.span, r.target);
    }
    for (i, d) in p.defs.iter().enumerate() {
        if d.span.file != file || matches!(d.kind, DefKind::Impl) || d.name.starts_with('<') {
            continue;
        }
        let id = DefId(i as u32);
        consider(d.span, RefTarget::Def(id));
        if let DefData::Adt(adt) = &d.data {
            for (vi, v) in adt.variants.iter().enumerate() {
                for (fi, f) in v.fields.iter().enumerate() {
                    // Field spans cover `name: Type`; only the name counts.
                    let name = Span::new(f.span.file, f.span.start, f.span.start + f.name.len() as u32);
                    consider(name, RefTarget::Field { adt: id, variant: vi, field: fi });
                }
            }
        }
    }
    best
}

/// Where the name at `offset` is defined.
pub fn definition(a: &Analysis, file: FileId, offset: u32) -> Option<Span> {
    let p = a.program.as_ref()?;
    let (_, target) = ref_at(p, file, offset)?;
    let span = match target {
        RefTarget::Def(d) => p.def(d).span,
        RefTarget::Field { adt, variant, field } => p.adt(adt).variants[variant].fields[field].span,
        RefTarget::Local { body, local } => p.bodies.get(&body)?.local(local).span,
    };
    (!span.is_dummy()).then_some(span)
}

/// Markdown hover text for `offset`, and the span it describes.
pub fn hover(a: &Analysis, file: FileId, offset: u32) -> Option<(Span, String)> {
    let p = a.program.as_ref()?;
    if let Some((span, target)) = ref_at(p, file, offset) {
        let text = match target {
            RefTarget::Def(d) => describe_def(p, &a.map, d),
            RefTarget::Field { adt, variant, field } => {
                let v = &p.adt(adt).variants[variant];
                let f = &v.fields[field];
                let owner = if p.adt(adt).is_enum { p.path_of(v.def) } else { p.path_of(adt) };
                format!("```lunar\n{owner}\n{}: {}\n```", f.name, p.ty_str(&f.ty))
            }
            RefTarget::Local { body, local } => {
                let l = p.bodies.get(&body)?.local(local);
                let kw = match l.kind {
                    thir::LocalKind::Param { .. } => "",
                    _ => "let ",
                };
                let m = if l.mutable { "mut " } else { "" };
                format!("```lunar\n{kw}{m}{}: {}\n```", l.name, p.ty_str(&l.ty))
            }
        };
        return Some((span, text));
    }
    // Otherwise the type of the innermost expression under the cursor.
    let mut best: Option<(Span, String)> = None;
    for body in p.bodies.values() {
        if body.span.file != file || offset < body.span.start || offset > body.span.end {
            continue;
        }
        walk_block(&body.block, &mut |e: &Expr| {
            let s = e.span;
            if s.file == file && s.start <= offset && offset <= s.end && !e.ty.is_error() && best.as_ref().is_none_or(|(b, _)| s.len() < b.len()) {
                best = Some((s, format!("```lunar\n{}\n```", p.ty_str(&e.ty))));
            }
        });
    }
    best
}

/// Signature (from source), container path and doc comments of a definition.
fn describe_def(p: &Program, map: &SourceMap, d: DefId) -> String {
    let info = p.def(d);
    let mut sig = if info.span.is_dummy() { String::new() } else { signature(map, info.span) };
    match (&info.data, info.kind) {
        (DefData::Adt(adt), DefKind::Struct) if adt.variants[0].fields.iter().any(|f| !f.name.chars().all(|c| c.is_ascii_digit())) => {
            sig.push_str(" {\n");
            for f in &adt.variants[0].fields {
                sig.push_str(&format!("    {}: {},\n", f.name, p.ty_str(&f.ty)));
            }
            sig.push('}');
        }
        (DefData::Adt(adt), DefKind::Enum) => {
            sig.push_str(" {\n");
            for v in &adt.variants {
                sig.push_str(&format!("    {},\n", signature(map, v.span).trim_end_matches(',')));
            }
            sig.push('}');
        }
        _ => {}
    }
    if info.kind == DefKind::Mod {
        sig = format!("mod {}", info.name);
    }
    let path = p.path_of(d);
    let container = path.rsplit_once("::").map(|(c, _)| c.to_string()).unwrap_or_default();
    let mut out = String::from("```lunar\n");
    if !container.is_empty() {
        out.push_str(&container);
        out.push('\n');
    }
    out.push_str(&sig);
    out.push_str("\n```");
    let docs = if info.span.is_dummy() { String::new() } else { doc_comment(map, info.span) };
    if !docs.is_empty() {
        out.push_str("\n\n---\n\n");
        out.push_str(&docs);
    }
    out
}

/// The declaration containing `name` (an identifier span), from the start
/// of its line to the first `{`, `;` or `=` (for fns, up to the body),
/// with whitespace collapsed.
fn signature(map: &SourceMap, name: Span) -> String {
    let text = &map.get(name.file).text;
    let start = text[..name.start as usize].rfind('\n').map_or(0, |i| i + 1);
    let mut depth = 0i32;
    let mut end = text.len();
    for (i, c) in text[start..].char_indices() {
        let at = start + i;
        match c {
            '(' | '[' | '<' => depth += 1,
            ')' | ']' => depth -= 1,
            '>' if !text[..at].ends_with('-') => depth -= 1,
            '{' | ';' if depth <= 0 && at >= name.end as usize => {
                end = at;
                break;
            }
            ',' if depth <= 0 && at >= name.end as usize => {
                // Enum variants end at their comma.
                end = at;
                break;
            }
            _ => {}
        }
    }
    let sig = text[start..end].split_whitespace().collect::<Vec<_>>().join(" ");
    sig.strip_prefix("pub ").unwrap_or(&sig).to_string()
}

/// `///` lines directly above the item declared at `name` (attributes skipped).
fn doc_comment(map: &SourceMap, name: Span) -> String {
    let text = &map.get(name.file).text;
    let line_start = text[..name.start as usize].rfind('\n').map_or(0, |i| i + 1);
    let mut lines = Vec::new();
    for line in text[..line_start].lines().rev() {
        let t = line.trim();
        if let Some(doc) = t.strip_prefix("///") {
            lines.push(doc.strip_prefix(' ').unwrap_or(doc).to_string());
        } else if t.starts_with("#[") {
            continue;
        } else {
            break;
        }
    }
    lines.reverse();
    lines.join("\n")
}

// ===================================================================== THIR walk

fn walk_block(b: &Block, f: &mut dyn FnMut(&Expr)) {
    for s in &b.stmts {
        match s {
            Stmt::Let { pat, init, .. } => {
                walk_pat(pat, f);
                if let Some(e) = init {
                    walk_expr(e, f);
                }
            }
            Stmt::Expr(e) => walk_expr(e, f),
        }
    }
    if let Some(t) = &b.tail {
        walk_expr(t, f);
    }
}

fn walk_pat(_p: &Pat, _f: &mut dyn FnMut(&Expr)) {
    // Bindings are covered by recorded references; nothing else to show.
    let _ = PatKind::Wild;
}

fn walk_expr(e: &Expr, f: &mut dyn FnMut(&Expr)) {
    f(e);
    use ExprKind as K;
    match &e.kind {
        K::Field { base, .. } | K::Deref(base) | K::Temp(base) | K::Read { place: base, .. } | K::Unary(_, base) | K::Cast(base) => walk_expr(base, f),
        K::Coerce { expr, .. } => walk_expr(expr, f),
        K::Index { base, index } => {
            walk_expr(base, f);
            walk_expr(index, f);
        }
        K::Call { callee, args } => {
            if let thir::Callee::Indirect(c) = callee {
                walk_expr(c, f);
            }
            for a in args {
                walk_expr(a.expr(), f);
            }
        }
        K::Binary(_, l, r) | K::Logical { lhs: l, rhs: r, .. } | K::Assign { place: l, value: r } | K::AssignOp { place: l, value: r, .. } => {
            walk_expr(l, f);
            walk_expr(r, f);
        }
        K::Tuple(es) | K::Array(es) => es.iter().for_each(|x| walk_expr(x, f)),
        K::Adt { fields, .. } => fields.iter().for_each(|(_, x)| walk_expr(x, f)),
        K::ArrayRepeat { value, count } => {
            walk_expr(value, f);
            walk_expr(count, f);
        }
        K::Slice { base, start, end, .. } => {
            walk_expr(base, f);
            start.iter().chain(end.iter()).for_each(|x| walk_expr(x, f));
        }
        K::Block(b) | K::Loop { body: b, .. } => walk_block(b, f),
        K::If { cond, then, else_ } => {
            walk_expr(cond, f);
            walk_expr(then, f);
            if let Some(x) = else_ {
                walk_expr(x, f);
            }
        }
        K::Match { scrut, arms } => {
            walk_expr(scrut, f);
            for a in arms {
                walk_pat(&a.pat, f);
                if let Some(g) = &a.guard {
                    walk_expr(g, f);
                }
                walk_expr(&a.body, f);
            }
        }
        K::While { cond, body, .. } => {
            walk_expr(cond, f);
            walk_block(body, f);
        }
        K::ForRange { start, end, body, .. } => {
            walk_expr(start, f);
            walk_expr(end, f);
            walk_block(body, f);
        }
        K::ForArray { pat, array, body, .. } => {
            walk_pat(pat, f);
            walk_expr(array, f);
            walk_block(body, f);
        }
        K::Break { value: Some(v), .. } | K::Return(Some(v)) => walk_expr(v, f),
        K::FmtArgs(pieces) => {
            for p in pieces {
                if let FmtPiece::Arg(x) = p {
                    walk_expr(x, f);
                }
            }
        }
        K::Local(_) | K::Static(_) | K::Lit(_) | K::FnRef { .. } | K::Break { .. } | K::Continue { .. } | K::Return(None) | K::Error => {}
    }
}
