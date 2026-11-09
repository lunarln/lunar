//! `lunar`: the Lunar compiler and project manager.
//!
//! Commands that take a file (`lunar run hello.ln`) work on that file alone.
//! Without one, they work on the project around the current directory (the
//! nearest `lunar.toml`).

use clap::{Parser, Subcommand};
use ln_diag::Diagnostic;
use ln_driver::project::{self, DepSpecArgs, Graph, Target};
use ln_driver::{Analysis, AnalyzeOptions, BuildOptions, analyze, analyze_craters, build, emit_diagnostics};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

mod explain;

#[derive(Parser)]
#[command(name = "lunar", version, about = "The Lunar programming language: compiler and project manager")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create a new package in a new directory.
    New {
        path: PathBuf,
        /// Create a library (src/lib.ln) instead of a program (src/main.ln).
        #[arg(long)]
        lib: bool,
    },
    /// Create a new package in the current directory.
    Init {
        #[arg(long)]
        lib: bool,
    },
    /// Type-check a file, or the current project, without generating code.
    Check {
        file: Option<PathBuf>,
        /// Evaluate `#[cfg]` for this OS instead of the host (linux, macos, windows).
        #[arg(long)]
        target_os: Option<String>,
    },
    /// Compile a file, or the current project's program, to an executable.
    Build {
        file: Option<PathBuf>,
        /// Output path (defaults to the file name without extension, or
        /// target/<profile>/<package> in a project).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Optimize (and disable overflow checks).
        #[arg(long)]
        release: bool,
        /// Also write the (optimized) LLVM IR to this file.
        #[arg(long)]
        emit_llvm: Option<PathBuf>,
    },
    /// Compile and run a file, or the current project's program.
    Run {
        file: Option<PathBuf>,
        #[arg(long)]
        release: bool,
        /// Arguments passed to the program (after `--`).
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Run the `#[test]` functions of a file (`lunar test file.ln [filter]`)
    /// or of the current project (`lunar test [filter]`).
    Test {
        /// A `.ln` file and/or a filter: only tests whose name contains it run.
        args: Vec<String>,
        #[arg(long)]
        release: bool,
    },
    /// Add a dependency to the current project.
    Add {
        /// The dependency's name, optionally with a version: `json` or `json@1.2`.
        name: String,
        /// A local package directory.
        #[arg(long)]
        path: Option<String>,
        /// A git repository URL.
        #[arg(long)]
        git: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        branch: Option<String>,
        #[arg(long)]
        rev: Option<String>,
        /// The package's real name, if the dependency is renamed.
        #[arg(long)]
        package: Option<String>,
    },
    /// Remove a dependency from the current project.
    Remove { name: String },
    /// Re-resolve dependencies, updating git dependencies to the newest
    /// commit of their branch or tag, and rewrite lunar.lock.
    Update,
    /// Remove the project's build output (target/).
    Clean,
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
    /// Run the language server (LSP over stdio), for editors.
    Lsp {
        /// Accepted for compatibility with LSP clients; stdio is the only transport.
        #[arg(long, hide = true)]
        stdio: bool,
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
    let result = match cli.command {
        Cmd::New { path, lib } => new_package(&path, lib),
        Cmd::Init { lib } => std::env::current_dir().map_err(|e| e.to_string()).and_then(|d| init_package(&d, lib)).map_err(fail),
        Cmd::Check { file: Some(file), target_os } => {
            let a = ln_driver::analyze_for(&file, false, target_os.as_deref());
            finish_front(&a, color)
        }
        Cmd::Check { file: None, target_os } => check_project(target_os.as_deref(), color),
        Cmd::Build { file: Some(file), output, release, emit_llvm } => {
            let out = output.unwrap_or_else(|| default_exe(&file));
            compile_file(&file, &out, release, emit_llvm, color)
        }
        Cmd::Build { file: None, output, release, emit_llvm } => build_project(output, release, emit_llvm, color).map(|_| ()),
        Cmd::Run { file: Some(file), release, args } => {
            let out = temp_exe(&file, "run");
            match compile_file(&file, &out, release, None, color) {
                Ok(()) => {
                    let code = run_program(&out, &args);
                    let _ = std::fs::remove_file(&out);
                    return code;
                }
                Err(()) => Err(()),
            }
        }
        Cmd::Run { file: None, release, args } => match build_project(None, release, None, color) {
            Ok(exe) => {
                status("Running", &exe.display().to_string());
                return run_program(&exe, &args);
            }
            Err(()) => Err(()),
        },
        Cmd::Test { args, release } => return test(&args, release, color),
        Cmd::Add { name, path, git, tag, branch, rev, package } => add(&name, DepSpecArgs { version: None, path, git, tag, branch, rev, package }),
        Cmd::Remove { name } => with_project(|root| {
            project::remove_dependency(root, &name)?;
            status("Removed", &name);
            resolve(root, false).map(|_| ())
        }),
        Cmd::Update => with_project(|root| resolve(root, true).map(|_| ())),
        Cmd::Clean => with_project(|root| {
            let target = root.join("target");
            if target.exists() {
                std::fs::remove_dir_all(&target).map_err(|e| format!("could not remove `{}`: {e}", target.display()))?;
            }
            status("Removed", &target.display().to_string());
            Ok(())
        }),
        Cmd::EmitLlvm { file, output, release, test } => match emit_ir(&file, release, test, color) {
            Ok(ir) => {
                let written = match &output {
                    Some(p) => std::fs::write(p, ir).map_err(|e| format!("could not write `{}`: {e}", p.display())),
                    None => std::io::Write::write_all(&mut std::io::stdout().lock(), ir.as_bytes()).map_err(|e| e.to_string()),
                };
                written.map_err(fail)
            }
            Err(()) => Err(()),
        },
        Cmd::Lsp { .. } => ln_driver::lsp::run().map_err(|e| fail(format!("language server failed: {e}"))),
        Cmd::Explain { code, list } => return explain::run(code.as_deref(), list),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(()) => ExitCode::FAILURE,
    }
}

fn fail(e: impl std::fmt::Display) {
    eprintln!("error: {e}");
}

/// A Cargo-style status line on stderr: `   Compiling hello v0.1.0`.
fn status(verb: &str, what: &str) {
    if std::io::stderr().is_terminal() {
        eprintln!("\x1b[1;32m{verb:>12}\x1b[0m {what}");
    } else {
        eprintln!("{verb:>12} {what}");
    }
}

// ===================================================================== projects

fn new_package(path: &Path, lib: bool) -> Result<(), ()> {
    if path.exists() && path.read_dir().is_ok_and(|mut d| d.next().is_some()) {
        fail(format!("`{}` already exists and is not empty", path.display()));
        return Err(());
    }
    std::fs::create_dir_all(path).map_err(|e| fail(format!("could not create `{}`: {e}", path.display())))?;
    init_package(path, lib).map_err(fail)
}

fn init_package(dir: &Path, lib: bool) -> Result<(), String> {
    let name = project::init(dir, lib)?;
    status("Created", &format!("{} `{name}` package", if lib { "library" } else { "program" }));
    Ok(())
}

fn project_root() -> Result<PathBuf, String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    project::find_project(&cwd).ok_or_else(|| {
        format!(
            "no `{}` in `{}` or any parent directory (create a package with `lunar new` or `lunar init`, or pass a .ln file)",
            project::MANIFEST,
            cwd.display()
        )
    })
}

fn with_project(f: impl FnOnce(&Path) -> Result<(), String>) -> Result<(), ()> {
    let root = project_root().map_err(fail)?;
    f(&root).map_err(fail)
}

fn resolve(root: &Path, update: bool) -> Result<Graph, String> {
    project::resolve(root, update, &mut |msg| {
        let (verb, rest) = msg.split_once(' ').unwrap_or((msg, ""));
        status(verb, rest);
    })
}

fn add(name: &str, mut spec: DepSpecArgs) -> Result<(), ()> {
    let (name, version) = match name.split_once('@') {
        Some((n, v)) => (n, Some(v.to_string())),
        None => (name, None),
    };
    spec.version = version;
    if spec.path.is_none() && spec.git.is_none() && spec.version.is_none() {
        fail(format!("say where `{name}` comes from: `--path <dir>`, `--git <url>`, or a registry version (`{name}@1.0`)"));
        return Err(());
    }
    with_project(|root| {
        project::add_dependency(root, name, &spec)?;
        status("Added", name);
        resolve(root, false).map(|_| ())
    })
}

/// Analyze one target of a project.
fn analyze_target(graph: &Graph, target: &Target, test: bool, target_os: Option<&str>) -> Result<Analysis, String> {
    let craters = project::craters(graph, target)?;
    Ok(analyze_craters(&craters, &AnalyzeOptions { test, target_os, ..Default::default() }))
}

fn check_project(target_os: Option<&str>, color: bool) -> Result<(), ()> {
    let root = project_root().map_err(fail)?;
    let graph = resolve(&root, false).map_err(fail)?;
    let m = &graph.packages[0].manifest;
    status("Checking", &format!("{} v{}", m.name, m.version));
    let mut targets = Vec::new();
    if m.lib_root().is_some() {
        targets.push(Target::Lib);
    }
    if m.main_root().is_some() {
        targets.push(Target::Main);
    }
    let mut ok = true;
    for t in &targets {
        let a = analyze_target(&graph, t, false, target_os).map_err(fail)?;
        ok &= finish_front(&a, color).is_ok();
    }
    if ok {
        status("Finished", "checking");
        Ok(())
    } else {
        Err(())
    }
}

/// Build the project's program; returns the executable's path.
fn build_project(output: Option<PathBuf>, release: bool, emit_llvm: Option<PathBuf>, color: bool) -> Result<PathBuf, ()> {
    let root = project_root().map_err(fail)?;
    let graph = resolve(&root, false).map_err(fail)?;
    let m = &graph.packages[0].manifest;
    let main = m.main_root().ok_or_else(|| fail(format!("package `{}` has no program (src/main.ln) to build", m.name)))?;
    status("Compiling", &format!("{} v{} ({})", m.name, m.version, root.display()));
    let profile = if release { "release" } else { "debug" };
    let out = output.unwrap_or_else(|| root.join("target").join(profile).join(format!("{}{}", m.name, std::env::consts::EXE_SUFFIX)));
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).map_err(|e| fail(format!("could not create `{}`: {e}", dir.display())))?;
    }
    let mut a = analyze_target(&graph, &Target::Main, false, None).map_err(fail)?;
    if a.program.is_some()
        && let Some(d) = ln_driver::check_main(&a, &main, false)
    {
        a.diags.insert(0, d);
    }
    finish_front(&a, color)?;
    let opts = BuildOptions { release, test: false, output: out.clone(), emit_llvm, overflow_checks: !release };
    build(&a, &opts).map_err(|d| report(&a, d, color))?;
    status("Finished", &format!("`{profile}` profile -> {}", out.display()));
    Ok(out)
}

// ===================================================================== single files

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

/// Analyze a program that will be built as an executable. A missing `main`
/// is reported first, ahead of any warnings, with the front end's errors.
fn analyze_for_build(file: &Path, test: bool, color: bool) -> Result<Analysis, ()> {
    let mut a = analyze(file, test);
    if a.program.is_some()
        && let Some(d) = ln_driver::check_main(&a, file, test)
    {
        a.diags.insert(0, d);
    }
    finish_front(&a, color)?;
    Ok(a)
}

fn compile_file(file: &Path, out: &Path, release: bool, emit_llvm: Option<PathBuf>, color: bool) -> Result<(), ()> {
    let a = analyze_for_build(file, false, color)?;
    let opts = BuildOptions { release, test: false, output: out.to_path_buf(), emit_llvm, overflow_checks: !release };
    build(&a, &opts).map_err(|d| report(&a, d, color))
}

/// Print a diagnostic from after the front end (a missing `main`, a code
/// generation or link failure) with the `lunar explain` hint.
fn report(a: &Analysis, d: Diagnostic, color: bool) {
    let diags = [d];
    emit_diagnostics(&a.map, &diags, color);
    if let Some(hint) = explain::hint(&diags) {
        eprintln!("\n{hint}");
    }
}

fn run_program(exe: &Path, args: &[String]) -> ExitCode {
    match std::process::Command::new(exe).args(args).status() {
        Ok(s) => exit_code_of(s),
        Err(e) => {
            eprintln!("error: could not run program: {e}");
            ExitCode::FAILURE
        }
    }
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
    let a = analyze_for_build(file, test, color)?;
    let opts = BuildOptions { release, test, output: PathBuf::new(), emit_llvm: None, overflow_checks: !release };
    ln_driver::emit_llvm(&a, &opts).map_err(|d| report(&a, d, color))
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

// ===================================================================== tests

#[derive(Default)]
struct TestTally {
    passed: usize,
    failed: Vec<(String, String)>,
    filtered: usize,
}

/// `lunar test file.ln [filter]` or, in a project, `lunar test [filter]`.
fn test(args: &[String], release: bool, color: bool) -> ExitCode {
    let file_mode = args.first().is_some_and(|a| a.ends_with(".ln"));
    let mut tally = TestTally::default();
    let ok = if file_mode {
        let file = PathBuf::from(&args[0]);
        let a = analyze(&file, true);
        run_tests(&a, &file.display().to_string(), &temp_exe(&file, "test"), args.get(1).map(String::as_str), release, color, &mut tally)
    } else {
        let filter = args.first().map(String::as_str);
        match test_project(filter, release, color, &mut tally) {
            Ok(ok) => ok,
            Err(()) => return ExitCode::FAILURE,
        }
    };
    if !tally.failed.is_empty() {
        println!("\nfailures:");
        for (n, out) in &tally.failed {
            println!("\n---- {n} ----\n{}", out.trim_end());
        }
    }
    println!(
        "\ntest result: {}. {} passed; {} failed; {} filtered out\n",
        if tally.failed.is_empty() && ok { "ok" } else { "FAILED" },
        tally.passed,
        tally.failed.len(),
        tally.filtered
    );
    if tally.failed.is_empty() && ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

/// Run the tests of the library, the program and each `tests/*.ln`.
fn test_project(filter: Option<&str>, release: bool, color: bool, tally: &mut TestTally) -> Result<bool, ()> {
    let root = project_root().map_err(fail)?;
    let graph = resolve(&root, false).map_err(fail)?;
    let m = graph.packages[0].manifest.clone();
    status("Testing", &format!("{} v{}", m.name, m.version));
    let profile = if release { "release" } else { "debug" };
    let dir = root.join("target").join(profile).join("tests");
    std::fs::create_dir_all(&dir).map_err(|e| fail(format!("could not create `{}`: {e}", dir.display())))?;
    let mut targets: Vec<(Target, String, String)> = Vec::new();
    if m.lib_root().is_some() {
        targets.push((Target::Lib, "src/lib.ln".into(), "lib".into()));
    }
    if m.main_root().is_some() {
        targets.push((Target::Main, "src/main.ln".into(), "main".into()));
    }
    for f in m.test_files() {
        let stem = f.file_stem().unwrap().to_string_lossy().into_owned();
        targets.push((Target::TestFile(f.clone()), format!("tests/{stem}.ln"), format!("test-{stem}")));
    }
    let mut ok = true;
    for (t, label, exe) in targets {
        let a = analyze_target(&graph, &t, true, None).map_err(fail)?;
        let out = dir.join(format!("{}-{exe}{}", m.name, std::env::consts::EXE_SUFFIX));
        ok &= run_tests(&a, &label, &out, filter, release, color, tally);
    }
    Ok(ok)
}

/// Build a test harness for `a` and run each selected test in its own
/// process. Returns false if it did not compile.
fn run_tests(a: &Analysis, label: &str, out: &Path, filter: Option<&str>, release: bool, color: bool, tally: &mut TestTally) -> bool {
    if finish_front(a, color).is_err() {
        return false;
    }
    let program = a.program.as_ref().unwrap();
    let names: Vec<String> = program.tests.iter().map(|t| program.def(*t).name.clone()).collect();
    let opts = BuildOptions { release, test: true, output: out.to_path_buf(), emit_llvm: None, overflow_checks: !release };
    if let Err(d) = build(a, &opts) {
        report(a, d, color);
        return false;
    }
    let selected: Vec<(usize, &String)> = names.iter().enumerate().filter(|(_, n)| filter.is_none_or(|f| n.contains(f))).collect();
    tally.filtered += names.len() - selected.len();
    println!("\nrunning {} test{} ({label})", selected.len(), if selected.len() == 1 { "" } else { "s" });
    for (i, name) in &selected {
        match std::process::Command::new(out).arg(i.to_string()).output() {
            Ok(o) if o.status.success() => {
                println!("test {name} ... ok");
                tally.passed += 1;
            }
            Ok(o) => {
                println!("test {name} ... FAILED");
                let text = String::from_utf8_lossy(&o.stderr).into_owned() + &String::from_utf8_lossy(&o.stdout);
                tally.failed.push((name.to_string(), text));
            }
            Err(e) => {
                println!("test {name} ... FAILED");
                tally.failed.push((name.to_string(), format!("could not run test binary: {e}")));
            }
        }
    }
    true
}
