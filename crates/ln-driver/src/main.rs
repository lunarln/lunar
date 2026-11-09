//! `lunar`: the Lunar compiler and project manager.
//!
//! Commands that take a file (`lunar run hello.ln`) work on that file alone.
//! Without one, they work on the project around the current directory: the
//! package there, or, at a workspace root, every member (`-p` picks one).

use clap::{Parser, Subcommand};
use ln_diag::{Diagnostic, SourceMap};
use ln_driver::project::{self, DepSpecArgs, Graph, ProjectError, Target};
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
        /// In a workspace, only this member package.
        #[arg(short, long)]
        package: Option<String>,
        /// Evaluate `#[cfg]` for this OS instead of the host (linux, macos, windows).
        #[arg(long)]
        target_os: Option<String>,
    },
    /// Compile a file, or the current project's programs, to executables.
    Build {
        file: Option<PathBuf>,
        /// In a workspace, only this member package.
        #[arg(short, long)]
        package: Option<String>,
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
        /// In a workspace, the member package whose program to run.
        #[arg(short, long)]
        package: Option<String>,
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
        /// In a workspace, only this member package.
        #[arg(short, long)]
        package: Option<String>,
        #[arg(long)]
        release: bool,
    },
    /// Add a dependency to the current package.
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
    /// Remove a dependency from the current package.
    Remove { name: String },
    /// Re-resolve dependencies, updating git dependencies to the newest
    /// commit of their branch or tag, and rewrite lunar.lock.
    Update,
    /// Remove the workspace's build output (target/).
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

/// The compiler recurses over nested expressions and macro expansions (up
/// to 128 deep), more than a main thread's stack (1 MiB on Windows) holds,
/// so it runs on a thread with a larger one. The memory is only reserved,
/// and used as needed.
const STACK_SIZE: usize = 256 << 20;

fn main() -> ExitCode {
    let worker = std::thread::Builder::new().name("lunar".into()).stack_size(STACK_SIZE).spawn(run);
    match worker.map(|w| w.join()) {
        Ok(Ok(code)) => code,
        // A panic was already reported by the panic hook.
        Ok(Err(_)) => ExitCode::from(101),
        Err(e) => {
            eprintln!("error: could not start the compiler thread: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> ExitCode {
    let cli = Cli::parse();
    let color = std::io::stderr().is_terminal();
    let result = match cli.command {
        Cmd::New { path, lib } => new_package(&path, lib, color),
        Cmd::Init { lib } => match std::env::current_dir() {
            Ok(d) => init_package(&d, lib).map_err(|e| report_project(&e, color)),
            Err(e) => Err(fail(e)),
        },
        Cmd::Check { file: Some(file), target_os, .. } => {
            let a = ln_driver::analyze_for(&file, false, target_os.as_deref());
            finish_front(&a, color)
        }
        Cmd::Check { file: None, package, target_os } => check_project(package.as_deref(), target_os.as_deref(), color),
        Cmd::Build { file: Some(file), output, release, emit_llvm, .. } => {
            let out = output.unwrap_or_else(|| default_exe(&file));
            compile_file(&file, &out, release, emit_llvm, color)
        }
        Cmd::Build { file: None, package, output, release, emit_llvm } => build_project(package.as_deref(), output, release, emit_llvm, color).map(|_| ()),
        Cmd::Run { file: Some(file), release, args, .. } => {
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
        Cmd::Run { file: None, package, release, args } => match run_project(package.as_deref(), release, color) {
            Ok(exe) => {
                status("Running", &exe.display().to_string());
                return run_program(&exe, &args);
            }
            Err(()) => Err(()),
        },
        Cmd::Test { args, package, release } => return test(&args, package.as_deref(), release, color),
        Cmd::Add { name, path, git, tag, branch, rev, package } => add(&name, DepSpecArgs { version: None, path, git, tag, branch, rev, package }, color),
        Cmd::Remove { name } => in_package(color, |dir| {
            project::remove_dependency(dir, &name)?;
            status("Removed", &name);
            resolve_at(dir, false).map(|_| ())
        }),
        Cmd::Update => in_package(color, |dir| resolve_at(dir, true).map(|_| ())),
        Cmd::Clean => in_package(color, |dir| {
            let ws = project::find_workspace(dir)?;
            let target = ws.root.join("target");
            if target.exists() {
                std::fs::remove_dir_all(&target)
                    .map_err(|e| ProjectError::new(ln_diag::codes::build::IO, format!("could not remove `{}`: {e}", target.display())))?;
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

/// Render a project error like a compiler diagnostic, pointing into
/// `lunar.toml` when it can.
fn report_project(e: &ProjectError, color: bool) {
    let mut map = SourceMap::new();
    let d = e.to_diagnostic(&mut map);
    let diags = [d];
    emit_diagnostics(&map, &diags, color);
    if let Some(hint) = explain::hint(&diags) {
        eprintln!("\n{hint}");
    }
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

fn new_package(path: &Path, lib: bool, color: bool) -> Result<(), ()> {
    if path.exists() && path.read_dir().is_ok_and(|mut d| d.next().is_some()) {
        fail(format!("`{}` already exists and is not empty", path.display()));
        return Err(());
    }
    std::fs::create_dir_all(path).map_err(|e| fail(format!("could not create `{}`: {e}", path.display())))?;
    init_package(path, lib).map_err(|e| report_project(&e, color))
}

fn init_package(dir: &Path, lib: bool) -> Result<(), ProjectError> {
    let name = project::init(dir, lib)?;
    status("Created", &format!("{} `{name}` package", if lib { "library" } else { "program" }));
    Ok(())
}

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Run `f` on the current directory, reporting project errors.
fn in_package(color: bool, f: impl FnOnce(&Path) -> Result<(), ProjectError>) -> Result<(), ()> {
    f(&cwd()).map_err(|e| report_project(&e, color))
}

fn resolve_at(dir: &Path, update: bool) -> Result<Graph, ProjectError> {
    let ws = project::find_workspace(dir)?;
    project::resolve(ws, update, &mut |msg| {
        let (verb, rest) = msg.split_once(' ').unwrap_or((msg, ""));
        status(verb, rest);
    })
}

/// The project graph around the current directory.
fn load(color: bool) -> Result<Graph, ()> {
    resolve_at(&cwd(), false).map_err(|e| report_project(&e, color))
}

/// The member packages a command works on: `-p <name>`, else the package
/// the command started in, else (at a workspace root) every member.
fn select(graph: &Graph, package: Option<&str>) -> Result<Vec<usize>, ProjectError> {
    if let Some(name) = package {
        return graph.member(name).map(|i| vec![i]).ok_or_else(|| {
            let names: Vec<&str> = graph.packages[..graph.members].iter().map(|p| p.manifest.name.as_str()).collect();
            ProjectError::new(ln_diag::codes::manifest::WORKSPACE, format!("no package `{name}` in this workspace"))
                .note(format!("its packages are: {}", names.join(", ")))
        });
    }
    Ok(match graph.current() {
        Some(i) => vec![i],
        None => (0..graph.members).collect(),
    })
}

fn add(name: &str, mut spec: DepSpecArgs, color: bool) -> Result<(), ()> {
    let (name, version) = match name.split_once('@') {
        Some((n, v)) => (n, Some(v.to_string())),
        None => (name, None),
    };
    spec.version = version;
    if spec.path.is_none() && spec.git.is_none() && spec.version.is_none() {
        fail(format!("say where `{name}` comes from: `--path <dir>`, `--git <url>`, or a registry version (`{name}@1.0`)"));
        return Err(());
    }
    in_package(color, |dir| {
        let pkg = project::find_project(dir).unwrap_or_else(|| dir.to_path_buf());
        project::add_dependency(&pkg, name, &spec)?;
        status("Added", name);
        resolve_at(dir, false).map(|_| ())
    })
}

/// Analyze one target of a member package.
fn analyze_target(graph: &Graph, pkg: usize, target: &Target, test: bool, target_os: Option<&str>) -> Result<Analysis, ProjectError> {
    let craters = project::craters(graph, pkg, target)?;
    Ok(analyze_craters(&craters, &AnalyzeOptions { test, target_os, ..Default::default() }))
}

fn check_project(package: Option<&str>, target_os: Option<&str>, color: bool) -> Result<(), ()> {
    let graph = load(color)?;
    let pkgs = select(&graph, package).map_err(|e| report_project(&e, color))?;
    let mut ok = true;
    for pkg in pkgs {
        let m = graph.packages[pkg].manifest.clone();
        status("Checking", &format!("{} v{}", m.name, m.version));
        let mut targets = Vec::new();
        if m.lib_root().is_some() {
            targets.push(Target::Lib);
        }
        if m.main_root().is_some() {
            targets.push(Target::Main);
        }
        for t in &targets {
            let a = analyze_target(&graph, pkg, t, false, target_os).map_err(|e| report_project(&e, color))?;
            ok &= finish_front(&a, color).is_ok();
        }
    }
    if ok {
        status("Finished", "checking");
        Ok(())
    } else {
        Err(())
    }
}

fn profile(release: bool) -> &'static str {
    if release { "release" } else { "debug" }
}

/// Build the selected packages' programs (and check library-only packages);
/// returns the executables built.
fn build_project(package: Option<&str>, output: Option<PathBuf>, release: bool, emit_llvm: Option<PathBuf>, color: bool) -> Result<Vec<PathBuf>, ()> {
    let graph = load(color)?;
    let pkgs = select(&graph, package).map_err(|e| report_project(&e, color))?;
    let programs = pkgs.iter().filter(|&&p| graph.packages[p].manifest.main_root().is_some()).count();
    if output.is_some() && programs > 1 {
        fail("`-o` names one file, but this builds several programs; pick one with `-p <package>`");
        return Err(());
    }
    let mut built = Vec::new();
    for pkg in pkgs {
        let m = graph.packages[pkg].manifest.clone();
        let Some(main) = m.main_root() else {
            // A library on its own has no executable: type-check it.
            status("Checking", &format!("{} v{} (library)", m.name, m.version));
            let a = analyze_target(&graph, pkg, &Target::Lib, false, None).map_err(|e| report_project(&e, color))?;
            finish_front(&a, color)?;
            continue;
        };
        status("Compiling", &format!("{} v{} ({})", m.name, m.version, m.dir.display()));
        let out = output
            .clone()
            .unwrap_or_else(|| graph.workspace.root.join("target").join(profile(release)).join(format!("{}{}", m.name, std::env::consts::EXE_SUFFIX)));
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir).map_err(|e| fail(format!("could not create `{}`: {e}", dir.display())))?;
        }
        let mut a = analyze_target(&graph, pkg, &Target::Main, false, None).map_err(|e| report_project(&e, color))?;
        if a.program.is_some()
            && let Some(d) = ln_driver::check_main(&a, &main, false)
        {
            a.diags.insert(0, d);
        }
        finish_front(&a, color)?;
        let opts = BuildOptions { release, test: false, output: out.clone(), emit_llvm: emit_llvm.clone(), overflow_checks: !release };
        build(&a, &opts).map_err(|d| report(&a, d, color))?;
        built.push(out);
    }
    status("Finished", &format!("`{}` profile", profile(release)));
    Ok(built)
}

/// Build the one program to run.
fn run_project(package: Option<&str>, release: bool, color: bool) -> Result<PathBuf, ()> {
    let graph = load(color)?;
    let pkgs = select(&graph, package).map_err(|e| report_project(&e, color))?;
    let programs: Vec<usize> = pkgs.into_iter().filter(|&p| graph.packages[p].manifest.main_root().is_some()).collect();
    let name = match programs.as_slice() {
        [p] => graph.packages[*p].manifest.name.clone(),
        [] => {
            let e = match package.or(graph.current().map(|i| graph.packages[i].manifest.name.as_str())) {
                Some(n) => ProjectError::new(ln_diag::codes::manifest::NO_TARGET, format!("package `{n}` has no program (`src/main.ln`) to run")),
                None => ProjectError::new(ln_diag::codes::manifest::NO_TARGET, "no package in this workspace has a program (`src/main.ln`) to run"),
            };
            report_project(&e, color);
            return Err(());
        }
        many => {
            let names: Vec<&str> = many.iter().map(|p| graph.packages[*p].manifest.name.as_str()).collect();
            let e = ProjectError::new(ln_diag::codes::manifest::WORKSPACE, "this workspace has several programs; pick one to run")
                .note(format!("programs: {}", names.join(", ")))
                .help(format!("`lunar run -p {}`", names[0]));
            report_project(&e, color);
            return Err(());
        }
    };
    let built = build_project(Some(&name), None, release, None, color)?;
    built.into_iter().next().ok_or(())
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
fn test(args: &[String], package: Option<&str>, release: bool, color: bool) -> ExitCode {
    let file_mode = args.first().is_some_and(|a| a.ends_with(".ln"));
    let mut tally = TestTally::default();
    let ok = if file_mode {
        let file = PathBuf::from(&args[0]);
        let a = analyze(&file, true);
        run_tests(&a, &file.display().to_string(), &temp_exe(&file, "test"), args.get(1).map(String::as_str), release, color, &mut tally)
    } else {
        let filter = args.first().map(String::as_str);
        match test_project(package, filter, release, color, &mut tally) {
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

/// Run the tests of each selected package's library, program and `tests/*.ln`.
fn test_project(package: Option<&str>, filter: Option<&str>, release: bool, color: bool, tally: &mut TestTally) -> Result<bool, ()> {
    let graph = load(color)?;
    let pkgs = select(&graph, package).map_err(|e| report_project(&e, color))?;
    let dir = graph.workspace.root.join("target").join(profile(release)).join("tests");
    std::fs::create_dir_all(&dir).map_err(|e| fail(format!("could not create `{}`: {e}", dir.display())))?;
    let mut ok = true;
    for pkg in pkgs {
        let m = graph.packages[pkg].manifest.clone();
        status("Testing", &format!("{} v{}", m.name, m.version));
        let mut targets: Vec<(Target, String, String)> = Vec::new();
        if m.lib_root().is_some() {
            targets.push((Target::Lib, format!("{}: src/lib.ln", m.name), "lib".into()));
        }
        if m.main_root().is_some() {
            targets.push((Target::Main, format!("{}: src/main.ln", m.name), "main".into()));
        }
        for f in m.test_files() {
            let stem = f.file_stem().unwrap().to_string_lossy().into_owned();
            targets.push((Target::TestFile(f.clone()), format!("{}: tests/{stem}.ln", m.name), format!("test-{stem}")));
        }
        for (t, label, exe) in targets {
            let a = analyze_target(&graph, pkg, &t, true, None).map_err(|e| report_project(&e, color))?;
            let out = dir.join(format!("{}-{exe}{}", m.name, std::env::consts::EXE_SUFFIX));
            ok &= run_tests(&a, &label, &out, filter, release, color, tally);
        }
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
