//! End-to-end language tests driven by files in the repository:
//!
//! * `tests/ui/*.ln`: must fail to compile; every `// error: scope::name`
//!   line must appear as `err(scope::name)` in the diagnostics.
//! * `tests/run/*.ln` and `examples/*.ln`: must compile and run; if a
//!   sibling `.out` file exists, stdout must match it exactly.
//! * Files with `#[test]` functions are also run through `lunar test`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn lunar() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_lunar"));
    c.env_remove("LUNAR_STD_DIR");
    c
}

fn ln_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "ln")).collect();
    v.sort();
    v
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for d in chars.by_ref() {
                if d.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[test]
fn ui_tests_report_expected_errors() {
    let mut failures = Vec::new();
    for f in ln_files(&root().join("tests/ui")) {
        let src = std::fs::read_to_string(&f).unwrap();
        let expected: Vec<&str> = src.lines().filter_map(|l| l.trim().strip_prefix("// error:")).map(str::trim).collect();
        assert!(!expected.is_empty(), "{} declares no `// error:` lines", f.display());
        let out = lunar().arg("check").arg(&f).output().unwrap();
        let stderr = strip_ansi(&String::from_utf8_lossy(&out.stderr));
        if out.status.success() {
            failures.push(format!("{}: compiled successfully but should fail", f.display()));
            continue;
        }
        for code in expected {
            if !stderr.contains(&format!("err({code})")) {
                failures.push(format!("{}: expected err({code}), got:\n{stderr}", f.display()));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

fn run_dir(dir: &str) {
    let mut failures = Vec::new();
    for f in ln_files(&root().join(dir)) {
        let src = std::fs::read_to_string(&f).unwrap();
        if src.contains("fn main()") {
            let out = lunar().arg("run").arg(&f).output().unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n");
            if !out.status.success() {
                failures.push(format!(
                    "{}: exited with {:?}\nstdout:\n{stdout}\nstderr:\n{}",
                    f.display(),
                    out.status.code(),
                    strip_ansi(&String::from_utf8_lossy(&out.stderr))
                ));
                continue;
            }
            let expect_path = f.with_extension("out");
            if let Ok(expected) = std::fs::read_to_string(&expect_path) {
                let expected = expected.replace("\r\n", "\n");
                if stdout != expected {
                    failures.push(format!("{}: output mismatch\n--- expected\n{expected}\n--- got\n{stdout}", f.display()));
                }
            }
        }
        if src.contains("#[test]") {
            let out = lunar().arg("test").arg(&f).output().unwrap();
            if !out.status.success() {
                failures.push(format!(
                    "{}: `lunar test` failed\n{}{}",
                    f.display(),
                    String::from_utf8_lossy(&out.stdout),
                    strip_ansi(&String::from_utf8_lossy(&out.stderr))
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn run_tests_produce_expected_output() {
    run_dir("tests/run");
}

#[test]
fn examples_run() {
    run_dir("examples");
}

#[test]
fn emit_llvm_prints_ir() {
    for release in [false, true] {
        let mut cmd = lunar();
        cmd.arg("emit-llvm").arg(root().join("examples/hello.ln"));
        if release {
            cmd.arg("--release");
        }
        let out = cmd.output().unwrap();
        assert!(out.status.success(), "emit-llvm failed:\n{}", String::from_utf8_lossy(&out.stderr));
        let ir = String::from_utf8(out.stdout).unwrap();
        assert!(ir.contains("define") && ir.contains("@main"), "unexpected IR (release={release}):\n{ir}");
    }
}

/// `tests/fatal/*.ln`: must compile, then exit with a failure status (not a
/// crash) printing the `// fatal:` message, in debug and release builds.
#[test]
fn fatal_tests_fail_cleanly() {
    let mut failures = Vec::new();
    for f in ln_files(&root().join("tests/fatal")) {
        let src = std::fs::read_to_string(&f).unwrap();
        let expected = src.lines().find_map(|l| l.trim().strip_prefix("// fatal:")).map(str::trim).expect("missing `// fatal:` line");
        for release in [false, true] {
            let mut cmd = lunar();
            cmd.arg("run").arg(&f);
            if release {
                cmd.arg("--release");
            }
            let out = cmd.output().unwrap();
            let stderr = strip_ansi(&String::from_utf8_lossy(&out.stderr));
            if out.status.code() != Some(134) || !stderr.contains(expected) {
                failures.push(format!("{} (release={release}): expected exit 134 and `{expected}`, got {:?}:\n{stderr}", f.display(), out.status.code()));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

mod lsp {
    use super::lunar;
    use serde_json::{Value, json};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::process::{ChildStdin, ChildStdout, Stdio};

    struct Client {
        stdin: ChildStdin,
        stdout: BufReader<ChildStdout>,
        next_id: i64,
        /// Notifications received while waiting for responses.
        notes: Vec<Value>,
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
                self.stdout.read_line(&mut line).unwrap();
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
                if m.get("id") == Some(&json!(id)) {
                    return m["result"].clone();
                }
                self.notes.push(m);
            }
        }

        fn notify(&mut self, method: &str, params: Value) {
            self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }));
        }

        /// Wait for diagnostics published for `uri`.
        fn diagnostics_for(&mut self, uri: &str) -> Vec<Value> {
            loop {
                if let Some(i) = self.notes.iter().position(|n| n["method"] == "textDocument/publishDiagnostics" && n["params"]["uri"] == uri) {
                    let n = self.notes.remove(i);
                    return n["params"]["diagnostics"].as_array().unwrap().clone();
                }
                let m = self.recv();
                self.notes.push(m);
            }
        }
    }

    fn uri_of(path: &std::path::Path) -> String {
        let s = path.to_string_lossy().replace('\\', "/");
        let s = s.trim_start_matches("//?/").to_string();
        if s.starts_with('/') { format!("file://{s}") } else { format!("file:///{s}") }
    }

    #[test]
    fn diagnostics_hover_and_definition() {
        let dir = std::env::temp_dir().join(format!("lunar-lsp-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("main.ln");
        let src = "struct Stack {\n    items: [i32; -..+],\n}\n\n/// Adds two numbers.\nfn add(a: i32, b: i32): i32 {\n    return a + b;\n}\n\nfn main() {\n    let total = add(1, 2);\n    let s = Stack { items: [] };\n    let n = s.items.len();\n    println(\"{total} {n}\");\n    let oops: i32 = undefined_name;\n}\n";
        std::fs::write(&file, "").unwrap(); // the editor buffer, not the disk, must be analyzed
        let uri = uri_of(&file);

        let mut child = lunar().arg("lsp").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        let mut c = Client { stdin: child.stdin.take().unwrap(), stdout: BufReader::new(child.stdout.take().unwrap()), next_id: 0, notes: Vec::new() };
        let init = c.request("initialize", json!({ "processId": null, "rootUri": null, "capabilities": {} }));
        assert_eq!(init["capabilities"]["hoverProvider"], json!(true));
        c.notify("initialized", json!({}));
        c.notify("textDocument/didOpen", json!({ "textDocument": { "uri": uri, "languageId": "lunar", "version": 1, "text": src } }));

        let diags = c.diagnostics_for(&uri);
        assert!(diags.iter().any(|d| d["code"] == "name::unresolved"), "expected an unresolved-name error, got {diags:#?}");

        let pos = |line: u32, col: u32| json!({ "textDocument": { "uri": uri }, "position": { "line": line, "character": col } });
        // `add` in `let total = add(1, 2);` (line 10).
        let h = c.request("textDocument/hover", pos(10, 17));
        let text = h["contents"]["value"].as_str().unwrap_or_default().to_string();
        assert!(text.contains("fn add(a: i32, b: i32): i32") && text.contains("Adds two numbers."), "hover on fn: {h:#?}");
        // `total` declaration.
        let h = c.request("textDocument/hover", pos(10, 9));
        assert!(h["contents"]["value"].as_str().unwrap_or_default().contains("let total: i32"), "hover on local: {h:#?}");
        // `items` field access (line 12).
        let h = c.request("textDocument/hover", pos(12, 15));
        assert!(h["contents"]["value"].as_str().unwrap_or_default().contains("items: [i32; -..+]"), "hover on field: {h:#?}");

        // Go to `add`'s definition (line 5), and into the standard library.
        let d = c.request("textDocument/definition", pos(10, 17));
        assert_eq!(d["range"]["start"]["line"], json!(5), "definition of add: {d:#?}");
        let d = c.request("textDocument/definition", pos(13, 6));
        let target = d["uri"].as_str().unwrap_or_default();
        assert!(target.contains("lunar-std-") && target.ends_with("io.ln"), "definition of println: {d:#?}");

        // Fixing the error clears it.
        let fixed = src.replace("undefined_name", "n as i32");
        c.notify("textDocument/didChange", json!({ "textDocument": { "uri": uri, "version": 2 }, "contentChanges": [{ "text": fixed }] }));
        let diags = c.diagnostics_for(&uri);
        assert!(!diags.iter().any(|d| d["severity"] == json!(1)), "errors remain after the fix: {diags:#?}");

        let _ = c.request("shutdown", Value::Null);
        c.notify("exit", Value::Null);
        assert!(child.wait().unwrap().success());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
