//! Link the LLVM 20 C API as a shared library.
//!
//! llvm-sys is built with `no-llvm-linking`, so we choose the library here:
//!
//! * Windows: `LLVM-C.lib` from `$LLVM_SYS_201_PREFIX`, `$LLVM_PATH`, or the
//!   default `C:\Program Files\LLVM` install (the official installer ships no
//!   `llvm-config`). `LLVM-C.dll` must be on `PATH` at run time.
//! * Linux / macOS: `llvm-config --libdir` (trying `llvm-config-20` and the
//!   Homebrew prefix too), linking `libLLVM` with an rpath to that directory.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    for v in ["LLVM_SYS_201_PREFIX", "LLVM_PATH", "LLVM_CONFIG"] {
        println!("cargo:rerun-if-env-changed={v}");
    }
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        let mut candidates: Vec<PathBuf> = Vec::new();
        for v in ["LLVM_SYS_201_PREFIX", "LLVM_PATH"] {
            if let Ok(p) = std::env::var(v) {
                candidates.push(PathBuf::from(p));
            }
        }
        candidates.push(PathBuf::from(r"C:\Program Files\LLVM"));
        let dir = candidates
            .iter()
            .map(|p| p.join("lib"))
            .find(|lib| lib.join("LLVM-C.lib").exists())
            .unwrap_or_else(|| panic!("LLVM 20 not found: install it from https://github.com/llvm/llvm-project/releases or set LLVM_SYS_201_PREFIX"));
        println!("cargo:rustc-link-search=native={}", dir.display());
        println!("cargo:rustc-link-lib=dylib=LLVM-C");
        return;
    }
    let configs: Vec<String> = match std::env::var("LLVM_CONFIG") {
        Ok(c) => vec![c],
        Err(_) => {
            let mut v = Vec::new();
            if let Ok(p) = std::env::var("LLVM_SYS_201_PREFIX") {
                v.push(format!("{p}/bin/llvm-config"));
            }
            v.extend(
                [
                    "llvm-config-20",
                    "llvm-config",
                    "/opt/homebrew/opt/llvm@20/bin/llvm-config",
                    "/usr/local/opt/llvm@20/bin/llvm-config",
                    "/usr/lib/llvm-20/bin/llvm-config",
                ]
                .map(String::from),
            );
            v
        }
    };
    for cfg in &configs {
        let Ok(out) = Command::new(cfg).arg("--version").output() else { continue };
        let version = String::from_utf8_lossy(&out.stdout);
        if !version.starts_with("20.") {
            continue;
        }
        let libdir = String::from_utf8_lossy(&Command::new(cfg).arg("--libdir").output().unwrap().stdout).trim().to_string();
        println!("cargo:rustc-link-search=native={libdir}");
        println!("cargo:rustc-link-lib=dylib=LLVM");
        println!("cargo:rustc-link-arg=-Wl,-rpath,{libdir}");
        return;
    }
    panic!("LLVM 20 not found: install llvm-20 (apt.llvm.org / brew install llvm@20) or set LLVM_CONFIG");
}
