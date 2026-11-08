use crate::{IdGen, parse_file};
use ln_ast::*;

fn parse_ok(src: &str) -> File {
    let mut ids = IdGen::new();
    let (f, diags) = parse_file(src, 0, &mut ids);
    if !diags.is_empty() {
        let mut map = ln_diag::SourceMap::new();
        map.add("test.ln", None, src);
        panic!("unexpected diagnostics:\n{}", ln_diag::render_to_string(&map, &diags));
    }
    f
}

fn parse_err(src: &str) -> Vec<ln_diag::Diagnostic> {
    let mut ids = IdGen::new();
    parse_file(src, 0, &mut ids).1
}

#[test]
fn example_arrays_parses() {
    let src = include_str!("../../../examples/arrays.ln");
    let f = parse_ok(src);
    assert!(f.items.len() >= 4);
}

#[test]
fn ui_pointy_parses() {
    parse_ok(include_str!("../../../tests/ui/array_pointy_items.ln"));
}

#[test]
fn generics_and_traits() {
    parse_ok(
        r#"
        pub struct Pair::<T> { pub a: T, b: T }
        enum Option::<T> { Some(T), None }
        trait Show: Display + Dull { fn show(self): str; fn dflt(self) { } }
        impl::<T: Dull> Show for Pair::<T> { fn show(self): str { return "pair"; } }
        impl !Dull for Foo {}
        fn f::<T: Dull + Eq, U>(x: Pair::<Pair::<T>>, mut s: ![T; -..+], p: *mut u8): ! { loop {} }
        extern "C" { fn write(fd: i32, buf: *u8, n: usize): isize; fn printf(f: *u8, ...): i32; }
        #[cfg(windows)] use std::io::{print, println as pl};
        const N: usize = 4;
        static mut COUNT: u64 = 0;
        type Bytes = [u8; &];
        "#,
    );
}

#[test]
fn expressions() {
    let f = parse_ok(
        r#"
        fn main() {
            let x = a.b.c(1, 2)[3].0;
            let y = if a < b { 1 } else if c >= d { 2 } else { 3 };
            let z = match o { Some(v) if v > 0 => v, None | Some(_) => 0, };
            x >>= 2; y = a >> b;
            let p = Point { x: 1, y };
            for i in 0..10 { if i == 3 { continue; } }
            while let Some(v) = it.next() { break; }
            let s = arr[..]; let t = arr[2..6]; let u = arr[..=n];
            let q = -x as i64 * 2 + !flag as i64;
            unsafe { *p = 3; }
            let w = Pair::<i32>::new(1, 2);
            let r = [0.; 8];
            if c { } *p = 1;
            x
        }
        "#,
    );
    let ItemKind::Fn(fd) = &f.items[0].kind else { panic!() };
    let body = fd.body.as_ref().unwrap();
    assert!(body.tail.is_some());
}

#[test]
fn format_strings() {
    let f = parse_ok(r#"fn main() { println("a {x[1]} b {{lit}} {f(1, 2)}"); }"#);
    let ItemKind::Fn(fd) = &f.items[0].kind else { panic!() };
    let StmtKind::Expr(e) = &fd.body.as_ref().unwrap().stmts[0].kind else { panic!() };
    let ExprKind::Call(_, args) = &e.kind else { panic!() };
    let ExprKind::Lit(Lit::Str { fmt: FmtString::Ok(pieces), .. }) = &args[0].kind else { panic!("{:?}", args[0]) };
    assert_eq!(pieces.len(), 4, "{pieces:?}");
}

#[test]
fn missing_semicolon_is_an_error() {
    let d = parse_err("fn main() { a() b(); }");
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].code, ln_diag::codes::syntax::MISSING_SEMI);
}

#[test]
fn recovers_after_errors() {
    let d = parse_err("fn a( { } fn b() { let = 3; } struct S { x: }");
    assert!(d.len() >= 2, "{d:?}");
}
