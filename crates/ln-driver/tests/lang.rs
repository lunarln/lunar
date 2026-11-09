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
