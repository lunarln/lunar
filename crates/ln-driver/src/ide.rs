//! Editor queries over an [`Analysis`]: hover, go-to-definition, and
//! positions. Protocol-independent so it can be tested directly.

use crate::Analysis;
use ln_ast::{self as ast, pretty};
use ln_diag::{FileId, SourceMap, Span};
use ln_sema::defs::{DefData, DefId, DefKind};
use ln_sema::loader::ModSource;
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
        RefTarget::TyParam(decl) => decl,
    };
    (!span.is_dummy()).then_some(span)
}

/// A declaration in the parsed source, found by the span of its name.
#[derive(Clone, Copy)]
enum Decl<'a> {
    /// An item, the `impl` or `trait` it is declared in, and the ABI of its
    /// `extern` block.
    Item {
        item: &'a ast::Item,
        parent: Option<&'a ast::Item>,
        abi: Option<&'a str>,
    },
    Variant(&'a ast::VariantDecl),
    Param(&'a ast::GenericParam),
}

fn find_decl(a: &Analysis, name: Span) -> Option<Decl<'_>> {
    fn module(m: &ModSource, file: FileId) -> Option<&ModSource> {
        if m.file == file {
            return Some(m);
        }
        m.children.values().find_map(|c| module(c, file))
    }
    let m = a.sources.iter().find_map(|c| module(&c.root, name.file))?;
    decl_in_items(&m.ast.items, name, None, None)
}

fn decl_in_items<'a>(items: &'a [ast::Item], name: Span, parent: Option<&'a ast::Item>, abi: Option<&'a str>) -> Option<Decl<'a>> {
    items.iter().find_map(|item| decl_in_item(item, name, parent, abi))
}

fn decl_in_item<'a>(item: &'a ast::Item, name: Span, parent: Option<&'a ast::Item>, abi: Option<&'a str>) -> Option<Decl<'a>> {
    use ast::ItemKind as K;
    let here = Decl::Item { item, parent, abi };
    let params = |g: &'a ast::Generics| g.params.iter().find(|p| p.name.span == name).map(Decl::Param);
    match &item.kind {
        K::Fn(f) if f.name.span == name => Some(here),
        K::Fn(f) => params(&f.generics).or_else(|| {
            // Items declared inside the function's body.
            f.body.as_ref()?.stmts.iter().find_map(|s| match &s.kind {
                ast::StmtKind::Item(i) => decl_in_item(i, name, None, None),
                _ => None,
            })
        }),
        K::Struct(s) if s.name.span == name => Some(here),
        K::Struct(s) => params(&s.generics),
        K::Enum(e) if e.name.span == name => Some(here),
        K::Enum(e) => params(&e.generics).or_else(|| e.variants.iter().find(|v| v.name.span == name).map(Decl::Variant)),
        K::Trait(t) if t.name.span == name => Some(here),
        K::Trait(t) => params(&t.generics).or_else(|| decl_in_items(&t.items, name, Some(item), None)),
        K::Impl(i) => params(&i.generics).or_else(|| decl_in_items(&i.items, name, Some(item), None)),
        K::Mod(m) if m.name.span == name => Some(here),
        K::Mod(m) => decl_in_items(m.items.as_deref()?, name, None, None),
        K::Const(c) => (c.name.span == name).then_some(here),
        K::Static(s) => (s.name.span == name).then_some(here),
        K::TypeAlias(t) if t.name.span == name => Some(here),
        K::TypeAlias(t) => params(&t.generics),
        K::Extern(b) => decl_in_items(&b.items, name, None, Some(b.abi.as_deref().unwrap_or("C"))),
        K::Use(_) => None,
    }
}

/// Markdown hover text for `offset`, and the span it describes.
pub fn hover(a: &Analysis, file: FileId, offset: u32) -> Option<(Span, String)> {
    let p = a.program.as_ref()?;
    if let Some((span, target)) = ref_at(p, file, offset) {
        let text = match target {
            RefTarget::Def(d) => describe_def(a, p, d),
            // Just the parameter and its bounds: `T: Ord + Clone`.
            RefTarget::TyParam(decl) => match find_decl(a, decl)? {
                Decl::Param(g) => code_block(&pretty::generic_param(g)),
                _ => return None,
            },
            RefTarget::Field { adt, variant, field } => {
                let data = p.adt(adt);
                let v = &data.variants[variant];
                let f = &v.fields[field];
                let owner = if data.is_enum { p.path_of(v.def) } else { p.path_of(adt) };
                let offset = if data.is_enum { None } else { p.field_offsets(adt, &[]).map(|o| o[field]) };
                let layout = layout_comment(p.layout_of(&f.ty), offset);
                let vis = if f.is_pub { "pub " } else { "" };
                format!("{}\n\n```lunar\n{layout}{vis}{}: {}\n```", code_block(&owner), f.name, p.ty_str(&f.ty))
            }
            RefTarget::Local { body, local } => {
                let l = p.bodies.get(&body)?.local(local);
                let mut_kw = if l.mutable { "mut " } else { "" };
                let (before, mode) = match l.kind {
                    thir::LocalKind::Param { inout: true, .. } => (String::new(), "&mut "),
                    thir::LocalKind::Param { owned: false, .. } => (String::new(), "&"),
                    thir::LocalKind::Param { .. } => (mut_kw.to_string(), ""),
                    _ => (format!("let {mut_kw}"), ""),
                };
                let layout = layout_comment(p.layout_of(l.ty.strip_bang()), None);
                format!("```lunar\n{layout}{before}{}: {mode}{}\n```", l.name, p.ty_str(&l.ty))
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

/// Declaration (printed from the AST), container path and doc comments of a
/// definition.
fn describe_def(a: &Analysis, p: &Program, d: DefId) -> String {
    let map = &a.map;
    let info = p.def(d);
    // Like rust-analyzer: the container path, then the declaration (with the
    // memory layout as a comment for types, and the enclosing `impl` or
    // `trait` for methods), then the docs.
    let mut decl = String::new();
    if let DefData::Adt(adt) = &info.data
        && adt.generics.names.is_empty()
    {
        decl.push_str(&layout_comment(p.layout_of(&Ty::Adt(d, Vec::new())), None));
    }
    let found = if info.span.is_dummy() { None } else { find_decl(a, info.span) };
    let sig = match found {
        Some(Decl::Item { item, parent, abi }) => {
            if let Some(parent) = parent {
                decl.push_str(&pretty::item(parent, None));
                decl.push('\n');
            }
            pretty::item(item, abi)
        }
        Some(Decl::Variant(v)) => pretty::variant(v),
        Some(Decl::Param(g)) => pretty::generic_param(g),
        None => info.name.clone(),
    };
    decl.push_str(&sig);
    // Methods: the path of the type for user-defined types (`tour::Point`),
    // else the module of the impl (`core::num` for `impl i32`), as
    // rust-analyzer does.
    let container = match info.parent.map(|pd| (pd, p.def(pd))) {
        Some((pd, pdi)) if pdi.kind == DefKind::Impl => match &p.impl_data(pd).self_ty {
            Ty::Adt(adt, _) => p.path_of(*adt),
            _ => p.path_of(pdi.module),
        },
        _ => {
            let path = p.path_of(d);
            path.rsplit_once("::").map(|(c, _)| c.to_string()).unwrap_or_default()
        }
    };
    let mut out = String::new();
    if !container.is_empty() {
        out.push_str(&code_block(&container));
        out.push_str("\n\n");
    }
    out.push_str(&code_block(&decl));
    let docs = if info.span.is_dummy() { String::new() } else { doc_comment(map, info.span) };
    if !docs.is_empty() {
        out.push_str("\n\n---\n\n");
        out.push_str(&docs);
    }
    out
}

fn code_block(code: &str) -> String {
    format!("```lunar\n{code}\n```")
}

/// rust-analyzer's memory layout line: `// size = 24 (0x18), align = 0x8`,
/// plus `offset` for fields. Empty when the layout is not known (generics).
fn layout_comment(layout: Option<ln_sema::layout::Layout>, offset: Option<u64>) -> String {
    let Some(l) = layout else { return String::new() };
    let size = if l.size >= 10 { format!("{} ({:#X})", l.size, l.size) } else { l.size.to_string() };
    let offset = offset.map(|o| format!(", offset = {o:#X}")).unwrap_or_default();
    format!("// size = {size}, align = {:#X}{offset}\n", l.align)
}

/// Documentation for the keyword under the cursor, in rust-analyzer's style.
fn keyword_hover(map: &SourceMap, file: FileId, offset: u32) -> Option<(Span, String)> {
    let text = &map.get(file).text;
    let (toks, _) = ln_lexer::lex(text);
    let tok = toks.iter().find(|t| t.range.start as u32 <= offset && offset < t.range.end as u32)?;
    let word = &text[tok.range.clone()];
    let doc = keyword_doc(word)?;
    Some((Span::new(file, tok.range.start as u32, tok.range.end as u32), format!("{}\n\n---\n\n{doc}", code_block(word))))
}

fn keyword_doc(word: &str) -> Option<&'static str> {
    Some(match word {
        "fn" => {
            r#"A function or function pointer.

Functions are the primary way code is executed. A parameter `x: T` takes ownership of its argument, `x: &T` borrows it, and `x: &mut T` is in-out. The return type follows `:`, and generic parameters use `::<>`.

```lunar
fn max::<T: Ord>(a: T, b: T): T {
    if a > b { a } else { b }
}
```"#
        }
        "let" => {
            r#"Bind a value to a variable.

`let` bindings are immutable; `let mut` makes them mutable. The type is usually inferred. The left side is a pattern, so `let` can also destructure.

```lunar
let x: i32 = 1;
let mut total = 0;
let (a, b) = (1, 2);
```"#
        }
        "mut" => {
            r#"A mutable variable, parameter, or in-out parameter.

On a `let`, `mut` allows the variable to be reassigned and its contents changed. On a parameter that takes ownership (`mut x: T`, `mut self`), it lets the function change its own value; the caller does not see the change. As `&mut T` (or `&mut self`), the parameter is in-out: the caller passes a mutable place and sees the changes after the call. A `!` parameter must be `&mut`.

```lunar
fn bump(x: &mut i32) {
    x += 1;
}

let mut n = 1;
bump(n); // n == 2
```"#
        }
        "return" => {
            r#"Returns a value from a function.

`return` exits the current function immediately. Without a value it returns `()`. The `return` expression itself has type `!`, so it fits anywhere a value is expected. A block's final expression without a semicolon is also its value.

```lunar
fn sign(x: i32): i32 {
    if x < 0 {
        return -1;
    }
    1
}
```"#
        }
        "if" => {
            r#"Evaluate a block if a condition holds.

`if` is an expression: with an `else` branch, it produces the value of the branch that ran.

```lunar
let kind = if n % 2 == 0 { "even" } else { "odd" };
```"#
        }
        "else" => {
            r#"What expression to evaluate when an `if` condition evaluates to `false`.

```lunar
if x > 0 {
    println("positive");
} else if x < 0 {
    println("negative");
} else {
    println("zero");
}
```"#
        }
        "match" => {
            r#"Control flow based on pattern matching.

Arms are tried in order; the first whose pattern matches (and whose `if` guard holds) runs. The arms must cover every possible value, or the compiler reports `err(type::non_exhaustive)`.

```lunar
match shape {
    Shape::Circle(r) => 3.14 * r * r,
    Shape::Rect { w, h } => w * h,
    Shape::Empty => 0.0,
}
```"#
        }
        "for" => {
            r#"Iteration over a range or the elements of an array.

`for` loops over an integer range, or over the elements of a Dull array, slice or anything implementing `Iterator`. Elements are copied out.

```lunar
for i in 0..10 {
    println("{i}");
}
for x in numbers {
    total += x;
}
```"#
        }
        "in" => {
            r#"Iterate over a series of values with `for`.

```lunar
for i in 0..3 { }
```"#
        }
        "while" => {
            r#"Loop while a condition is upheld.

```lunar
let mut i = 0;
while i < 10 {
    i += 1;
}
```"#
        }
        "loop" => {
            r#"Loop indefinitely.

`loop` runs its body until a `break`. A `break` can carry a value, which becomes the value of the `loop` expression.

```lunar
let first_big = loop {
    n += 1;
    if n * n > 100 {
        break n;
    }
};
```"#
        }
        "break" => {
            r#"Exit early from a loop.

`break` leaves the innermost loop. Inside `loop`, it can carry a value. The expression has type `!`."#
        }
        "continue" => {
            r#"Skip to the next iteration of a loop.

The expression has type `!`."#
        }
        "struct" => {
            r#"A type that is composed of other types.

Structs come in three forms: with named fields, tuple structs, and unit structs. Fields are private to the module that declares the struct unless marked `pub`.

```lunar
struct Point {
    pub x: f64,
    pub y: f64,
}

struct Meters(f64);

struct Marker;
```"#
        }
        "enum" => {
            r#"A type that can be any one of several variants.

Variants can carry data, either as a tuple or with named fields. Variant fields are always public.

```lunar
enum Shape {
    Circle(f64),
    Rect { w: f64, h: f64 },
    Empty,
}
```"#
        }
        "impl" => {
            r#"Implementations of functionality for a type, or a type implementing some functionality.

An inherent `impl` adds associated functions and methods to a type. A trait `impl` implements a trait for a type. Two impls of the same trait may not apply to the same type.

```lunar
impl Point {
    pub fn origin(): Point {
        return Point { x: 0.0, y: 0.0 };
    }
}

impl Display for Point {
    fn fmt(self, mut out: String) {
        out.write("({self.x}, {self.y})");
    }
}
```"#
        }
        "trait" => {
            r#"A common interface for a group of types.

A trait declares methods that implementing types provide; methods may have default bodies. Supertraits are written `trait A: B`.

```lunar
trait Describe {
    fn name(self): str;
    fn describe(self) {
        println("a {self.name()}");
    }
}
```"#
        }
        "type" => {
            r#"Define an alias for an existing type.

```lunar
type Bytes = [u8; -..+];
```"#
        }
        "mod" => {
            r#"Organize code into modules.

`mod m { ... }` declares a module inline. `mod m;` loads it from `m.ln` or `m/mod.ln` next to the declaring file."#
        }
        "use" => {
            r#"Import or rename items from other modules.

```lunar
use std::io::println;
use core::ops::{Add, Sub};
```"#
        }
        "pub" => {
            r#"Make an item visible to others.

Items, fields and methods are private to the module that declares them (and its children) unless marked `pub`."#
        }
        "const" => {
            r#"Compile-time constants.

The value is evaluated during compilation and inlined at each use.

```lunar
const LIMIT: usize = 1024;
```"#
        }
        "static" => {
            r#"A static item is a value which is valid for the entire duration of your program.

A `static` has a fixed address. A `static mut` can be changed, but every read or write of it requires `unsafe`.

```lunar
static GREETING: str = "hello";
```"#
        }
        "unsafe" => {
            r#"Code or interfaces whose memory safety cannot be verified by the type system.

An `unsafe { }` block allows unsafe operations: calling `unsafe` or `extern` functions, dereferencing raw pointers, and accessing a `static mut`. Marking a function `unsafe fn` only makes it unsafe to call. Its body gets no implicit unsafe context, so unsafe operations inside it still need an `unsafe` block (a hard error otherwise)."#
        }
        "extern" => {
            r#"Link to or import external code.

An `extern "C"` block declares functions implemented elsewhere with the C calling convention. Calling them requires `unsafe`.

```lunar
extern "C" {
    fn puts(s: *u8): i32;
}
```"#
        }
        "as" => {
            r#"Cast between types, or rename an import.

```lunar
let n = x as u64;
use std::io::println as say;
```"#
        }
        "self" => {
            r#"The receiver of a method, or the current module.

As a method's first parameter, `self` is the value the method is called on: `self` takes ownership of it, `&self` borrows it, `&mut self` makes it in-out, and `&mut !self` may shrink it. In paths, `self::x` names an item of the current module."#
        }
        "Self" => r#"The implementing type within a `trait` or `impl` block, or within a type definition."#,
        "crater" => {
            r#"A path segment that refers to the root of the current crater.

A crater is what Lunar compiles as one unit: a root file and every module it loads with `mod`. Your program is a crater, and so are `core` and `std`. In a path, `crater` names the root module of the crater the code is in.

```lunar
crater::util::helper();
```"#
        }
        "super" => {
            r#"The parent of the current module.

```lunar
super::helper();
```"#
        }
        "true" => r#"A value of type `bool` representing logical **true**."#,
        "false" => r#"A value of type `bool` representing logical **false**."#,
        _ => return None,
    })
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
