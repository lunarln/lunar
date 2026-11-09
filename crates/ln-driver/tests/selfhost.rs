//! The compiler written in Lunar (`craters/`) against the Rust one: built
//! with `lunar`, run over every `.ln` file in the repository, and compared
//! stage by stage: the lexer's tokens, then the parser's syntax trees and
//! syntax errors.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

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

/// Every program in the repository.
fn all_files(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    for dir in ["std", "tests", "examples", "craters"] {
        ln_files(root, &root.join(dir), &mut files);
    }
    files.sort();
    assert!(files.len() > 50, "found only {} files", files.len());
    files
}

/// `craters/` built once (with this checkout's `lunar`), for every test.
fn driver() -> &'static Path {
    static EXE: OnceLock<PathBuf> = OnceLock::new();
    EXE.get_or_init(|| {
        let craters = root().join("craters");
        let home = std::env::temp_dir().join(format!("lunar-selfhost-home-{}", std::process::id()));
        let build = Command::new(env!("CARGO_BIN_EXE_lunar"))
            .args(["build", "--release"])
            .current_dir(&craters)
            .env("LUNAR_HOME", &home)
            .env_remove("LUNAR_STD_DIR")
            .output()
            .unwrap();
        assert!(build.status.success(), "building craters/:\n{}{}", String::from_utf8_lossy(&build.stdout), String::from_utf8_lossy(&build.stderr));
        craters.join("target/release").join(format!("ln-driver{}", std::env::consts::EXE_SUFFIX))
    })
}

/// Run the Lunar driver over every program and compare each file's output
/// with `rust`'s for that file.
fn compare(args: &[&str], what: &str, rust: impl Fn(&str) -> String) {
    let root = root();
    let files = all_files(&root);
    compare_files(&root, &files, args, what, rust);
}

/// Run the Lunar driver over `files` (relative to `root`) and compare each
/// file's output with `rust`'s for that file.
fn compare_files(root: &Path, files: &[String], args: &[&str], what: &str, rust: impl Fn(&str) -> String) {
    let run = Command::new(driver()).args(args).args(files).current_dir(root).output().unwrap();
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
    for f in files {
        let src = std::fs::read_to_string(root.join(f)).unwrap();
        let expected = rust(&src);
        let got = lunar.get(f.as_str()).map(String::as_str).unwrap_or("<missing>");
        if got != expected {
            // The first differing line, with the source text around the
            // last offset mentioned on it.
            let (i, (e, g)) = expected.lines().zip(got.lines()).enumerate().find(|(_, (e, g))| e != g).unwrap_or((
                expected.lines().count().min(got.lines().count()),
                (expected.lines().nth(got.lines().count()).unwrap_or("<end>"), got.lines().nth(expected.lines().count()).unwrap_or("<end>")),
            ));
            let at: usize = e.rsplit(['@', '.', ' ']).find_map(|s| s.parse().ok()).unwrap_or(0);
            let context: String = src.get(at.saturating_sub(10)..(at + 20).min(src.len())).unwrap_or("").into();
            mismatches.push(format!("{f}, line {}: Rust `{}`, Lunar `{}` near {context:?}", i + 1, e.trim(), g.trim()));
        }
    }
    assert!(mismatches.is_empty(), "{} of {} files {what} differently:\n{}", mismatches.len(), files.len(), mismatches.join("\n"));
}

/// `ln_lexer`'s tokens and errors in the format of `ln_lexer::dump` in
/// craters/ln-lexer.
fn rust_tokens(src: &str) -> String {
    let (toks, errs) = ln_lexer::lex(src);
    let mut out = String::new();
    for t in toks {
        out.push_str(&format!("{:?} {} {}\n", t.token, t.range.start, t.range.end));
    }
    for e in errs {
        out.push_str(&format!("error {:?} {} {}\n", e.kind, e.range.start, e.range.end));
    }
    let range = |r: &Option<std::ops::Range<usize>>| r.as_ref().map_or(" -".to_string(), |r| format!(" {}..{}", r.start, r.end));
    for c in ln_lexer::conflict_markers(src) {
        out.push_str(&format!("conflict {}..{}{}{}{}\n", c.start.start, c.start.end, range(&c.base), range(&c.mid), range(&c.end)));
    }
    out
}

/// `ln_parser`'s tree and syntax errors in the format of `ln_ast::dump` and
/// `ln_parser::dump_diags` in craters/.
fn rust_tree(src: &str) -> String {
    let mut ids = ln_parser::IdGen::new();
    let (file, diags) = ln_parser::parse_file(src, 0, &mut ids);
    let mut out = ln_ast::dump::file(&file);
    for d in diags {
        out.push_str(&format!("error {} @{}..{} {}\n", d.code, d.span.start, d.span.end, ln_ast::dump::escape(&d.message)));
    }
    out
}

#[test]
fn lexer_matches_rust_lexer() {
    compare(&[], "lex", rust_tokens);
}

#[test]
fn parser_matches_rust_parser() {
    compare(&["--parse"], "parse", rust_tree);
}

/// Error recovery: every program cut off a third and two thirds of the way
/// through (at a character boundary) parses to the same tree and errors.
#[test]
fn parser_matches_rust_parser_on_broken_programs() {
    let root = root();
    let dir = std::env::temp_dir().join(format!("lunar-selfhost-broken-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut files = Vec::new();
    for (n, f) in all_files(&root).iter().enumerate() {
        let src = std::fs::read_to_string(root.join(f)).unwrap();
        for (k, frac) in [(1, 3), (2, 3)] {
            let mut cut = src.len() * k / frac;
            while !src.is_char_boundary(cut) {
                cut -= 1;
            }
            let name = format!("{n}-{k}.ln");
            std::fs::write(dir.join(&name), &src[..cut]).unwrap();
            files.push(name);
        }
    }
    compare_files(&dir, &files, &["--parse"], "parse", rust_tree);
    let _ = std::fs::remove_dir_all(&dir);
}
