//! The Lunar compiler pipeline: load sources, analyze, generate code, link.

pub mod ide;
pub mod link;
pub mod lsp;
pub mod project;

use ln_diag::{Diagnostic, SourceMap};
use ln_parser::IdGen;
use ln_sema::Program;
use ln_sema::loader::{self, CrateSource, DiskFiles, FileProvider, MemFiles};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/std_files.rs"));
}

/// The standard library sources: from `std_dir` or `$LUNAR_STD_DIR` if set
/// (for working on std itself), otherwise the copy embedded in the compiler.
/// An on-disk std sees `overlay` (unsaved editor buffers) like user files do.
fn std_provider<'a>(std_dir: Option<&Path>, overlay: &'a HashMap<PathBuf, String>) -> (Box<dyn FileProvider + 'a>, PathBuf, &'static str) {
    if let Some(dir) = std_dir {
        return (Box::new(Overlay(overlay)), dir.to_path_buf(), "");
    }
    if let Ok(dir) = std::env::var("LUNAR_STD_DIR") {
        return (Box::new(DiskFiles), PathBuf::from(dir), "");
    }
    let files: HashMap<PathBuf, String> = embedded::STD_FILES.iter().map(|(p, s)| (PathBuf::from(p), s.to_string())).collect();
    (Box::new(MemFiles(files)), PathBuf::new(), "<std>/")
}

/// The embedded standard library: (path relative to the std root, such as
/// `core/string.ln`, source text). Editors show these as virtual documents.
pub fn std_files() -> impl Iterator<Item = (&'static str, &'static str)> {
    embedded::STD_FILES.iter().copied()
}

/// Result of the front end.
pub struct Analysis {
    pub map: SourceMap,
    /// The analyzed program: present when there were no errors, or always
    /// with [`AnalyzeOptions::keep_going`] (as far as analysis got).
    pub program: Option<Program>,
    pub diags: Vec<Diagnostic>,
    /// The parsed craters (user craters first, then `core` and `std`), for
    /// editor queries that print declarations.
    pub sources: Vec<CrateSource>,
}

/// Options for [`analyze_with`].
#[derive(Default)]
pub struct AnalyzeOptions<'a> {
    /// Analyze for the `#[test]` harness.
    pub test: bool,
    /// Evaluate `#[cfg]` for this OS instead of the host.
    pub target_os: Option<&'a str>,
    /// Keep analyzing after errors and return a partial program (editors).
    pub keep_going: bool,
    /// File contents that replace what is on disk (unsaved editor buffers).
    pub overlay: Option<&'a HashMap<PathBuf, String>>,
    /// Read the standard library from this directory.
    pub std_dir: Option<&'a Path>,
}

/// Disk files, with some replaced by in-memory contents.
struct Overlay<'a>(&'a HashMap<PathBuf, String>);

/// Canonical key for comparing paths (separators, and case on Windows).
pub fn path_key(p: &Path) -> PathBuf {
    let s = loader::normalize(p).to_string_lossy().into_owned();
    PathBuf::from(if cfg!(windows) { s.to_lowercase() } else { s })
}

impl FileProvider for Overlay<'_> {
    fn read(&self, path: &Path) -> Option<String> {
        self.0.get(&path_key(path)).cloned().or_else(|| DiskFiles.read(path))
    }
}

/// Parse and analyze `path` together with `core` and `std` for the host.
pub fn analyze(path: &Path, test: bool) -> Analysis {
    analyze_with(path, &AnalyzeOptions { test, ..Default::default() })
}

/// Like [`analyze`], but evaluating `#[cfg]` for another OS (`linux`,
/// `macos`, `windows`). Only useful for checking; code generation always
/// targets the host.
pub fn analyze_for(path: &Path, test: bool, target_os: Option<&str>) -> Analysis {
    analyze_with(path, &AnalyzeOptions { test, target_os, ..Default::default() })
}

/// The front end for a single file (plus `core` and `std`).
pub fn analyze_with(path: &Path, o: &AnalyzeOptions) -> Analysis {
    let name = path.file_stem().map(|s| sanitize(&s.to_string_lossy())).unwrap_or_else(|| "main".into());
    analyze_craters(&[CraterSpec { name, root: path.to_path_buf(), deps: Vec::new() }], o)
}

/// One crater to compile: its name, root file, and the craters it depends
/// on (the name used in its paths, and an index into the same list).
#[derive(Clone, Debug)]
pub struct CraterSpec {
    pub name: String,
    pub root: PathBuf,
    pub deps: Vec<(String, usize)>,
}

/// Analyze a graph of craters together with `core` and `std`. The first
/// crater is the one being built (its `main` or its tests); the others are
/// its dependencies, direct and indirect.
pub fn analyze_craters(craters: &[CraterSpec], o: &AnalyzeOptions) -> Analysis {
    let mut map = SourceMap::new();
    let mut diags = Vec::new();
    let mut ids = IdGen::new();
    let mut crates: Vec<CrateSource> = Vec::new();
    let empty = HashMap::new();
    let user_files = Overlay(o.overlay.unwrap_or(&empty));
    for spec in craters {
        match loader::load_crate(&spec.name, &spec.root, "", &user_files, &mut map, &mut ids, &mut diags) {
            Some(mut c) => {
                c.deps = spec.deps.clone();
                crates.push(c);
            }
            None => {
                diags.push(Diagnostic::error(ln_diag::codes::build::IO, ln_diag::Span::DUMMY, format!("could not read `{}`", spec.root.display())));
                return Analysis { map, program: None, diags, sources: crates };
            }
        }
    }
    let (provider, base, prefix) = std_provider(o.std_dir, o.overlay.unwrap_or(&empty));
    for name in ["core", "std"] {
        let root = base.join(name).join("lib.ln");
        match loader::load_crate(name, &root, prefix, provider.as_ref(), &mut map, &mut ids, &mut diags) {
            Some(c) => crates.push(c),
            None => {
                diags.push(Diagnostic::error(
                    ln_diag::codes::build::IO,
                    ln_diag::Span::DUMMY,
                    format!("standard library crater `{name}` not found at {}", root.display()),
                ));
                return Analysis { map, program: None, diags, sources: crates };
            }
        }
    }
    if diags.iter().any(|d| d.is_error()) && !o.keep_going {
        return Analysis { map, program: None, diags, sources: crates };
    }
    let mut opts = ln_sema::Options::host(o.test);
    opts.keep_going = o.keep_going;
    if let Some(os) = o.target_os {
        opts.target_os = os.to_string();
        opts.target_family = if os == "windows" { "windows" } else { "unix" }.to_string();
    }
    let (program, mut d) = ln_sema::analyze(&crates, &mut map, opts);
    diags.append(&mut d);
    Analysis { map, program, diags, sources: crates }
}

fn sanitize(s: &str) -> String {
    let mut out: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if out.is_empty() || out.chars().next().unwrap().is_ascii_digit() {
        out.insert(0, '_');
    }
    out
}

/// The root of the standard library source tree that `path` belongs to: an
/// ancestor directory with `core/lib.ln` and `std/lib.ln`, with `path` under
/// its `core/` or `std/`. Such files are the standard library being worked
/// on, not programs.
pub fn std_tree_of(path: &Path) -> Option<PathBuf> {
    path.ancestors().skip(1).find_map(|dir| {
        let is_tree = dir.join("core").join("lib.ln").is_file() && dir.join("std").join("lib.ln").is_file();
        let inside = path.starts_with(dir.join("core")) || path.starts_with(dir.join("std"));
        (is_tree && inside).then(|| dir.to_path_buf())
    })
}

/// Analyze the standard library tree at `dir` on its own (from disk, with
/// `o.overlay` for unsaved buffers), for editors working on std itself.
pub fn analyze_std_tree(dir: &Path, o: &AnalyzeOptions) -> Analysis {
    let o = AnalyzeOptions { std_dir: Some(dir), test: o.test, target_os: o.target_os, keep_going: o.keep_going, overlay: o.overlay };
    analyze_craters(&[], &o)
}

/// Print diagnostics to stderr.
pub fn emit_diagnostics(map: &SourceMap, diags: &[Diagnostic], color: bool) {
    let mut err = std::io::stderr();
    ln_diag::render(map, diags, color, &mut err);
}

/// Options for building an executable.
#[derive(Clone, Debug)]
pub struct BuildOptions {
    pub release: bool,
    pub test: bool,
    pub output: PathBuf,
    pub emit_llvm: Option<PathBuf>,
    pub overflow_checks: bool,
}

fn codegen_options(opts: &BuildOptions) -> ln_codegen::Options {
    ln_codegen::Options {
        opt_level: if opts.release { 2 } else { 0 },
        test: opts.test,
        overflow_checks: opts.overflow_checks,
        emit_llvm: opts.emit_llvm.clone(),
    }
}

/// `build::no_main` if a program that is built as an executable (not as a
/// test harness) has no `main` function.
pub fn check_main(analysis: &Analysis, file: &Path, test: bool) -> Option<Diagnostic> {
    let program = analysis.program.as_ref()?;
    if test || program.main.is_some() {
        return None;
    }
    Some(
        Diagnostic::error(ln_diag::codes::build::NO_MAIN, ln_diag::Span::DUMMY, format!("`{}` has no `main` function", file.display()))
            .with_help("add `fn main() { ... }` to the file; to run its `#[test]` functions instead, use `lunar test`"),
    )
}

/// A code generation failure as a `build::llvm` diagnostic. The first line
/// of `msg` is the message; any further lines (such as the LLVM verifier's
/// output) become a note.
fn codegen_error(msg: String) -> Diagnostic {
    let (first, rest) = msg.split_once('\n').unwrap_or((&msg, ""));
    let d = Diagnostic::error(ln_diag::codes::build::LLVM, ln_diag::Span::DUMMY, first.trim_end_matches(':'));
    if rest.trim().is_empty() { d } else { d.with_note(rest.trim_end()) }
}

fn analyzed(analysis: &Analysis) -> Result<&Program, Diagnostic> {
    analysis.program.as_ref().ok_or_else(|| codegen_error("cannot generate code: the program has errors".into()))
}

/// Generate code for an analyzed program and link it into an executable.
/// Failures are `build::llvm` and `build::link` diagnostics.
pub fn build(analysis: &Analysis, opts: &BuildOptions) -> Result<(), Diagnostic> {
    let program = analyzed(analysis)?;
    let obj_path = opts.output.with_extension(if cfg!(windows) { "obj" } else { "o" });
    ln_codegen::compile(program, &analysis.map, &codegen_options(opts), &obj_path).map_err(codegen_error)?;
    let res = link::link(&obj_path, &opts.output);
    let _ = std::fs::remove_file(&obj_path);
    res
}

/// Generate code for an analyzed program and return its optimized LLVM IR
/// as text, without writing an object file or linking. `opts.output` is
/// ignored.
pub fn emit_llvm(analysis: &Analysis, opts: &BuildOptions) -> Result<String, Diagnostic> {
    let program = analyzed(analysis)?;
    ln_codegen::emit_ir(program, &analysis.map, &codegen_options(opts)).map_err(codegen_error)
}
