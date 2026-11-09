//! Embed the Lunar standard library sources (`std/`) and the diagnostic
//! explanations (`docs/errors/`) into the compiler.

use std::fmt::Write;
use std::path::{Path, PathBuf};

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).expect("read std dir").flatten().collect();
    entries.sort_by_key(|e| e.path());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|x| x == "ln") {
            out.push(p);
        }
    }
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let std_dir = manifest.join("../../std").canonicalize().expect("std/ directory");
    println!("cargo:rerun-if-changed={}", std_dir.display());
    let mut files = Vec::new();
    collect(&std_dir, &mut files);
    let mut src = String::from("/// (relative path, contents) of every embedded std source file.\npub static STD_FILES: &[(&str, &str)] = &[\n");
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
        let rel = f.strip_prefix(&std_dir).unwrap().to_string_lossy().replace('\\', "/");
        let abs = f.to_string_lossy().replace('\\', "/");
        writeln!(src, "    ({rel:?}, include_str!({abs:?})),").unwrap();
    }
    src.push_str("];\n");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    std::fs::write(out_dir.join("std_files.rs"), src).unwrap();
    embed_explanations(&manifest, &out_dir);
}

/// Embed `docs/errors/<scope>/<name>.md` as `(code, markdown)` pairs for
/// `lunar explain`.
fn embed_explanations(manifest: &Path, out_dir: &Path) {
    let dir = manifest.join("../../docs/errors").canonicalize().expect("docs/errors/ directory");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut scopes: Vec<PathBuf> = std::fs::read_dir(&dir).expect("read docs/errors").flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    scopes.sort();
    let mut src = String::from("/// (`scope::name`, Markdown) of every embedded diagnostic explanation.\npub static EXPLANATIONS: &[(&str, &str)] = &[\n");
    for scope_dir in scopes {
        println!("cargo:rerun-if-changed={}", scope_dir.display());
        let scope = scope_dir.file_name().unwrap().to_string_lossy().into_owned();
        let mut files: Vec<PathBuf> = std::fs::read_dir(&scope_dir)
            .expect("read docs/errors scope")
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "md"))
            .collect();
        files.sort();
        for f in files {
            println!("cargo:rerun-if-changed={}", f.display());
            let code = format!("{scope}::{}", f.file_stem().unwrap().to_string_lossy());
            let abs = f.to_string_lossy().replace('\\', "/");
            writeln!(src, "    ({code:?}, include_str!({abs:?})),").unwrap();
        }
    }
    src.push_str("];\n");
    std::fs::write(out_dir.join("explain_files.rs"), src).unwrap();
}
