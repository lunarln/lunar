//! One scripted language server session (initialize, a document with an
//! error, hovers, definitions, a fix, std documents, an unsupported
//! request, close, shutdown), run against this crate's `lunar lsp` and,
//! with `$LUNAR_BIN` set (such as the compiler built from craters/),
//! against that one too: both must answer the same.
//!
//! What may differ is normalized away: the order of keys, numbers written
//! as `1` or `1.0`, the order of notifications relative to responses, and
//! where the standard library is (the Rust compiler's embedded copy is the
//! virtual `lunar-std:std::io`, a compiler that reads std from disk points
//! at `.../std/std/io.ln`).

use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, ChildStdout, Command, Stdio};

const SRC: &str = r#"/// A pair of values.
struct Pair::<T: Dull> {
    /// The first.
    pub a: T,
    b: T,
}

impl::<T: Dull> Pair::<T> {
    pub fn first(&self): T {
        return self.a;
    }
}

/// Adds two numbers.
fn add(a: i32, b: i32): i32 {
    return a + b;
}

enum Shape {
    Dot,
    Line { from: i32, to: i32 },
}

fn main() {
    let total = add(1, 2);
    let p = Pair { a: 1, b: 2 };
    let f = p.first();
    let mut items: [i32; -..+] = [];
    items.push(total + f);
    let s = Shape::Line { from: 1, to: 2 };
    match s {
        Shape::Line { from, .. } => println("{from}"),
        Shape::Dot => {}
    }
    println("{total} {items.len()} {p.b}");
    let oops: i32 = undefined_name;
}
"#;

/// Places to hover over and go to the definition of: (text, occurrence,
/// offset into it).
const PLACES: &[(&str, usize, usize)] = &[
    ("add(1", 0, 0),
    ("add(a", 0, 1),
    ("total =", 0, 2),
    ("Pair {", 0, 1),
    ("Pair::<T: Dull> {", 0, 0),
    ("T: Dull> {", 0, 0),
    ("first()", 0, 1),
    ("pub fn first", 0, 8),
    ("pub a", 0, 4),
    ("p.b", 0, 2),
    ("items.push", 0, 7),
    ("items.push", 0, 1),
    ("println(\"{total}", 0, 3),
    ("return a", 0, 2),
    ("a + b", 0, 2),
    ("Shape::Line {", 0, 8),
    ("from, ..", 0, 1),
    ("Dot,", 0, 0),
    ("match s", 0, 1),
    ("[i32; -..+]", 0, 3),
    ("undefined_name", 0, 3),
    ("impl::<T", 0, 0),
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// A position in `SRC`.
fn position(needle: &str, n: usize, at: usize) -> Value {
    let start = SRC.match_indices(needle).nth(n).unwrap_or_else(|| panic!("occurrence {n} of {needle:?}")).0 + at;
    let before = &SRC[..start];
    let line = before.matches('\n').count();
    let character = before.len() - before.rfind('\n').map_or(0, |i| i + 1);
    json!({ "line": line, "character": character })
}

struct Client {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    /// Everything the server sent.
    got: Vec<Value>,
    next_id: i64,
}

impl Client {
    fn send(&mut self, msg: Value) {
        let body = msg.to_string();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        self.stdin.flush().unwrap();
    }

    fn recv(&mut self) -> Value {
        let mut len = 0;
        loop {
            let mut line = String::new();
            assert!(self.stdout.read_line(&mut line).unwrap() > 0, "the server closed its output");
            let line = line.trim();
            if line.is_empty() {
                break;
            }
            if let Some(n) = line.strip_prefix("Content-Length: ") {
                len = n.parse().unwrap();
            }
        }
        let mut buf = vec![0; len];
        self.stdout.read_exact(&mut buf).unwrap();
        serde_json::from_slice(&buf).unwrap()
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let m = self.recv();
            self.got.push(m.clone());
            if m.get("id") == Some(&json!(id)) {
                return m;
            }
        }
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }
}

/// Run the session against `bin`: every message the server sent.
fn session(bin: &std::ffi::OsStr, tag: &str) -> Vec<Value> {
    let dir = std::env::temp_dir().join(format!("lunar-lsp-session-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.ln");
    // The editor's buffer, not the disk, is analyzed.
    std::fs::write(&file, "").unwrap();
    let uri = format!("file:///{}", file.to_string_lossy().replace('\\', "/").trim_start_matches('/'));

    let mut child = Command::new(bin).args(["lsp", "--stdio"]).env_remove("LUNAR_STD_DIR").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let mut c = Client { stdin: child.stdin.take().unwrap(), stdout: BufReader::new(child.stdout.take().unwrap()), got: Vec::new(), next_id: 0 };
    c.request("initialize", json!({ "processId": null, "rootUri": null, "capabilities": {} }));
    c.notify("initialized", json!({}));
    c.notify("textDocument/didOpen", json!({ "textDocument": { "uri": uri, "languageId": "lunar", "version": 1, "text": SRC } }));
    for &(needle, n, at) in PLACES {
        let p = json!({ "textDocument": { "uri": uri }, "position": position(needle, n, at) });
        c.request("textDocument/hover", p.clone());
        c.request("textDocument/definition", p);
    }
    // Inside the standard library.
    let text = c.request("lunar/stdSource", json!({ "uri": "lunar-std:std%3A%3Aio" }));
    let text = text["result"].as_str().unwrap_or_default().to_string();
    if let Some(line) = text.lines().position(|l| l.contains("pub fn println(")) {
        c.request("textDocument/hover", json!({ "textDocument": { "uri": "lunar-std:std::io" }, "position": { "line": line, "character": 8 } }));
        c.request("textDocument/definition", json!({ "textDocument": { "uri": "lunar-std:std::io" }, "position": { "line": line, "character": 8 } }));
    }
    c.request("lunar/stdSource", json!({ "uri": "lunar-std:no::such" }));
    c.request("lunar/stdSource", json!({ "uri": "file:///x.ln" }));
    c.request("textDocument/formatting", json!({ "textDocument": { "uri": uri }, "options": { "tabSize": 4, "insertSpaces": true } }));
    c.request("textDocument/hover", json!({ "textDocument": { "uri": "file:///not/open.ln" }, "position": { "line": 0, "character": 0 } }));
    // Fix the error, then close the document.
    let fixed = SRC.replace("undefined_name", "total");
    c.notify("textDocument/didChange", json!({ "textDocument": { "uri": uri, "version": 2 }, "contentChanges": [{ "text": fixed }] }));
    c.request("textDocument/hover", json!({ "textDocument": { "uri": uri }, "position": position("total =", 0, 2) }));
    c.notify("textDocument/didClose", json!({ "textDocument": { "uri": uri } }));
    c.request("shutdown", Value::Null);
    c.notify("exit", Value::Null);
    assert!(child.wait().unwrap().success(), "{tag}: the server failed");
    let _ = std::fs::remove_dir_all(&dir);
    // The URI of the document, which differs between the two runs.
    let mut got = c.got;
    for m in &mut got {
        replace_strings(m, &uri, "<doc>");
    }
    got
}

fn replace_strings(v: &mut Value, from: &str, to: &str) {
    match v {
        Value::String(s) if s == from => *s = to.to_string(),
        Value::Array(a) => a.iter_mut().for_each(|x| replace_strings(x, from, to)),
        Value::Object(o) => o.values_mut().for_each(|x| replace_strings(x, from, to)),
        _ => {}
    }
}

/// `core/string.ln` -> `core::string`, `std/lib.ln` -> `std`.
fn std_module_path(rel: &str) -> String {
    let p = rel.trim_end_matches(".ln");
    let p = p.strip_suffix("/lib").or_else(|| p.strip_suffix("/mod")).unwrap_or(p);
    p.replace('/', "::")
}

/// The same answer from either compiler: std files as `lunar-std:` URIs,
/// integral floats as integers, and the diagnostics of a notification in a
/// fixed order.
fn normalize(v: &mut Value, std_prefix: &str) {
    match v {
        Value::String(s) => {
            let lower = s.to_lowercase();
            if let Some(rel) = lower.strip_prefix(std_prefix) {
                *s = format!("lunar-std:{}", std_module_path(&s[s.len() - rel.len()..]));
            }
        }
        Value::Number(n) => {
            if let Some(f) = n.as_f64()
                && f.fract() == 0.0
                && !n.is_i64()
                && !n.is_u64()
            {
                *v = json!(f as i64);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| normalize(x, std_prefix)),
        Value::Object(o) => {
            o.values_mut().for_each(|x| normalize(x, std_prefix));
            if let Some(Value::Array(ds)) = o.get_mut("diagnostics") {
                ds.sort_by_key(|d| d.to_string());
            }
        }
        _ => {}
    }
}

/// Responses by id, and the notifications in order (relative to each
/// other, not to the responses).
fn split(msgs: &[Value]) -> (BTreeMap<i64, Value>, Vec<Value>) {
    let mut responses = BTreeMap::new();
    let mut notes = Vec::new();
    for m in msgs {
        match m.get("id").and_then(Value::as_i64) {
            Some(id) => {
                responses.insert(id, m.clone());
            }
            None => notes.push(m.clone()),
        }
    }
    (responses, notes)
}

#[test]
fn rust_and_lunar_servers_answer_alike() {
    let std_prefix = format!("file:///{}/std/", root().to_string_lossy().replace('\\', "/").trim_start_matches("//?/").trim_start_matches('/')).to_lowercase();
    let rust = session(env!("CARGO_BIN_EXE_lunar").as_ref(), "rust");
    let (mut responses, notes) = split(&rust);
    // The session itself: the answers the Rust server gives.
    assert_eq!(responses.len(), 3 + PLACES.len() * 2 + 2 + 4 + 1, "{rust:#?}");
    assert!(notes.iter().any(|n| n.to_string().contains("name::unresolved")), "{notes:#?}");
    assert!(responses.values().any(|r| r["result"]["contents"]["value"].as_str().is_some_and(|t| t.contains("fn add(a: i32, b: i32): i32"))));
    let Some(bin) = std::env::var_os("LUNAR_BIN") else { return };

    let lunar = session(&bin, "lunar");
    let (mut lunar_responses, mut lunar_notes) = split(&lunar);
    let mut notes = notes;
    for v in responses.values_mut().chain(notes.iter_mut()).chain(lunar_responses.values_mut()).chain(lunar_notes.iter_mut()) {
        normalize(v, &std_prefix);
    }
    let mut diffs = Vec::new();
    for (id, r) in &responses {
        // The server built from craters/ can do more than the Rust one: it
        // must declare at least the Rust server's capabilities, alike.
        if *id == 1 {
            let (Some(rc), Some(lc)) = (r["result"]["capabilities"].as_object(), lunar_responses.get(id).and_then(|l| l["result"]["capabilities"].as_object())) else {
                diffs.push(format!("response {id}: no capabilities"));
                continue;
            };
            for (k, v) in rc {
                if lc.get(k) != Some(v) {
                    diffs.push(format!("capability {k}: rust {v}, lunar {:?}", lc.get(k)));
                }
            }
            continue;
        }
        match lunar_responses.get(id) {
            Some(l) if l == r => {}
            Some(l) => diffs.push(format!("response {id}:\n  rust:  {r}\n  lunar: {l}")),
            None => diffs.push(format!("response {id}: no answer from {}", bin.to_string_lossy())),
        }
    }
    if notes != lunar_notes {
        diffs.push(format!("notifications:\n  rust:  {}\n  lunar: {}", Value::from(notes.clone()), Value::from(lunar_notes.clone())));
    }
    let answered = responses.values().filter(|r| !r["result"].is_null()).count();
    eprintln!("compared {} responses ({answered} not null) and {} notifications", responses.len(), notes.len());
    assert!(diffs.is_empty(), "{} differences:\n{}", diffs.len(), diffs.join("\n"));
}
