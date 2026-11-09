//! Linking object files into executables with the platform's toolchain.
//!
//! * Windows: `clang` if available, otherwise MSVC `link.exe` (located with
//!   the same registry/vswhere logic Rust uses, via the `cc` crate).
//! * Linux / macOS: the system C compiler driver (`cc`).
//!
//! `$LUNAR_CC` overrides the linker driver on every platform (it is invoked
//! like a C compiler: `$LUNAR_CC file.o -o out`).

use std::path::Path;
use std::process::Command;

pub fn link(obj: &Path, out: &Path) -> Result<(), String> {
    if let Ok(cc) = std::env::var("LUNAR_CC") {
        return run(Command::new(cc).arg(obj).arg("-o").arg(out).args(unix_libs()));
    }
    if cfg!(windows) { link_windows(obj, out) } else { run(Command::new("cc").arg(obj).arg("-o").arg(out).args(unix_libs())) }
}

fn unix_libs() -> Vec<&'static str> {
    if cfg!(target_os = "linux") {
        vec!["-lm"]
    } else if cfg!(windows) {
        vec!["-llegacy_stdio_definitions", "-lkernel32"]
    } else {
        vec![]
    }
}

fn link_windows(obj: &Path, out: &Path) -> Result<(), String> {
    if which("clang").is_some() {
        return run(Command::new("clang").arg(obj).arg("-o").arg(out).args(["-llegacy_stdio_definitions", "-lkernel32"]));
    }
    let target = if cfg!(target_arch = "aarch64") { "aarch64-pc-windows-msvc" } else { "x86_64-pc-windows-msvc" };
    let Some(tool) = cc::windows_registry::find_tool(target, "link.exe") else {
        return Err("no linker found: install the MSVC build tools (or LLVM clang), or set LUNAR_CC".into());
    };
    let mut cmd = tool.to_command();
    cmd.arg("/NOLOGO").arg(format!("/OUT:{}", out.display())).arg(obj).args([
        "/SUBSYSTEM:CONSOLE",
        "libcmt.lib",
        "kernel32.lib",
        "legacy_stdio_definitions.lib",
    ]);
    run(&mut cmd)
}

fn which(name: &str) -> Option<std::path::PathBuf> {
    let exts: &[&str] = if cfg!(windows) { &[".exe", ".cmd", ".bat", ""] } else { &[""] };
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for ext in exts {
            let p = dir.join(format!("{name}{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let output = cmd.output().map_err(|e| format!("failed to run linker `{:?}`: {e}", cmd.get_program()))?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!("linking failed ({:?}):\n{}{}", cmd.get_program(), String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)))
}
