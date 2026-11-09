//! Printing declarations back as Lunar source, for editor hovers: names,
//! generics, parameters and types, without bodies, attributes or comments.
//! Output is normalized (one space after `,` and `:`, ` + ` between bounds),
//! not a copy of how the source was formatted.

use crate::*;

/// `a::b::<T, U>::c`
pub fn path(p: &Path) -> String {
    let mut out = String::new();
    for (i, seg) in p.segments.iter().enumerate() {
        if i > 0 {
            out.push_str("::");
        }
        out.push_str(&seg.ident.name);
        if let Some(args) = &seg.args {
            out.push_str("::<");
            out.push_str(&list(args.iter().map(ty)));
            out.push('>');
        }
    }
    out
}

pub fn ty(t: &Ty) -> String {
    match &t.kind {
        TyKind::Path(p) => path(p),
        TyKind::Array(elem, size) => {
            let size = match size {
                ArraySize::Fixed(e) => expr(e),
                ArraySize::Inferred => "_".into(),
                ArraySize::Grow => "..+".into(),
                ArraySize::GrowShrink => "-..+".into(),
                ArraySize::Slice => "&".into(),
                ArraySize::SliceMut => "&mut".into(),
            };
            format!("[{}; {size}]", ty(elem))
        }
        TyKind::DynArray(elem) => format!("[{}]", ty(elem)),
        TyKind::Bang(inner) => format!("!{}", ty(inner)),
        TyKind::Ptr { is_mut, inner } => format!("*{}{}", if *is_mut { "mut " } else { "" }, ty(inner)),
        TyKind::Tuple(items) if items.len() == 1 => format!("({},)", ty(&items[0])),
        TyKind::Tuple(items) => format!("({})", list(items.iter().map(ty))),
        TyKind::Fn { params, ret } => {
            let params = list(params.iter().map(|(mode, t)| format!("{}{}", mode_prefix(*mode), ty(t))));
            match ret {
                Some(r) => format!("fn({params}): {}", ty(r)),
                None => format!("fn({params})"),
            }
        }
        TyKind::Never => "!".into(),
        TyKind::Infer => "_".into(),
        TyKind::MacCall(m) => mac_call(m),
    }
}

/// `path!(tokens)`.
pub fn mac_call(m: &MacCall) -> String {
    format!("{}!{}{}{}", path(&m.path), m.delim.open(), tokens(&m.tokens), m.delim.close())
}

/// Token trees as source text, one space between tokens except where that
/// would look odd (`a::b`, `f(x)`, `x.y`, `$x`).
pub fn tokens(trees: &[TokenTree]) -> String {
    let mut out = String::new();
    let mut prev_glues = true;
    for t in trees {
        let (text, glue_before, glue_after) = match t {
            TokenTree::Token(tok) => {
                let glue = matches!(tok.text.as_str(), "::" | "." | "," | ";" | ":" | "?" | "!");
                (tok.text.clone(), glue, matches!(tok.text.as_str(), "::" | "." | "$" | "&" | "#"))
            }
            TokenTree::Group { delim, trees, .. } => (format!("{}{}{}", delim.open(), tokens(trees), delim.close()), *delim != Delim::Brace, false),
        };
        if !out.is_empty() && !prev_glues && !glue_before {
            out.push(' ');
        }
        out.push_str(&text);
        prev_glues = glue_after;
    }
    out
}

fn mode_prefix(mode: ParamMode) -> &'static str {
    match mode {
        ParamMode::Take => "",
        ParamMode::Borrow => "&",
        ParamMode::InOut => "&mut ",
    }
}

/// `T: Ord + Clone`, or just `T`.
pub fn generic_param(p: &GenericParam) -> String {
    if p.bounds.is_empty() {
        return p.name.name.clone();
    }
    format!("{}: {}", p.name.name, p.bounds.iter().map(path).collect::<Vec<_>>().join(" + "))
}

/// `::<T: Ord, U>`, or nothing without parameters.
pub fn generics(g: &Generics) -> String {
    if g.params.is_empty() {
        return String::new();
    }
    format!("::<{}>", list(g.params.iter().map(generic_param)))
}

pub fn param(p: &Param) -> String {
    let mut_kw = if p.is_mut { "mut " } else { "" };
    match &p.kind {
        ParamKind::SelfParam => match p.mode {
            ParamMode::Take => format!("{mut_kw}self"),
            ParamMode::Borrow => "&self".into(),
            ParamMode::InOut if p.self_bang => "&mut !self".into(),
            ParamMode::InOut => "&mut self".into(),
        },
        ParamKind::Named(name) => format!("{mut_kw}{}: {}{}", name.name, mode_prefix(p.mode), p.ty.as_ref().map(ty).unwrap_or_default()),
        ParamKind::Wild => format!("_: {}{}", mode_prefix(p.mode), p.ty.as_ref().map(ty).unwrap_or_default()),
    }
}

/// `unsafe fn name::<T: Ord>(a: T, b: &T): T`, without `pub`.
pub fn fn_sig(f: &FnDecl) -> String {
    let mut params: Vec<String> = f.params.iter().map(param).collect();
    if f.variadic {
        params.push("...".into());
    }
    let unsafe_kw = if f.is_unsafe { "unsafe " } else { "" };
    let ret = f.ret.as_ref().map(|r| format!(": {}", ty(r))).unwrap_or_default();
    format!("{unsafe_kw}fn {}{}({}){ret}", f.name.name, generics(&f.generics), params.join(", "))
}

/// A variant as written in its enum: `Some(T)`, `Point { x: i32 }`, `None`.
pub fn variant(v: &VariantDecl) -> String {
    format!("{}{}", v.name.name, variant_data(&v.data, false))
}

/// The fields after a struct or variant name. Named fields go one per line
/// when `multiline` (structs), else on one line (`Rect { w: f64, h: f64 }`).
fn variant_data(data: &VariantData, multiline: bool) -> String {
    let vis = |is_pub: bool| if is_pub { "pub " } else { "" };
    match data {
        VariantData::Unit => String::new(),
        VariantData::Tuple(fields) => format!("({})", list(fields.iter().map(|f| format!("{}{}", vis(f.is_pub), ty(&f.ty))))),
        VariantData::Named(fields) if fields.is_empty() => " {}".into(),
        VariantData::Named(fields) if !multiline => {
            format!(" {{ {} }}", list(fields.iter().map(|f| format!("{}{}: {}", vis(f.is_pub), f.name.name, ty(&f.ty)))))
        }
        VariantData::Named(fields) => {
            let mut out = String::from(" {\n");
            for f in fields {
                out.push_str(&format!("    {}{}: {},\n", vis(f.is_pub), f.name.name, ty(&f.ty)));
            }
            out.push('}');
            out
        }
    }
}

/// The declaration of `item`: the signature of a function, the header of a
/// trait or impl, a struct with its fields, an enum with its variants.
/// `extern_abi` is the ABI of the `extern` block the item is in, if any.
/// Attributes are never included.
pub fn item(item: &Item, extern_abi: Option<&str>) -> String {
    let vis = if item.is_pub { "pub " } else { "" };
    match &item.kind {
        ItemKind::Fn(f) => {
            let abi = extern_abi.map(|a| format!("extern {a:?} ")).unwrap_or_default();
            format!("{vis}{abi}{}", fn_sig(f))
        }
        ItemKind::Struct(s) => format!("{vis}struct {}{}{}", s.name.name, generics(&s.generics), variant_data(&s.kind, true)),
        ItemKind::Enum(e) => {
            let mut out = format!("{vis}enum {}{}", e.name.name, generics(&e.generics));
            if e.variants.is_empty() {
                out.push_str(" {}");
            } else {
                out.push_str(" {\n");
                for v in &e.variants {
                    out.push_str(&format!("    {},\n", variant(v)));
                }
                out.push('}');
            }
            out
        }
        ItemKind::Trait(t) => {
            let supers = if t.supertraits.is_empty() { String::new() } else { format!(": {}", t.supertraits.iter().map(path).collect::<Vec<_>>().join(" + ")) };
            format!("{vis}trait {}{}{supers}", t.name.name, generics(&t.generics))
        }
        ItemKind::Impl(i) => {
            let g = if i.generics.params.is_empty() { String::new() } else { format!("::<{}>", list(i.generics.params.iter().map(generic_param))) };
            match &i.trait_ref {
                Some(tr) => format!("impl{g} {}{} for {}", if i.negative { "!" } else { "" }, path(tr), ty(&i.self_ty)),
                None => format!("impl{g} {}", ty(&i.self_ty)),
            }
        }
        ItemKind::Mod(m) => format!("{vis}mod {}", m.name.name),
        ItemKind::Const(c) => format!("{vis}const {}: {} = {}", c.name.name, ty(&c.ty), expr(&c.value)),
        ItemKind::Static(s) => format!("{vis}static {}{}: {} = {}", if s.is_mut { "mut " } else { "" }, s.name.name, ty(&s.ty), expr(&s.value)),
        ItemKind::TypeAlias(a) => format!("{vis}type {}{} = {}", a.name.name, generics(&a.generics), ty(&a.ty)),
        ItemKind::Extern(b) => format!("extern {:?}", b.abi.as_deref().unwrap_or("C")),
        ItemKind::Macro(m) => format!("{vis}macro {}", m.name.name),
        ItemKind::MacCall(m) => mac_call(m),
        ItemKind::Use(_) => "use ...".into(),
    }
}

/// A constant expression (array sizes, `const` values). Anything more
/// involved than literals, paths and operators prints as `...`.
pub fn expr(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Lit(l) => lit(l),
        ExprKind::Path(p) => path(p),
        ExprKind::Unary(op, x) => {
            let op = match op {
                UnOp::Neg => "-",
                UnOp::Not => "!",
                UnOp::Deref => "*",
            };
            format!("{op}{}", expr(x))
        }
        ExprKind::Borrow { mutable, expr: x } => format!("&{}{}", if *mutable { "mut " } else { "" }, expr(x)),
        ExprKind::Binary(op, a, b) => format!("{} {} {}", expr(a), op.as_str(), expr(b)),
        ExprKind::Paren(x) => format!("({})", expr(x)),
        ExprKind::Cast(x, t) => format!("{} as {}", expr(x), ty(t)),
        ExprKind::Tuple(xs) => format!("({})", list(xs.iter().map(expr))),
        ExprKind::Array(xs) if xs.len() <= 8 => format!("[{}]", list(xs.iter().map(expr))),
        ExprKind::ArrayRepeat(x, n) => format!("[{}; {}]", expr(x), expr(n)),
        ExprKind::StructLit { path: p, fields } if fields.is_empty() => format!("{} {{}}", path(p)),
        _ => "...".into(),
    }
}

fn lit(l: &Lit) -> String {
    match l {
        Lit::Int { value, suffix } => format!("{value}{}", suffix.as_deref().unwrap_or("")),
        Lit::Float { value, suffix } => {
            let mut s = value.to_string();
            if !s.contains(['.', 'e', 'E']) && !s.contains("inf") && !s.contains("NaN") {
                s.push_str(".0");
            }
            format!("{s}{}", suffix.as_deref().unwrap_or(""))
        }
        Lit::Bool(b) => b.to_string(),
        Lit::Char(c) => format!("'{}'", escape(&c.to_string(), '\'')),
        Lit::Str { value, .. } => format!("\"{}\"", escape(value, '"')),
    }
}

fn escape(s: &str, quote: char) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            '\\' => out.push_str("\\\\"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}

fn list(items: impl Iterator<Item = String>) -> String {
    items.collect::<Vec<_>>().join(", ")
}
