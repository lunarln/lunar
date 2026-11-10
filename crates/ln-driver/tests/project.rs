//! Projects end to end: `lunar new`, path and git dependencies, the
//! lockfile, `lunar test`, `lunar update` and dependency visibility. Every
//! scenario runs in its own temporary directory, outside this repository.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The compiler under test: `$LUNAR_BIN` if set (such as the Lunar-built
/// `craters/target/release/lunar`), else this crate's `lunar`.
fn lunar_bin() -> std::ffi::OsString {
    std::env::var_os("LUNAR_BIN").unwrap_or_else(|| env!("CARGO_BIN_EXE_lunar").into())
}

fn lunar(dir: &Path, home: &Path, args: &[&str]) -> Output {
    Command::new(lunar_bin()).args(args).current_dir(dir).env("LUNAR_HOME", home).env_remove("LUNAR_STD_DIR").output().unwrap()
}

fn ok(dir: &Path, home: &Path, args: &[&str]) -> String {
    let o = lunar(dir, home, args);
    let all = String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr);
    assert!(o.status.success(), "`lunar {}` failed in {}:\n{all}", args.join(" "), dir.display());
    all
}

fn fails(dir: &Path, home: &Path, args: &[&str]) -> String {
    let o = lunar(dir, home, args);
    assert!(!o.status.success(), "`lunar {}` should fail in {}", args.join(" "), dir.display());
    String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr)
}

fn git(dir: &Path, args: &[&str]) {
    let o = Command::new("git").args(["-c", "user.email=test@example.com", "-c", "user.name=test"]).args(args).current_dir(dir).output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lunar-project-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn path_and_git_dependencies() {
    let w = scratch("deps");
    let home = w.join("home");
    ok(&w, &home, &["new", "app"]);
    ok(&w, &home, &["new", "mathx", "--lib"]);
    ok(&w, &home, &["new", "strutil", "--lib"]);

    // strutil: a git repository with a tag.
    let strutil = w.join("strutil");
    std::fs::write(strutil.join("src/lib.ln"), "pub fn shout(mut s: String): String {\n    s.push_str(\"!\");\n    return s;\n}\n").unwrap();
    git(&strutil, &["add", "-A"]);
    git(&strutil, &["commit", "-qm", "init"]);
    git(&strutil, &["tag", "v0.1.0"]);

    // mathx depends on strutil by git, and has an integration test.
    let mathx = w.join("mathx");
    std::fs::write(
        mathx.join("src/lib.ln"),
        "pub fn add(a: i32, b: i32): i32 {\n    return a + b;\n}\n\npub fn describe(a: i32, b: i32): String {\n    let mut s = String::from(\"sum is \");\n    s.write(\"{add(a, b)}\");\n    return strutil::shout(s);\n}\n\n#[test]\nfn adds() {\n    std::test::assert_eq(&add(2, 2), &4, \"2 + 2\");\n}\n",
    )
    .unwrap();
    std::fs::create_dir_all(mathx.join("tests")).unwrap();
    std::fs::write(mathx.join("tests/sums.ln"), "#[test]\nfn uses_lib() {\n    std::test::assert_eq(&mathx::add(1, 1), &2, \"1 + 1\");\n}\n").unwrap();
    let url = strutil.to_string_lossy().replace('\\', "/");
    ok(&mathx, &home, &["add", "strutil", "--git", &url, "--tag", "v0.1.0"]);
    let out = ok(&mathx, &home, &["test"]);
    assert!(out.contains("test result: ok. 2 passed"), "dependency tests must not run with the crater's own:\n{out}");

    // app depends on mathx by path.
    let app = w.join("app");
    std::fs::write(
        app.join("src/main.ln"),
        "use mathx::describe;\n\nfn main() {\n    println(\"{mathx::add(40, 2)}\");\n    let d = describe(1, 2);\n    println(\"{d}\");\n}\n",
    )
    .unwrap();
    ok(&app, &home, &["add", "mathx", "--path", "../mathx"]);
    let out = ok(&app, &home, &["run"]);
    assert!(out.contains("42\n") && out.contains("sum is 3!\n"), "{out}");
    let lock = std::fs::read_to_string(app.join("lunar.lock")).unwrap();
    assert!(lock.contains("source = \"path+../mathx\"") && lock.contains("?tag=v0.1.0#"), "{lock}");

    // The lock pins the commit: moving the tag changes nothing until `update`.
    std::fs::write(strutil.join("src/lib.ln"), "pub fn shout(mut s: String): String {\n    s.push_str(\"!!\");\n    return s;\n}\n").unwrap();
    git(&strutil, &["commit", "-qam", "louder"]);
    git(&strutil, &["tag", "-f", "v0.1.0"]);
    let out = ok(&app, &home, &["run"]);
    assert!(out.contains("sum is 3!\n") && !out.contains("Fetching"), "{out}");
    ok(&app, &home, &["update"]);
    let out = ok(&app, &home, &["run"]);
    assert!(out.contains("sum is 3!!\n"), "{out}");

    // A dependency's own dependencies are not visible.
    std::fs::write(app.join("src/main.ln"), "fn main() {\n    let s = strutil::shout(String::from(\"x\"));\n    println(\"{s}\");\n}\n").unwrap();
    let out = fails(&app, &home, &["build"]);
    assert!(out.contains("err(name::unresolved)"), "{out}");

    let _ = std::fs::remove_dir_all(&w);
}

#[test]
fn manifest_errors() {
    let w = scratch("errors");
    let home = w.join("home");
    ok(&w, &home, &["new", "app"]);
    ok(&w, &home, &["new", "lib2", "--lib"]);
    let app = w.join("app");
    let manifest = app.join("lunar.toml");

    // Version requirements are checked.
    std::fs::write(&manifest, "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nlib2 = { path = \"../lib2\", version = \"2\" }\n").unwrap();
    let out = fails(&app, &home, &["build"]);
    assert!(out.contains("requires `lib2` ^2") && out.contains("version 0.1.0"), "{out}");

    // The registry does not exist yet.
    std::fs::write(&manifest, "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\njson = \"1.2\"\n").unwrap();
    let out = fails(&app, &home, &["build"]);
    assert!(out.contains("registry, which is not available yet"), "{out}");

    // Unknown keys and bad names are reported.
    std::fs::write(&manifest, "[package]\nname = \"App\"\nversion = \"0.1.0\"\n").unwrap();
    let out = fails(&app, &home, &["build"]);
    assert!(out.contains("package name `App` is not valid"), "{out}");

    // Errors carry their code and point into the manifest.
    assert!(out.contains("err(manifest::bad_name)") && out.contains("lunar.toml:2"), "{out}");
    std::fs::write(&manifest, "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nmissing = { path = \"../missing\" }\n").unwrap();
    let out = fails(&app, &home, &["build"]);
    assert!(out.contains("err(manifest::not_found)") && out.contains("lunar.toml:6"), "{out}");
    std::fs::write(&manifest, "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nlib2 = { path = \"../lib2\", git = \"x\" }\n").unwrap();
    let out = fails(&app, &home, &["build"]);
    assert!(out.contains("err(manifest::bad_dependency)"), "{out}");
    std::fs::write(&manifest, "[package]\nname = \"app\"\nversion = \"one\"\n").unwrap();
    let out = fails(&app, &home, &["build"]);
    assert!(out.contains("err(manifest::bad_version)"), "{out}");
    std::fs::write(&manifest, "[package\nname = \"app\"\n").unwrap();
    let out = fails(&app, &home, &["build"]);
    assert!(out.contains("err(manifest::parse)"), "{out}");

    // A library has no program to run.
    let out = fails(&w.join("lib2"), &home, &["run"]);
    assert!(out.contains("err(manifest::no_target)"), "{out}");

    // Outside any project, commands without a file explain what they need.
    let out = fails(&w, &home, &["build"]);
    assert!(out.contains("no `lunar.toml`"), "{out}");

    let _ = std::fs::remove_dir_all(&w);
}

#[test]
fn workspaces() {
    let w = scratch("workspace");
    let home = w.join("home");
    let ws = w.join("ws");
    std::fs::create_dir_all(ws.join("crates")).unwrap();
    std::fs::write(ws.join("lunar.toml"), "[workspace]\nmembers = [\"crates/*\", \"tool\"]\n").unwrap();
    ok(&ws, &home, &["new", "tool"]);
    ok(&ws.join("crates"), &home, &["new", "core-util", "--lib"]);
    ok(&ws.join("crates"), &home, &["new", "app"]);
    std::fs::write(
        ws.join("crates/core-util/src/lib.ln"),
        "pub fn twice(x: i32): i32 {\n    return x * 2;\n}\n\n#[test]\nfn doubles() {\n    std::test::assert_eq(&twice(2), &4, \"twice\");\n}\n",
    )
    .unwrap();
    ok(&ws.join("crates/app"), &home, &["add", "core-util", "--path", "../core-util"]);
    std::fs::write(ws.join("crates/app/src/main.ln"), "fn main() {\n    println(\"app {core_util::twice(21)}\");\n}\n").unwrap();
    std::fs::write(ws.join("tool/src/main.ln"), "fn main() {\n    println(\"tool\");\n}\n").unwrap();

    // At the root, several members have programs: `run` needs `-p`.
    let out = fails(&ws, &home, &["run"]);
    assert!(out.contains("err(manifest::workspace)") && out.contains("-p"), "{out}");
    let out = ok(&ws, &home, &["run", "-p", "app"]);
    assert!(out.contains("app 42\n"), "{out}");
    let out = fails(&ws, &home, &["run", "-p", "nope"]);
    assert!(out.contains("err(manifest::workspace)"), "{out}");

    // In a member, commands act on that member; output and lock are shared.
    let out = ok(&ws.join("tool"), &home, &["run"]);
    assert!(out.contains("tool\n"), "{out}");
    ok(&ws, &home, &["build"]);
    let exe = if cfg!(windows) { ".exe" } else { "" };
    for name in ["app", "tool"] {
        assert!(ws.join(format!("target/debug/{name}{exe}")).is_file(), "{name} built at the workspace root");
    }
    // Members are not locked (they are part of the workspace), so path-only
    // workspaces have no lockfile at all, and members never get their own.
    assert!(!ws.join("lunar.lock").exists() && !ws.join("crates/app/lunar.lock").exists());
    assert!(!ws.join("crates/app/target").exists());

    // `test` at the root tests every member.
    let out = ok(&ws, &home, &["test"]);
    assert!(out.contains("doubles"), "{out}");

    // Member names must be unique.
    std::fs::write(ws.join("tool/lunar.toml"), "[package]\nname = \"app\"\nversion = \"0.1.0\"\n").unwrap();
    let out = fails(&ws, &home, &["build"]);
    assert!(out.contains("err(manifest::workspace)"), "{out}");
    std::fs::write(ws.join("tool/lunar.toml"), "[package]\nname = \"tool\"\nversion = \"0.1.0\"\n").unwrap();

    // A listed member must exist, and the error points at it.
    std::fs::write(ws.join("lunar.toml"), "[workspace]\nmembers = [\"crates/*\", \"tool\", \"gone\"]\n").unwrap();
    let out = fails(&ws.join("tool"), &home, &["build"]);
    assert!(out.contains("err(manifest::not_found)") && out.contains("`gone`"), "{out}");

    let _ = std::fs::remove_dir_all(&w);
}

/// A minimal language server client over stdio.
struct Lsp {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    stdout: std::io::BufReader<std::process::ChildStdout>,
    next_id: i64,
}

impl Lsp {
    fn start(home: &Path) -> Lsp {
        use std::process::Stdio;
        let mut child = Command::new(lunar_bin())
            .args(["lsp", "--stdio"])
            .env("LUNAR_HOME", home)
            .env_remove("LUNAR_STD_DIR")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut lsp = Lsp { child, stdin, stdout, next_id: 1 };
        lsp.request("initialize", serde_json::json!({ "processId": null, "rootUri": null, "capabilities": {} }));
        lsp.notify("initialized", serde_json::json!({}));
        lsp
    }

    fn send(&mut self, msg: serde_json::Value) {
        use std::io::Write;
        let body = msg.to_string();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        self.stdin.flush().unwrap();
    }

    fn recv(&mut self) -> serde_json::Value {
        use std::io::{BufRead, Read};
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

    fn notify(&mut self, method: &str, params: serde_json::Value) {
        self.send(serde_json::json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    /// Send a request and wait for its result. Diagnostics published
    /// meanwhile are collected into `diags` (by URI, latest wins).
    fn request_collecting(&mut self, method: &str, params: serde_json::Value, diags: &mut Vec<(String, serde_json::Value)>) -> serde_json::Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let m = self.recv();
            if m["method"] == "textDocument/publishDiagnostics" {
                let uri = m["params"]["uri"].as_str().unwrap_or_default().to_string();
                diags.retain(|(u, _)| *u != uri);
                diags.push((uri, m["params"]["diagnostics"].clone()));
            }
            if m.get("id") == Some(&serde_json::json!(id)) {
                return m["result"].clone();
            }
        }
    }

    fn request(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        self.request_collecting(method, params, &mut Vec::new())
    }

    fn open(&mut self, uri: &str, text: &str) {
        self.notify("textDocument/didOpen", serde_json::json!({ "textDocument": { "uri": uri, "languageId": "lunar", "version": 1, "text": text } }));
    }

    fn stop(mut self) {
        self.request("shutdown", serde_json::Value::Null);
        self.notify("exit", serde_json::Value::Null);
        assert!(self.child.wait().unwrap().success());
    }
}

fn file_uri(p: &Path) -> String {
    let path = p.to_string_lossy().replace('\\', "/");
    if path.starts_with('/') { format!("file://{path}") } else { format!("file:///{path}") }
}

/// The language server analyzes a project's files with their dependencies.
#[test]
fn language_server_sees_dependencies() {
    use serde_json::json;

    let w = scratch("lsp");
    let home = w.join("home");
    ok(&w, &home, &["new", "app"]);
    ok(&w, &home, &["new", "mathx", "--lib"]);
    let app = w.join("app");
    ok(&app, &home, &["add", "mathx", "--path", "../mathx"]);
    let main = app.join("src").join("main.ln");
    let text = "fn main() {\n    println(\"{mathx::add(40, 2)}\");\n}\n";
    std::fs::write(&main, text).unwrap();

    let mut lsp = Lsp::start(&home);
    let uri = file_uri(&main);
    lsp.open(&uri, text);
    // `add` in `mathx::add(40, 2)`, line 1.
    let mut diags = Vec::new();
    let definition =
        lsp.request_collecting("textDocument/definition", json!({ "textDocument": { "uri": uri }, "position": { "line": 1, "character": 23 } }), &mut diags);
    let main_diags = diags.iter().find(|(u, _)| u.ends_with("main.ln")).map(|(_, d)| d.clone()).expect("diagnostics for main.ln");
    assert_eq!(main_diags, json!([]), "the dependency should resolve");
    let target = definition["uri"].as_str().unwrap_or_default();
    assert!(target.ends_with("mathx/src/lib.ln"), "definition of mathx::add: {definition:#?}");
    lsp.stop();
    let _ = std::fs::remove_dir_all(&w);
}

/// Files of a standard library source tree (this repository's `std/`) are
/// analyzed as the standard library, from disk and open buffers, not as
/// programs of their own.
#[test]
fn language_server_edits_std() {
    use serde_json::json;

    let w = scratch("lsp-std");
    let home = w.join("home");
    // A copy of the std tree, so edits stay out of the repository.
    let std_root = w.join("std");
    copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../std"), &std_root);
    let str_ln = std_root.join("core").join("str.ln");
    let text = std::fs::read_to_string(&str_ln).unwrap();

    let mut lsp = Lsp::start(&home);
    let uri = file_uri(&str_ln);
    lsp.open(&uri, &text);
    let mut diags = Vec::new();
    // Hover on `len` in `pub fn len(self)`, to sync with the analysis.
    let line = text.lines().position(|l| l.contains("pub fn len(self)")).unwrap();
    let hover =
        lsp.request_collecting("textDocument/hover", json!({ "textDocument": { "uri": uri }, "position": { "line": line, "character": 12 } }), &mut diags);
    let errors: Vec<_> = diags.iter().filter(|(_, d)| d.as_array().is_some_and(|a| !a.is_empty())).collect();
    assert!(errors.is_empty(), "the unmodified std has no errors: {errors:#?}");
    assert!(hover.to_string().contains("len"), "hover in std: {hover}");

    // An edit shows up as an error in the edited file.
    let broken = text.replacen("return intrinsics::str_len(self);", "return intrinsics::str_len(self) + true;", 1);
    assert_ne!(broken, text);
    lsp.notify("textDocument/didChange", json!({ "textDocument": { "uri": uri, "version": 2 }, "contentChanges": [{ "text": broken }] }));
    let mut diags = Vec::new();
    lsp.request_collecting("textDocument/hover", json!({ "textDocument": { "uri": uri }, "position": { "line": line, "character": 12 } }), &mut diags);
    let mine = diags.iter().find(|(u, _)| *u == uri).map(|(_, d)| d.clone()).expect("diagnostics for str.ln");
    assert!(mine.as_array().is_some_and(|a| !a.is_empty()), "the edit is an error: {diags:#?}");
    lsp.stop();
    let _ = std::fs::remove_dir_all(&w);
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(e.file_name()));
        } else {
            std::fs::copy(&p, to.join(e.file_name())).unwrap();
        }
    }
}
