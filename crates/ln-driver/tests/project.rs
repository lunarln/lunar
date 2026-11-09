//! Projects end to end: `lunar new`, path and git dependencies, the
//! lockfile, `lunar test`, `lunar update` and dependency visibility. Every
//! scenario runs in its own temporary directory, outside this repository.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn lunar(dir: &Path, home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_lunar")).args(args).current_dir(dir).env("LUNAR_HOME", home).env_remove("LUNAR_STD_DIR").output().unwrap()
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
        "pub fn add(a: i32, b: i32): i32 {\n    return a + b;\n}\n\npub fn describe(a: i32, b: i32): String {\n    let mut s = String::from(\"sum is \");\n    s.write(\"{add(a, b)}\");\n    return strutil::shout(s);\n}\n\n#[test]\nfn adds() {\n    std::test::assert_eq(add(2, 2), 4, \"2 + 2\");\n}\n",
    )
    .unwrap();
    std::fs::create_dir_all(mathx.join("tests")).unwrap();
    std::fs::write(mathx.join("tests/sums.ln"), "#[test]\nfn uses_lib() {\n    std::test::assert_eq(mathx::add(1, 1), 2, \"1 + 1\");\n}\n").unwrap();
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
        "pub fn twice(x: i32): i32 {\n    return x * 2;\n}\n\n#[test]\nfn doubles() {\n    std::test::assert_eq(twice(2), 4, \"twice\");\n}\n",
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

/// The language server analyzes a project's files with their dependencies.
#[test]
fn language_server_sees_dependencies() {
    use serde_json::{Value, json};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::process::Stdio;

    let w = scratch("lsp");
    let home = w.join("home");
    ok(&w, &home, &["new", "app"]);
    ok(&w, &home, &["new", "mathx", "--lib"]);
    let app = w.join("app");
    ok(&app, &home, &["add", "mathx", "--path", "../mathx"]);
    let main = app.join("src").join("main.ln");
    let text = "fn main() {\n    println(\"{mathx::add(40, 2)}\");\n}\n";
    std::fs::write(&main, text).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_lunar"))
        .args(["lsp", "--stdio"])
        .env("LUNAR_HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut send = |msg: Value| {
        let body = msg.to_string();
        write!(stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        stdin.flush().unwrap();
    };
    let mut recv = || -> Value {
        let mut len = 0;
        loop {
            let mut line = String::new();
            stdout.read_line(&mut line).unwrap();
            let line = line.trim();
            if line.is_empty() {
                break;
            }
            if let Some(n) = line.strip_prefix("Content-Length: ") {
                len = n.parse().unwrap();
            }
        }
        let mut buf = vec![0; len];
        stdout.read_exact(&mut buf).unwrap();
        serde_json::from_slice(&buf).unwrap()
    };
    let path = main.to_string_lossy().replace('\\', "/");
    let uri = if path.starts_with('/') { format!("file://{path}") } else { format!("file:///{path}") };

    send(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "processId": null, "rootUri": null, "capabilities": {} } }));
    while recv().get("id") != Some(&json!(1)) {}
    send(json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }));
    send(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri, "languageId": "lunar", "version": 1, "text": text } } }),
    );
    // `add` in `mathx::add(40, 2)`, line 1.
    send(
        json!({ "jsonrpc": "2.0", "id": 2, "method": "textDocument/definition", "params": { "textDocument": { "uri": uri }, "position": { "line": 1, "character": 23 } } }),
    );
    let mut diags = None;
    let definition = loop {
        let m = recv();
        if m["method"] == "textDocument/publishDiagnostics" && m["params"]["uri"].as_str().is_some_and(|u| u.ends_with("main.ln")) {
            diags = Some(m["params"]["diagnostics"].clone());
        }
        if m.get("id") == Some(&json!(2)) {
            break m["result"].clone();
        }
    };
    let diags = diags.expect("diagnostics for main.ln");
    assert_eq!(diags, json!([]), "the dependency should resolve");
    let target = definition["uri"].as_str().unwrap_or_default();
    assert!(target.ends_with("mathx/src/lib.ln"), "definition of mathx::add: {definition:#?}");

    send(json!({ "jsonrpc": "2.0", "id": 3, "method": "shutdown", "params": null }));
    while recv().get("id") != Some(&json!(3)) {}
    send(json!({ "jsonrpc": "2.0", "method": "exit", "params": null }));
    assert!(child.wait().unwrap().success());
    let _ = std::fs::remove_dir_all(&w);
}
