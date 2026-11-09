//! `lunar`: the Lunar compiler.

use clap::{Parser, Subcommand};
use ln_driver::{Analysis, BuildOptions, analyze, build, emit_diagnostics};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

mod explain;

#[derive(Parser)]
#[command(name = "lunar", version, about = "The Lunar programming language compiler")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Type-check a program without generating code.
    Check {
        file: PathBuf,
        /// Evaluate `#[cfg]` for this OS instead of the host (linux, macos, windows).
        #[arg(long)]
        target_os: Option<String>,
    },
    /// Compile a program to an executable.
    Build {
        file: PathBuf,
        /// Output path (defaults to the file name without extension).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Optimize (and disable overflow checks).
        #[arg(long)]
        release: bool,
        /// Also write the (optimized) LLVM IR to this file.
        #[arg(long)]
        emit_llvm: Option<PathBuf>,
    },
    /// Print the optimized LLVM IR for a program without building it.
    EmitLlvm {
        file: PathBuf,
        /// Write the IR to this file instead of stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Optimize (and disable overflow checks), as `build --release` does.
        #[arg(long)]
        release: bool,
        /// Emit the `#[test]` harness instead of `main`.
        #[arg(long)]
        test: bool,
    },
    /// Compile and run a program.
    Run {
        file: PathBuf,
        #[arg(long)]
        release: bool,
        /// Arguments passed to the program (after `--`).
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Run the language server (LSP over stdio), for editors.
    Lsp {
        /// Accepted for compatibility with LSP clients; stdio is the only transport.
        #[arg(long, hide = true)]
        stdio: bool,
    },
    /// Compile and run the `#[test]` functions in a program.
    Test {
        file: PathBuf,
        /// Only run tests whose name contains this string.
        filter: Option<String>,
        #[arg(long)]
        release: bool,
    },
    /// Explain a diagnostic code, e.g. `borrow::shrink_while_borrowed`.
    Explain {
        /// The code, as `scope::name` or pasted as `err(scope::name)`.
        /// Without one, list every code.
        code: Option<String>,
        /// List every code with a one-line summary.
        #[arg(long)]
        list: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let color = std::io::stderr().is_terminal();
    match cli.command {
        Cmd::Check { file, target_os } => {
            let a = ln_driver::analyze_for(&file, false, target_os.as_deref());
            match finish_front(&a, color) {
                Ok(()) => ExitCode::SUCCESS,
                Err(()) => ExitCode::FAILURE,
            }
        }
        Cmd::Build { file, output, release, emit_llvm } => {
            let out = output.unwrap_or_else(|| default_exe(&file));
            match compile(&file, &out, release, false, emit_llvm, color) {
                Ok(()) => ExitCode::SUCCESS,
                Err(()) => ExitCode::FAILURE,
            }
        }
        Cmd::EmitLlvm { file, output, release, test } => match emit_ir(&file, release, test, color) {
            Ok(ir) => {
                let written = match &output {
                    Some(p) => std::fs::write(p, ir).map_err(|e| format!("could not write `{}`: {e}", p.display())),
                    None => std::io::Write::write_all(&mut std::io::stdout().lock(), ir.as_bytes()).map_err(|e| e.to_string()),
                };
                match written {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(e) => {
                        eprintln!("error: {e}");
                        ExitCode::FAILURE
                    }
                }
            }
            Err(()) => ExitCode::FAILURE,
        },
        Cmd::Run { file, release, args } => {
            let out = temp_exe(&file, "run");
            if compile(&file, &out, release, false, None, color).is_err() {
                return ExitCode::FAILURE;
            }
            let status = std::process::Command::new(&out).args(&args).status();
            let _ = std::fs::remove_file(&out);
            match status {
                Ok(s) => exit_code_of(s),
                Err(e) => {
                    eprintln!("error: could not run program: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Cmd::Lsp { .. } => match ln_driver::lsp::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: language server failed: {e}");
                ExitCode::FAILURE
            }
        },
        Cmd::Test { file, filter, release } => run_tests(&file, filter.as_deref(), release, color),
        Cmd::Explain { code, list } => explain::run(code.as_deref(), list),
    }
}

/// Report front-end diagnostics; `Err` if there were errors.
fn finish_front(a: &Analysis, color: bool) -> Result<(), ()> {
    emit_diagnostics(&a.map, &a.diags, color);
    let errors = a.diags.iter().filter(|d| d.is_error()).count();
    if errors > 0 || a.program.is_none() {
        eprintln!("error: could not compile due to {errors} previous error{}", if errors == 1 { "" } else { "s" });
        if let Some(hint) = explain::hint(&a.diags) {
            eprintln!("\n{hint}");
        }
        return Err(());
    }
    Ok(())
}

fn compile(file: &Path, out: &Path, release: bool, test: bool, emit_llvm: Option<PathBuf>, color: bool) -> Result<(), ()> {
    let a = analyze(file, test);
    finish_front(&a, color)?;
    if !test && a.program.as_ref().is_some_and(|p| p.main.is_none()) {
        eprintln!("error: `{}` has no `main` function", file.display());
        return Err(());
    }
    let opts = BuildOptions { release, test, output: out.to_path_buf(), emit_llvm, overflow_checks: !release };
    build(&a, &opts).map_err(|e| eprintln!("error: {e}"))
}

/// Forward a program's exit status. A crash (a signal on Unix, an NTSTATUS
/// such as `0xC0000005` on Windows) must never be reported as success.
fn exit_code_of(s: std::process::ExitStatus) -> ExitCode {
    match s.code() {
        Some(c @ 0..=255) => ExitCode::from(c as u8),
        Some(c) => {
            eprintln!("error: program crashed with status {:#010X}", c as u32);
            ExitCode::from(101)
        }
        None => {
            eprintln!("error: program was terminated: {s}");
            ExitCode::from(101)
        }
    }
}

fn emit_ir(file: &Path, release: bool, test: bool, color: bool) -> Result<String, ()> {
    let a = analyze(file, test);
    finish_front(&a, color)?;
    if !test && a.program.as_ref().is_some_and(|p| p.main.is_none()) {
        eprintln!("error: `{}` has no `main` function", file.display());
        return Err(());
    }
    let opts = BuildOptions { release, test, output: PathBuf::new(), emit_llvm: None, overflow_checks: !release };
    ln_driver::emit_llvm(&a, &opts).map_err(|e| eprintln!("error: {e}"))
}

fn default_exe(file: &Path) -> PathBuf {
    let stem = file.with_extension("");
    if cfg!(windows) { stem.with_extension("exe") } else { stem }
}

fn temp_exe(file: &Path, tag: &str) -> PathBuf {
    let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "a".into());
    let name = format!("lunar-{tag}-{stem}-{}{}", std::process::id(), std::env::consts::EXE_SUFFIX);
    std::env::temp_dir().join(name)
}

fn run_tests(file: &Path, filter: Option<&str>, release: bool, color: bool) -> ExitCode {
    let a = analyze(file, true);
    if finish_front(&a, color).is_err() {
        return ExitCode::FAILURE;
    }
    let program = a.program.as_ref().unwrap();
    let names: Vec<String> = program.tests.iter().map(|t| program.def(*t).name.clone()).collect();
    let out = temp_exe(file, "test");
    let opts = BuildOptions { release, test: true, output: out.clone(), emit_llvm: None, overflow_checks: !release };
    if let Err(e) = build(&a, &opts) {
        eprintln!("error: {e}");
        return ExitCode::FAILURE;
    }
    let selected: Vec<(usize, &String)> = names.iter().enumerate().filter(|(_, n)| filter.is_none_or(|f| n.contains(f))).collect();
    println!("\nrunning {} test{}", selected.len(), if selected.len() == 1 { "" } else { "s" });
    let mut failed = Vec::new();
    for (i, name) in &selected {
        match std::process::Command::new(&out).arg(i.to_string()).output() {
            Ok(o) if o.status.success() => println!("test {name} ... ok"),
            Ok(o) => {
                println!("test {name} ... FAILED");
                let text = String::from_utf8_lossy(&o.stderr).into_owned() + &String::from_utf8_lossy(&o.stdout);
                failed.push((name.to_string(), text));
            }
            Err(e) => {
                println!("test {name} ... FAILED");
                failed.push((name.to_string(), format!("could not run test binary: {e}")));
            }
        }
    }
    let _ = std::fs::remove_file(&out);
    if !failed.is_empty() {
        println!("\nfailures:");
        for (n, out) in &failed {
            println!("\n---- {n} ----\n{}", out.trim_end());
        }
    }
    let passed = selected.len() - failed.len();
    println!(
        "\ntest result: {}. {passed} passed; {} failed; {} filtered out\n",
        if failed.is_empty() { "ok" } else { "FAILED" },
        failed.len(),
        names.len() - selected.len()
    );
    if failed.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
