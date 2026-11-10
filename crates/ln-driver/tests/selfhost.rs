//! The compiler written in Lunar (`craters/`) against the Rust one: built
//! with `lunar`, run over every `.ln` file in the repository, and compared
//! stage by stage: the lexer's tokens, then the parser's syntax trees and
//! syntax errors, then the diagnostics as `lunar check` draws them on
//! tests/ui. Then the bootstrap: it compiles itself, and the compiler that
//! makes compiles itself into the same executable.

use std::collections::BTreeMap;
use std::ffi::OsString;
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

/// Where LLVM's shared library is, for linking `craters/` (whose code
/// generator calls LLVM's C API): `$LUNAR_LIB_PATH` if set, else the LLVM
/// the Rust compiler was built with.
fn llvm_lib_path() -> Option<OsString> {
    if let Some(p) = std::env::var_os("LUNAR_LIB_PATH") {
        return Some(p);
    }
    if cfg!(windows) {
        let prefix = std::env::var_os("LLVM_SYS_201_PREFIX").map_or_else(|| PathBuf::from(r"C:\Program Files\LLVM"), PathBuf::from);
        return Some(prefix.join("lib").into_os_string());
    }
    let config = std::env::var("LLVM_CONFIG").unwrap_or_else(|_| "llvm-config".into());
    let out = Command::new(config).arg("--libdir").output().ok()?;
    Some(String::from_utf8(out.stdout).ok()?.trim().into())
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
            .envs(llvm_lib_path().map(|p| ("LUNAR_LIB_PATH", p)))
            .output()
            .unwrap();
        assert!(build.status.success(), "building craters/:\n{}{}", String::from_utf8_lossy(&build.stdout), String::from_utf8_lossy(&build.stderr));
        craters.join("target/release").join(format!("lunar{}", std::env::consts::EXE_SUFFIX))
    })
}

/// Run the Lunar driver over every program and compare each file's output
/// with `rust`'s for that file.
fn compare(args: &[&str], what: &str, rust: impl Fn(&str) -> String) {
    let root = root();
    let files = all_files(&root);
    compare_files(&root, &files, args, what, |_, src| rust(src));
}

/// Run the Lunar driver over `files` (relative to `root`) and compare each
/// file's output with `rust`'s for that file (given its path and text).
fn compare_files(root: &Path, files: &[String], args: &[&str], what: &str, rust: impl Fn(&str, &str) -> String) {
    let run = Command::new(driver()).args(args).args(files).current_dir(root).env("LUNAR_STD_DIR", std_dir_of(root)).output().unwrap();
    assert!(run.status.success(), "the Lunar compiler failed:\n{}", String::from_utf8_lossy(&run.stderr));
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
        let expected = rust(f, &src);
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
    compare(&["--lex"], "lex", rust_tokens);
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
    compare_files(&dir, &files, &["--parse"], "parse", |_, src| rust_tree(src));
    let _ = std::fs::remove_dir_all(&dir);
}

/// What `lunar check FILE` (run from `root`, without color) prints before
/// its summary line (`error: could not compile ...`): the diagnostics. It
/// reads the same std directory as the Lunar driver, so labels in std
/// (such as the trait parameter of a mismatched impl) name the same file.
fn rust_rendered(root: &Path, file: &str) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_lunar")).args(["check", file]).current_dir(root).env("LUNAR_STD_DIR", std_dir_of(root)).output().unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    let mut text = String::new();
    for line in stderr.lines() {
        if line.starts_with("error: could not compile") {
            break;
        }
        text.push_str(line);
        text.push('\n');
    }
    text
}

/// The Lunar compiler draws its diagnostics (with ln-ariadne) exactly as
/// the Rust compiler does (with ariadne), byte for byte without color:
/// the same codes, messages, labels, notes and helps, laid out the same.
#[test]
fn diagnostics_render_as_rust_renders_them() {
    let root = root();
    let mut files = Vec::new();
    ln_files(&root, &root.join("tests/ui"), &mut files);
    files.sort();
    assert!(files.len() > 50, "found only {} files", files.len());
    compare_files(&root, &files, &["--render"], "render diagnostics", |f, _| rust_rendered(&root, f));
}

/// The standard library's directory, for `$LUNAR_STD_DIR`: without
/// `canonicalize`'s `\\?\`, under which Windows takes no `/` (and the
/// driver appends `/core/lib.ln`).
fn std_dir_of(root: &Path) -> String {
    root.join("std").to_string_lossy().trim_start_matches(r"\\?\").to_string()
}

/// The bootstrap. Stage 1 is the Lunar compiler as built by the Rust one;
/// stage 2 is what stage 1 makes of the compiler's source, and stage 3 what
/// stage 2 makes of it. Stages 1 and 2 are the same program built by
/// different compilers, so stages 2 and 3 must be identical: the same LLVM
/// IR and the same bytes.
///
/// Each stage is linked to a file of the same name (in its own directory):
/// on macOS the linker signs the executable ad hoc, and the signature
/// names the executable by its file name.
#[test]
fn compiler_builds_itself() {
    let root = root();
    let dir = std::env::temp_dir().join(format!("lunar-bootstrap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let std_dir = std_dir_of(&root);
    let stage = |compiler: &Path, n: u32| -> (PathBuf, String) {
        let stage_dir = dir.join(format!("stage{n}"));
        std::fs::create_dir_all(&stage_dir).unwrap();
        let exe = stage_dir.join(format!("lunar{}", std::env::consts::EXE_SUFFIX));
        let ll = dir.join(format!("stage{n}.ll"));
        let out = Command::new(compiler)
            .args(["build", "-p", "lunar", "-o"])
            .arg(&exe)
            .arg("--emit-llvm")
            .arg(&ll)
            .current_dir(root.join("craters"))
            .env("LUNAR_STD_DIR", &std_dir)
            .envs(llvm_lib_path().map(|p| ("LUNAR_LIB_PATH", p)))
            .output()
            .unwrap();
        assert!(out.status.success(), "stage {n}:\n{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        (exe, std::fs::read_to_string(&ll).unwrap())
    };
    let (stage2, ir2) = stage(driver(), 2);
    let (stage3, ir3) = stage(&stage2, 3);
    if let Some((i, (a, b))) = ir2.lines().zip(ir3.lines()).enumerate().find(|(_, (a, b))| a != b) {
        panic!("stages 2 and 3 differ at IR line {}:\n  {a}\n  {b}", i + 1);
    }
    assert_eq!(ir2.len(), ir3.len(), "stage 3's IR is a prefix of stage 2's, or the other way round");
    let (bin2, bin3) = (std::fs::read(&stage2).unwrap(), std::fs::read(&stage3).unwrap());
    if let Some(at) = bin2.iter().zip(&bin3).position(|(a, b)| a != b).or((bin2.len() != bin3.len()).then(|| bin2.len().min(bin3.len()))) {
        let around = |bin: &[u8]| String::from_utf8_lossy(&bin[at.saturating_sub(32).min(bin.len())..(at + 32).min(bin.len())]).into_owned();
        panic!(
            "stages 2 and 3 have the same IR but link to different bytes ({} and {} bytes, first difference at byte {at}):\n  {:?}\n  {:?}",
            bin2.len(),
            bin3.len(),
            around(&bin2),
            around(&bin3)
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
