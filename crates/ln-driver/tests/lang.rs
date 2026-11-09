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
