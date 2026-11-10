//! Editor hovers: declarations are printed from the syntax tree.

use ln_driver::{AnalyzeOptions, analyze_with, ide};

const SRC: &str = r#"trait Named {
    fn name(&self): str;
}

#[inline]
pub fn largest::<T: Dull + Ord, U>(items: [T; &], extra: &mut U): T {
    return items[0];
}

#[test] fn adds() {}

/// Two values.
//// Not documentation.
struct Pair::<A: Dull, B: Dull + Named> {
    /// The first.
    pub a: A,
    b: [B; -..+],
}

/// The largest item.
#[inline]
/// Assumes there is one.
fn largest_doc(x: i32): i32 { return x; }

enum Shape {
    /// A single point.
    Dot,
    Line { /// From here.
        from: i32, to: i32 },
}

impl::<A: Dull, B: Dull + Named> Pair::<A, B> {
    pub fn first(&self): A { return self.a; }
}

fn main() {}
"#;

/// The hover text at the `n`th occurrence of `needle` (offset `at` into it).
fn hover_at(needle: &str, n: usize, at: usize) -> String {
    // Tests run in parallel: one file per call.
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let k = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("lunar-hover-{}-{k}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("main.ln");
    std::fs::write(&path, SRC).unwrap();
    if let Some(bin) = std::env::var_os("LUNAR_BIN") {
        return lsp::hover_at(&bin, &path, needle, n, at);
    }
    let a = analyze_with(&path, &AnalyzeOptions { keep_going: true, ..Default::default() });
    let errors: Vec<_> = a.diags.iter().filter(|d| d.is_error()).map(|d| format!("{}: {}", d.code, d.message)).collect();
    assert!(errors.is_empty(), "the sample has errors: {errors:#?}");
    let file = ide::file_for_path(&a.map, &path).expect("file in the analysis");
    let start = SRC.match_indices(needle).nth(n).unwrap_or_else(|| panic!("occurrence {n} of {needle:?}")).0;
    let (_, text) = ide::hover(&a, file, (start + at) as u32).unwrap_or_else(|| panic!("no hover at {needle:?}"));
    text
}

/// With `$LUNAR_BIN` (such as the compiler built from craters/), hovers
/// come from its language server.
mod lsp {
    use serde_json::{Value, json};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::path::Path;
    use std::process::{ChildStdin, ChildStdout, Command, Stdio};

    fn send(w: &mut ChildStdin, msg: Value) {
        let body = msg.to_string();
        write!(w, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        w.flush().unwrap();
    }

    fn recv(r: &mut BufReader<ChildStdout>) -> Value {
        let mut len = 0;
        loop {
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            let line = line.trim();
            if line.is_empty() {
                break;
            }
            if let Some(n) = line.strip_prefix("Content-Length: ") {
                len = n.parse().unwrap();
            }
        }
        let mut buf = vec![0; len];
        r.read_exact(&mut buf).unwrap();
        serde_json::from_slice(&buf).unwrap()
    }

    pub fn hover_at(bin: &std::ffi::OsStr, path: &Path, needle: &str, n: usize, at: usize) -> String {
        let start = super::SRC.match_indices(needle).nth(n).unwrap_or_else(|| panic!("occurrence {n} of {needle:?}")).0 + at;
        let before = &super::SRC[..start];
        let line = before.matches('\n').count();
        let character = before.len() - before.rfind('\n').map_or(0, |i| i + 1);
        let uri = format!("file:///{}", path.to_string_lossy().replace('\\', "/").trim_start_matches('/'));
        let mut child = Command::new(bin).args(["lsp", "--stdio"]).env_remove("LUNAR_STD_DIR").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        let mut w = child.stdin.take().unwrap();
        let mut r = BufReader::new(child.stdout.take().unwrap());
        send(&mut w, json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } }));
        recv(&mut r);
        send(&mut w, json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }));
        send(&mut w, json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri, "languageId": "lunar", "version": 1, "text": super::SRC } } }));
        let pos = json!({ "textDocument": { "uri": uri }, "position": { "line": line, "character": character } });
        send(&mut w, json!({ "jsonrpc": "2.0", "id": 2, "method": "textDocument/hover", "params": pos }));
        let hover = loop {
            let m = recv(&mut r);
            if m["method"] == "textDocument/publishDiagnostics" {
                let errors: Vec<&Value> = m["params"]["diagnostics"].as_array().unwrap().iter().filter(|d| d["severity"] == 1).collect();
                assert!(errors.is_empty(), "the sample has errors: {errors:#?}");
            }
            if m["id"] == 2 {
                break m;
            }
        };
        send(&mut w, json!({ "jsonrpc": "2.0", "id": 3, "method": "shutdown" }));
        while recv(&mut r)["id"] != 3 {}
        send(&mut w, json!({ "jsonrpc": "2.0", "method": "exit" }));
        assert!(child.wait().unwrap().success());
        hover["result"]["contents"]["value"].as_str().unwrap_or_else(|| panic!("no hover at {needle:?}: {hover}")).to_string()
    }
}

#[test]
fn type_params_show_name_and_bounds() {
    // At the declaration and at uses.
    assert_eq!(hover_at("T: Dull", 0, 0), "```lunar\nT: Dull + Ord\n```");
    assert_eq!(hover_at("[T; &]", 0, 1), "```lunar\nT: Dull + Ord\n```");
    assert_eq!(hover_at("): T {", 0, 3), "```lunar\nT: Dull + Ord\n```");
    assert_eq!(hover_at("&mut U", 0, 5), "```lunar\nU\n```");
    assert_eq!(hover_at("[B; -..+]", 0, 1), "```lunar\nB: Dull + Named\n```");
    // An impl's parameters, used in a method.
    assert_eq!(hover_at("): A {", 0, 3), "```lunar\nA: Dull\n```");
}

#[test]
fn functions_are_printed_without_attributes() {
    let h = hover_at("largest", 0, 0);
    assert!(h.contains("pub fn largest::<T: Dull + Ord, U>(items: [T; &], extra: &mut U): T"), "{h}");
    assert!(!h.contains("#["), "{h}");
    let h = hover_at("adds", 0, 0);
    assert!(h.contains("fn adds()") && !h.contains("#["), "{h}");
}

#[test]
fn doc_comments_come_from_the_syntax_tree() {
    let h = hover_at("largest_doc", 0, 0);
    assert!(h.ends_with("\n\n---\n\nThe largest item.\nAssumes there is one."), "{h}");
    let h = hover_at("Pair", 0, 0);
    assert!(h.ends_with("\n\n---\n\nTwo values."), "a `////` comment is not documentation: {h}");
    let h = hover_at("pub a", 0, 4);
    assert!(h.ends_with("\n\n---\n\nThe first."), "{h}");
    let h = hover_at("Dot", 0, 0);
    assert!(h.ends_with("\n\n---\n\nA single point."), "{h}");
    let h = hover_at("from:", 0, 0);
    assert!(h.ends_with("\n\n---\n\nFrom here."), "{h}");
    let h = hover_at("Shape", 0, 0);
    assert!(!h.contains("---"), "no docs: {h}");
}

#[test]
fn types_and_methods() {
    let h = hover_at("Pair", 0, 0);
    assert!(h.contains("struct Pair::<A: Dull, B: Dull + Named> {\n    pub a: A,\n    b: [B; -..+],\n}"), "{h}");
    let h = hover_at("fn first", 0, 3);
    assert!(h.contains("impl::<A: Dull, B: Dull + Named> Pair::<A, B>\npub fn first(&self): A"), "{h}");
    let h = hover_at("name", 0, 0);
    assert!(h.contains("trait Named\nfn name(&self): str"), "{h}");
}
