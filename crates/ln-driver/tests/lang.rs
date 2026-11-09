//! End-to-end language tests driven by files in the repository:
//!
//! * `tests/ui/*.ln`: every `// error: scope::name` line must appear as
//!   `err(scope::name)` in the diagnostics, and every `// warning:
//!   scope::name` line as `warn(scope::name)`. A file with `// error:`
//!   lines must fail to compile; one with only `// warning:` lines must
//!   compile. `// absent: scope::name` must not be reported at all, and
//!   `// once: scope::name` exactly once. A `// command: build` line runs
//!   `lunar build` instead of `lunar check` (for errors only a build
//!   reports, such as a missing `main`).
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
        let header = |prefix: &str| -> Vec<String> { src.lines().filter_map(|l| l.trim().strip_prefix(prefix)).map(|c| c.trim().to_string()).collect() };
        let errors = header("// error:");
        let warnings = header("// warning:");
        assert!(!errors.is_empty() || !warnings.is_empty(), "{} declares no `// error:` or `// warning:` lines", f.display());
        let mut cmd = lunar();
        if header("// command:").iter().any(|c| c == "build") {
            let exe =
                PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("ui-{}{}", f.file_stem().unwrap().to_string_lossy(), std::env::consts::EXE_SUFFIX));
            cmd.arg("build").arg(&f).arg("-o").arg(exe);
        } else {
            cmd.arg("check").arg(&f);
        }
        let out = cmd.output().unwrap();
        let stderr = strip_ansi(&String::from_utf8_lossy(&out.stderr));
        match (errors.is_empty(), out.status.success()) {
            (false, true) => failures.push(format!("{}: compiled successfully but should fail", f.display())),
            (true, false) => failures.push(format!("{}: should compile with warnings only, but failed:\n{stderr}", f.display())),
            _ => {}
        }
        for (kind, codes) in [("err", &errors), ("warn", &warnings)] {
            for code in codes {
                if !stderr.contains(&format!("{kind}({code})")) {
                    failures.push(format!("{}: expected {kind}({code}), got:\n{stderr}", f.display()));
                }
            }
        }
        for code in header("// absent:") {
            if stderr.contains(&format!("({code})")) {
                failures.push(format!("{}: {code} should not be reported, got:\n{stderr}", f.display()));
            }
        }
        for (code, n) in header("// once:").iter().map(|c| (c, stderr.matches(&format!("({c})")).count())) {
            if n != 1 {
                failures.push(format!("{}: {code} should be reported exactly once, got {n}:\n{stderr}", f.display()));
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

        // `--stdio` is what vscode-languageclient appends.
        let mut child = lunar().args(["lsp", "--stdio"]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
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
        // The name in a declaration (line 5: `fn add`).
        let h = c.request("textDocument/hover", pos(5, 4));
        assert!(h["contents"]["value"].as_str().unwrap_or_default().contains("fn add("), "hover on declaration: {h:#?}");
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

/// Every code in `ln_diag::ALL_CODES` has a `docs/errors/<scope>/<name>.md`
/// starting with `# scope::name: title`, and every such file names a code.
#[test]
fn every_code_has_an_explanation() {
    let mut failures = Vec::new();
    let codes: Vec<String> = ln_diag::ALL_CODES.iter().map(|c| c.to_string()).collect();
    for code in &codes {
        let (scope, name) = code.split_once("::").unwrap();
        let path = root().join("docs/errors").join(scope).join(format!("{name}.md"));
        match std::fs::read_to_string(&path) {
            Ok(md) => {
                let first = md.lines().next().unwrap_or("");
                if !first.strip_prefix(&format!("# {code}: ")).is_some_and(|t| !t.trim().is_empty()) {
                    failures.push(format!("{}: first line must be `# {code}: <title>`, got `{first}`", path.display()));
                }
                if md.contains('\r') {
                    failures.push(format!("{}: must use LF line endings", path.display()));
                }
            }
            Err(_) => failures.push(format!("{code}: missing {}", path.display())),
        }
    }
    for scope in std::fs::read_dir(root().join("docs/errors")).unwrap().flatten() {
        for f in std::fs::read_dir(scope.path()).unwrap().flatten() {
            let code = format!("{}::{}", scope.file_name().to_string_lossy(), f.path().file_stem().unwrap().to_string_lossy());
            if !codes.contains(&code) {
                failures.push(format!("{}: `{code}` is not a code in ln_diag::ALL_CODES", f.path().display()));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The first code block under `heading` in an explanation, as (info string,
/// source).
fn example(md: &str, heading: &str) -> Option<(String, String)> {
    let start = md.find(&format!("\n{heading}\n"))? + heading.len() + 2;
    let section = &md[start..];
    let section = &section[..section.find("\n## ").unwrap_or(section.len())];
    let mut lines = section.lines().skip_while(|l| !l.starts_with("```"));
    let info = lines.next()?.trim_start_matches('`').trim().to_string();
    let src: Vec<&str> = lines.take_while(|l| !l.starts_with("```")).collect();
    Some((info, src.join("\n") + "\n"))
}

/// The `err(...)` / `warn(...)` codes in compiler output.
fn reported_codes(stderr: &str) -> Vec<String> {
    let mut codes: Vec<String> = Vec::new();
    for open in ["err(", "warn("] {
        for (i, _) in stderr.match_indices(open) {
            let Some((code, _)) = stderr[i + open.len()..].split_once(')') else { continue };
            let part = |p: &str| !p.is_empty() && p.chars().all(|c| c.is_ascii_lowercase() || c == '_');
            if code.split_once("::").is_some_and(|(s, n)| part(s) && part(n)) && !codes.iter().any(|c| c == code) {
                codes.push(code.to_string());
            }
        }
    }
    codes
}

/// For every explanation, the erroneous example reports exactly its code and
/// the fixed example checks without any diagnostics. Examples marked
/// ```lunar,ignore are skipped.
#[test]
fn explanation_examples_check() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("explain");
    std::fs::create_dir_all(&dir).unwrap();
    let mut failures = Vec::new();
    for code in ln_diag::ALL_CODES.iter().map(|c| c.to_string()) {
        let (scope, name) = code.split_once("::").unwrap();
        let Ok(md) = std::fs::read_to_string(root().join("docs/errors").join(scope).join(format!("{name}.md"))) else {
            continue; // reported by `every_code_has_an_explanation`
        };
        for (heading, bad) in [("## Erroneous example", true), ("## Fixed", false)] {
            let Some((info, src)) = example(&md, heading) else { continue };
            match info.as_str() {
                "lunar,ignore" => continue,
                "lunar" => {}
                other => {
                    failures.push(format!("{code}: `{heading}` block must be ```lunar or ```lunar,ignore, not ```{other}"));
                    continue;
                }
            }
            let file = dir.join(format!("{scope}_{name}_{}.ln", if bad { "bad" } else { "fixed" }));
            std::fs::write(&file, src).unwrap();
            let out = lunar().arg("check").arg(&file).output().unwrap();
            let stderr = strip_ansi(&String::from_utf8_lossy(&out.stderr));
            let reported = reported_codes(&stderr);
            if bad && reported != [code.clone()] {
                failures.push(format!("{code}: erroneous example should report exactly {code}, reported {reported:?}:\n{stderr}"));
            }
            if !bad && (!out.status.success() || !reported.is_empty()) {
                failures.push(format!("{code}: fixed example should check cleanly, reported {reported:?}:\n{stderr}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn explain_prints_explanations() {
    let expected = std::fs::read_to_string(root().join("docs/errors/borrow/shrink_while_borrowed.md")).unwrap();
    for arg in ["borrow::shrink_while_borrowed", "err(borrow::shrink_while_borrowed)"] {
        let out = lunar().args(["explain", arg]).output().unwrap();
        assert!(out.status.success(), "`lunar explain {arg}` failed:\n{}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(String::from_utf8_lossy(&out.stdout), expected, "`lunar explain {arg}`");
    }

    let out = lunar().args(["explain", "borrow::shrink_while_borowed"]).output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "unknown code should exit 1");
    assert!(stderr.contains("did you mean `borrow::shrink_while_borrowed`?"), "no suggestion:\n{stderr}");

    for args in [&["explain"][..], &["explain", "--list"]] {
        let out = lunar().args(args).output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{args:?} failed");
        assert_eq!(stdout.lines().count(), ln_diag::ALL_CODES.len(), "{args:?} should list every code:\n{stdout}");
        assert!(stdout.lines().any(|l| l.starts_with("lint::unused_mut ")), "{args:?}:\n{stdout}");
    }
}

#[test]
fn failed_check_suggests_explain() {
    let out = lunar().arg("check").arg(root().join("tests/ui/shrink_while_borrowed.ln")).output().unwrap();
    let stderr = strip_ansi(&String::from_utf8_lossy(&out.stderr));
    assert!(
        stderr.trim_end().ends_with("For more information about this error, try `lunar explain borrow::shrink_while_borrowed`."),
        "missing hint:\n{stderr}"
    );
}
