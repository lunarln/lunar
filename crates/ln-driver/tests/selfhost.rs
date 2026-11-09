//! The compiler written in Lunar (`craters/`) against the Rust one: built
//! with `lunar`, run over every `.ln` file in the repository, and compared
//! stage by stage. For now that is the lexer.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// Every `.ln` file under `dir`, relative to the repository root.
fn ln_files(root: &Path, dir: &Path, out: &mut Vec<String>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            ln_files(root, &p, out);
        } else if p.extension().is_some_and(|x| x == "ln") {
            out.push(p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"));
        }
    }
}

/// `ln_lexer`'s tokens and errors in the format of `ln_lexer::dump` in
/// craters/ln-lexer.
fn rust_dump(src: &str) -> String {
    let (toks, errs) = ln_lexer::lex(src);
    let mut out = String::new();
    for t in toks {
        out.push_str(&format!("{:?} {} {}\n", t.token, t.range.start, t.range.end));
    }
    for e in errs {
        out.push_str(&format!("error {:?} {} {}\n", e.kind, e.range.start, e.range.end));
    }
    out
}

#[test]
fn lexer_matches_rust_lexer() {
    let root = root();
    let craters = root.join("craters");
    let home = std::env::temp_dir().join(format!("lunar-selfhost-home-{}", std::process::id()));
    let build = Command::new(env!("CARGO_BIN_EXE_lunar"))
        .args(["build", "--release"])
        .current_dir(&craters)
        .env("LUNAR_HOME", &home)
        .env_remove("LUNAR_STD_DIR")
        .output()
        .unwrap();
    assert!(build.status.success(), "building craters/:\n{}", String::from_utf8_lossy(&build.stderr));
    let exe = craters.join("target/release").join(format!("ln-driver{}", std::env::consts::EXE_SUFFIX));

    let mut files = Vec::new();
    for dir in ["std", "tests", "examples", "craters"] {
        ln_files(&root, &root.join(dir), &mut files);
    }
    files.sort();
    assert!(files.len() > 50, "found only {} files", files.len());

    let run = Command::new(&exe).args(&files).current_dir(&root).output().unwrap();
    assert!(run.status.success(), "ln-driver failed:\n{}", String::from_utf8_lossy(&run.stderr));
    let stdout = String::from_utf8(run.stdout).unwrap();
    let mut lunar: BTreeMap<&str, String> = BTreeMap::new();
    let mut current = None;
    for line in stdout.lines() {
        if let Some(path) = line.strip_prefix("== ") {
            current = Some(path);
            lunar.insert(path, String::new());
        } else if let Some(path) = current {
            let dump = lunar.get_mut(path).unwrap();
            dump.push_str(line);
            dump.push('\n');
        }
    }

    let mut mismatches = Vec::new();
    for f in &files {
        let src = std::fs::read_to_string(root.join(f)).unwrap();
        let expected = rust_dump(&src);
        let got = lunar.get(f.as_str()).map(String::as_str).unwrap_or("<missing>");
        if got != expected {
            // The first differing line, with the source text around it.
            let (i, (e, g)) = expected.lines().zip(got.lines()).enumerate().find(|(_, (e, g))| e != g).unwrap_or((
                expected.lines().count().min(got.lines().count()),
                (expected.lines().nth(got.lines().count()).unwrap_or("<end>"), got.lines().nth(expected.lines().count()).unwrap_or("<end>")),
            ));
            let at: usize = e.split(' ').nth_back(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            let context: String = src.get(at.saturating_sub(10)..(at + 20).min(src.len())).unwrap_or("").into();
            mismatches.push(format!("{f}, token {i}: Rust `{e}`, Lunar `{g}` near {context:?}"));
        }
    }
    assert!(mismatches.is_empty(), "{} of {} files lex differently:\n{}", mismatches.len(), files.len(), mismatches.join("\n"));
}
