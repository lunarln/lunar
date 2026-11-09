//! Embed the Lunar standard library sources (`std/`) into the compiler.

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
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("std_files.rs");
    std::fs::write(out, src).unwrap();
}
