//! Editor queries over an [`Analysis`]: hover, go-to-definition, and
//! positions. Protocol-independent so it can be tested directly.

use crate::Analysis;
use ln_diag::{FileId, SourceMap, Span};
use ln_sema::defs::{DefData, DefId, DefKind};
use ln_sema::thir::{self, Block, Expr, ExprKind, FmtPiece, Pat, PatKind, Stmt};
use ln_sema::ty::Ty;
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
                let data = p.adt(adt);
                let v = &data.variants[variant];
                let f = &v.fields[field];
                let owner = if data.is_enum { p.path_of(v.def) } else { p.path_of(adt) };
                let env = &data.generics.preds;
                let mut facts = Vec::new();
                if !data.is_enum
                    && let Some(offsets) = p.field_offsets(adt, &[])
                {
                    facts.push(format!("offset {}", offsets[field]));
                }
                facts.extend(type_facts(p, &f.ty, env));
                let vis = if f.is_pub { "pub " } else { "" };
                format!("```lunar\n{owner}\n{vis}{}: {}\n```{}", f.name, p.ty_str(&f.ty), facts_block(&facts))
            }
            RefTarget::Local { body, local } => {
                let l = p.bodies.get(&body)?.local(local);
                let env = match &p.def(body).data {
                    DefData::Fn(fd) => fd.generics.preds.clone(),
                    _ => Vec::new(),
                };
                let mut facts = Vec::new();
                let kw = match l.kind {
                    thir::LocalKind::Param { inout, bang, .. } => {
                        facts.push(param_role(p, &l.ty, inout, bang, &env));
                        ""
                    }
                    _ => "let ",
                };
                facts.extend(type_facts(p, &l.ty, &env));
                let m = if l.mutable { "mut " } else { "" };
                format!("```lunar\n{kw}{m}{}: {}\n```{}", l.name, p.ty_str(&l.ty), facts_block(&facts))
            }
        };
        return Some((span, text));
    }
    // Keywords explain themselves (rather than showing, say, `!` for `return`).
    if let Some((span, text)) = keyword_hover(&a.map, file, offset) {
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
    // Plain-text facts: attributes, parameter roles, layout.
    let mut facts = Vec::new();
    let attrs = if info.span.is_dummy() { Vec::new() } else { attributes(map, info.span) };
    if !attrs.is_empty() {
        facts.push(format!("attributes: {}", attrs.join(" ")));
    }
    match &info.data {
        DefData::Fn(fd) => {
            let env = &fd.generics.preds;
            if fd.is_unsafe {
                facts.push("unsafe to call: callers need an `unsafe` block".into());
            }
            for prm in &fd.sig.params {
                let bang = matches!(prm.ty, Ty::Bang(_));
                facts.push(format!("`{}`: {}", prm.name, param_role(p, &prm.ty, prm.inout, bang, env)));
            }
        }
        DefData::Adt(adt) => {
            let args: Vec<Ty> = adt.generics.names.iter().enumerate().map(|(i, n)| Ty::Param(i as u32, n.clone())).collect();
            let ty = Ty::Adt(d, args);
            facts.extend(type_facts(p, &ty, &adt.generics.preds));
            if p.layout_of(&ty).is_none() && !adt.generics.names.is_empty() {
                facts.push("size depends on its type arguments".into());
            }
        }
        _ => {}
    }
    out.push_str(&facts_block(&facts));
    let docs = if info.span.is_dummy() { String::new() } else { doc_comment(map, info.span) };
    if !docs.is_empty() {
        out.push_str("\n\n---\n\n");
        out.push_str(&docs);
    }
    out
}

/// How a function treats an argument.
fn param_role(p: &Program, ty: &Ty, inout: bool, bang: bool, env: &[ln_sema::defs::Pred]) -> String {
    let inner = match ty {
        Ty::Bang(t) => t.as_ref(),
        t => t,
    };
    match (inout, bang) {
        (true, true) => "in-out (`mut`), may be shrunk (`!`): only the owner can pass it, with no live views into it".into(),
        (true, false) => "in-out (`mut`): the caller passes a mutable place and sees the changes".into(),
        (false, _) if p.has_owner(inner) => "borrowed: the function cannot move out of it or keep it".into(),
        (false, _) if p.is_copy(inner, env) => "copied".into(),
        _ => "borrowed".into(),
    }
}

/// Layout and value semantics of a type, as short plain-text facts.
fn type_facts(p: &Program, ty: &Ty, env: &[ln_sema::defs::Pred]) -> Vec<String> {
    let ty = match ty {
        Ty::Bang(t) => t.as_ref(),
        t => t,
    };
    let mut out = Vec::new();
    if let Some(l) = p.layout_of(ty) {
        out.push(format!("size {} bytes, align {}", l.size, l.align));
    }
    out.push(if p.is_dull(ty, env) { "Dull".into() } else { "pointy (not Dull)".into() });
    if p.has_owner(ty) {
        out.push("owning: moved, not copied".into());
    } else if p.is_copy(ty, env) {
        out.push("copied".into());
    }
    out
}

/// Plain-text facts below the code block, one per line.
fn facts_block(facts: &[String]) -> String {
    if facts.is_empty() {
        return String::new();
    }
    // Two trailing spaces make a markdown line break.
    format!("\n\n{}", facts.join("  \n"))
}

/// Attribute lines (`#[...]`) directly above the item declared at `name`.
fn attributes(map: &SourceMap, name: Span) -> Vec<String> {
    let text = &map.get(name.file).text;
    let line_start = text[..name.start as usize].rfind('\n').map_or(0, |i| i + 1);
    let mut out = Vec::new();
    for line in text[..line_start].lines().rev() {
        let t = line.trim();
        if t.starts_with("#[") {
            out.push(t.to_string());
        } else if !t.starts_with("///") {
            break;
        }
    }
    out.reverse();
    out
}

/// Documentation for the keyword under the cursor, if any.
fn keyword_hover(map: &SourceMap, file: FileId, offset: u32) -> Option<(Span, String)> {
    let text = &map.get(file).text;
    let (toks, _) = ln_lexer::lex(text);
    let tok = toks.iter().find(|t| t.range.start as u32 <= offset && offset < t.range.end as u32)?;
    let word = &text[tok.range.clone()];
    let doc = keyword_doc(word)?;
    Some((Span::new(file, tok.range.start as u32, tok.range.end as u32), format!("```lunar\n{word}\n```\n\n{doc}")))
}

fn keyword_doc(word: &str) -> Option<&'static str> {
    Some(match word {
        "fn" => {
            "Declares a function. Parameters are borrowed unless declared `mut` (in-out); the return type follows `:`. Generics use `::<>`: `fn max::<T: Ord>(a: T, b: T): T`."
        }
        "let" => "Introduces a binding. `let` bindings are immutable; `let mut` makes them mutable. The type can usually be inferred: `let x: i32 = 1;`.",
        "mut" => {
            "On a `let`, makes the binding mutable. On a parameter, makes it **in-out**: the caller passes a mutable place and sees the changes. A `!` parameter must also be `mut`."
        }
        "return" => "Leaves the current function with a value (or `()`). The `return` expression itself has type `!`, so it fits anywhere a value is expected.",
        "if" => "Conditional expression: `if cond { a } else { b }` has a value when both branches do.",
        "else" => "The alternative branch of an `if`.",
        "match" => "Pattern matching. Arms are checked in order and must cover every possible value (`err(type::non_exhaustive)` otherwise).",
        "for" => "Loops over a range (`for i in 0..n`) or the elements of a Dull array or slice (`for x in arr`).",
        "in" => "Separates the pattern from what a `for` loop iterates over.",
        "while" => "Loops while a condition holds.",
        "loop" => "Loops forever; leave with `break`, which can carry a value: `let x = loop { break 5; };`.",
        "break" => "Leaves the innermost loop, optionally with a value for `loop`. Has type `!`.",
        "continue" => "Skips to the next iteration of the innermost loop. Has type `!`.",
        "struct" => {
            "Declares a struct: named fields `struct P { x: i32 }`, a tuple struct `struct P(i32);` or a unit struct `struct P;`. Fields are private to the module unless `pub`."
        }
        "enum" => "Declares an enum: a value is exactly one of its variants. Variants can carry fields, and their fields are always public.",
        "impl" => "Adds methods to a type (`impl T { ... }`) or implements a trait (`impl Tr for T { ... }`). Two impls of a trait may not overlap.",
        "trait" => "Declares a trait: methods (optionally with default bodies) that types can implement. Supertraits: `trait A: B`.",
        "type" => "Declares a type alias: `type Bytes = [u8; -..+];`.",
        "mod" => "Declares a module, inline (`mod m { ... }`) or from a file (`mod m;` loads `m.ln` or `m/mod.ln`).",
        "use" => "Imports names from another module: `use std::io::println;`, `use core::ops::{Add, Sub};`.",
        "pub" => "Makes an item, field or method visible outside its module. Without it, items are private to their module and its children.",
        "const" => "A compile-time constant: `const N: usize = 16;`. Its value is evaluated during compilation.",
        "static" => "A global variable with a fixed address. `static mut` is mutable; reading or writing it requires `unsafe`.",
        "unsafe" => {
            "An `unsafe { }` block allows unsafe operations: calling `unsafe` or `extern` functions, dereferencing raw pointers, and using `static mut`. On a function, makes it unsafe to *call*; its body still needs `unsafe` blocks (hard error otherwise)."
        }
        "extern" => "Declares functions implemented elsewhere with the C ABI: `extern \"C\" { fn puts(s: *u8): i32; }`. Calling them requires `unsafe`.",
        "as" => "Numeric and pointer casts (`x as u64`), and renaming imports (`use a::b as c;`).",
        "self" => "The receiver of a method (`fn len(self)`, `fn push(mut self, ..)`, `fn pop(mut self: !Self)`), or the current module in paths (`self::x`).",
        "Self" => "The type an `impl` or trait is for.",
        "crate" => "The root module of the current crate: `crate::util::f`.",
        "super" => "The parent module: `super::helper()`.",
        "true" | "false" => "A `bool` value.",
        _ => return None,
    })
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
