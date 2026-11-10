//! Linking object files into executables with the platform's toolchain.
//!
//! * Windows: `clang` if available, otherwise MSVC `link.exe` (located with
//!   the same registry/vswhere logic Rust uses, via the `cc` crate).
//! * Linux / macOS: the system C compiler driver (`cc`).
//!
//! `$LUNAR_CC` overrides the linker driver on every platform (it is invoked
//! like a C compiler: `$LUNAR_CC file.o -o out`).
//!
//! Native libraries named by `#[link(name = "...")]` are passed on
//! (`-lname`, or `name.lib` for `link.exe`), searched for in the
//! directories listed in `$LUNAR_LIB_PATH` (separated like `PATH`) as well
//! as the linker's defaults.

use ln_diag::{Diagnostic, Span, codes::build};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Link `obj` and the native libraries `libs` into the executable `out`.
/// Failures are `build::link` diagnostics; the linker's own output is
/// attached as a note.
pub fn link(obj: &Path, out: &Path, libs: &[String]) -> Result<(), Diagnostic> {
    let search = lib_path();
    let gnu_args = |cmd: &mut Command| {
        for d in &search {
            cmd.arg(format!("-L{}", d.display()));
        }
        for l in libs {
            cmd.arg(format!("-l{l}"));
        }
    };
    if let Ok(cc) = std::env::var("LUNAR_CC") {
        let mut cmd = Command::new(cc);
        cmd.arg(obj).arg("-o").arg(out);
        gnu_args(&mut cmd);
        return run(cmd.args(system_libs()));
    }
    if cfg!(windows) {
        return link_windows(obj, out, libs, &search);
    }
    let mut cmd = Command::new("cc");
    cmd.arg(obj).arg("-o").arg(out);
    gnu_args(&mut cmd);
    for d in &search {
        // Shared libraries are found through the search path at run time too.
        cmd.arg(format!("-Wl,-rpath,{}", d.display()));
    }
    run(cmd.args(system_libs()))
}

/// The directories in `$LUNAR_LIB_PATH`.
fn lib_path() -> Vec<PathBuf> {
    std::env::var_os("LUNAR_LIB_PATH").map(|p| std::env::split_paths(&p).filter(|d| !d.as_os_str().is_empty()).collect()).unwrap_or_default()
}

fn system_libs() -> Vec<&'static str> {
    if cfg!(target_os = "linux") {
        vec!["-lm"]
    } else if cfg!(windows) {
        vec!["-llegacy_stdio_definitions", "-lkernel32"]
    } else {
        vec![]
    }
}

/// The main thread's stack on Windows, in bytes (reserved, used as
/// needed). Windows' default, 1 MiB, is less than deeply recursive
/// programs need, such as the self-hosted compiler on nested macro
/// expansions (up to 128 deep); Linux gives 8 MiB by default.
const MAIN_STACK: usize = 8 << 20;

fn link_windows(obj: &Path, out: &Path, libs: &[String], search: &[PathBuf]) -> Result<(), Diagnostic> {
    if which("clang").is_some() {
        let mut cmd = Command::new("clang");
        cmd.arg(obj).arg("-o").arg(out);
        for d in search {
            cmd.arg(format!("-L{}", d.display()));
        }
        for l in libs {
            cmd.arg(format!("-l{l}"));
        }
        // `/Brepro`: no link timestamp, so the same program links to the
        // same bytes (the self-hosted compiler's bootstrap compares them).
        return run(cmd.args(["-llegacy_stdio_definitions", "-lkernel32", "-Wl,/Brepro", &format!("-Wl,/STACK:{MAIN_STACK}")]));
    }
    let target = if cfg!(target_arch = "aarch64") { "aarch64-pc-windows-msvc" } else { "x86_64-pc-windows-msvc" };
    let Some(tool) = cc::windows_registry::find_tool(target, "link.exe") else {
        return Err(Diagnostic::error(build::LINK, Span::DUMMY, "no linker found").with_help("install the MSVC build tools or LLVM `clang`, or set `LUNAR_CC`"));
    };
    let mut cmd = tool.to_command();
    cmd.arg("/NOLOGO").arg(format!("/OUT:{}", out.display())).arg(obj).args([
        "/SUBSYSTEM:CONSOLE",
        "libcmt.lib",
        "kernel32.lib",
        "legacy_stdio_definitions.lib",
        "/Brepro",
        &format!("/STACK:{MAIN_STACK}"),
    ]);
    for d in search {
        cmd.arg(format!("/LIBPATH:{}", d.display()));
    }
    for l in libs {
        cmd.arg(format!("{l}.lib"));
    }
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

fn run(cmd: &mut Command) -> Result<(), Diagnostic> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    let output = cmd.output().map_err(|e| {
        Diagnostic::error(build::LINK, Span::DUMMY, format!("could not run the linker `{program}`: {e}"))
            .with_help("install a linker, or point `LUNAR_CC` at one")
    })?;
    if output.status.success() {
        return Ok(());
    }
    let mut d = Diagnostic::error(build::LINK, Span::DUMMY, format!("linking with `{program}` failed: {}", output.status));
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    if !text.trim().is_empty() {
        d = d.with_note(format!("linker output:\n{}", text.trim_end()));
    }
    Err(d)
}
