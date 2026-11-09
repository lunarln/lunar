//! The Lunar compiler pipeline: load sources, analyze, generate code, link.

pub mod ide;
pub mod link;
pub mod lsp;

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
fn std_provider(std_dir: Option<&Path>) -> (Box<dyn FileProvider>, PathBuf, &'static str) {
    if let Some(dir) = std_dir {
        return (Box::new(DiskFiles), dir.to_path_buf(), "");
    }
    if let Ok(dir) = std::env::var("LUNAR_STD_DIR") {
        return (Box::new(DiskFiles), PathBuf::from(dir), "");
    }
    let files: HashMap<PathBuf, String> = embedded::STD_FILES.iter().map(|(p, s)| (PathBuf::from(p), s.to_string())).collect();
    (Box::new(MemFiles(files)), PathBuf::new(), "<std>/")
}

/// Write the embedded standard library to `dir` (as `dir/core/...`,
/// `dir/std/...`), so editors can open its files.
pub fn write_std_to(dir: &Path) -> std::io::Result<()> {
    for (rel, text) in embedded::STD_FILES {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if std::fs::read_to_string(&path).ok().as_deref() != Some(text) {
            if let Ok(meta) = std::fs::metadata(&path) {
                let mut perm = meta.permissions();
                #[allow(clippy::permissions_set_readonly_false)]
                perm.set_readonly(false);
                std::fs::set_permissions(&path, perm)?;
            }
            std::fs::write(&path, text)?;
        }
        // A copy: make accidental edits fail instead of silently doing nothing.
        let mut perm = std::fs::metadata(&path)?.permissions();
        perm.set_readonly(true);
        std::fs::set_permissions(&path, perm)?;
    }
    Ok(())
}

/// A hash of the embedded standard library, to version on-disk copies.
pub fn std_hash() -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for (rel, text) in embedded::STD_FILES {
        for b in rel.bytes().chain(text.bytes()) {
            h = (h ^ b as u64).wrapping_mul(0x100000001b3);
        }
    }
    h
}

/// Result of the front end.
pub struct Analysis {
    pub map: SourceMap,
    /// The analyzed program: present when there were no errors, or always
    /// with [`AnalyzeOptions::keep_going`] (as far as analysis got).
    pub program: Option<Program>,
    pub diags: Vec<Diagnostic>,
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

/// The general front-end entry point.
pub fn analyze_with(path: &Path, o: &AnalyzeOptions) -> Analysis {
    let mut map = SourceMap::new();
    let mut diags = Vec::new();
    let mut ids = IdGen::new();
    let user_name = path.file_stem().map(|s| sanitize(&s.to_string_lossy())).unwrap_or_else(|| "main".into());
    let mut crates: Vec<CrateSource> = Vec::new();
    let empty = HashMap::new();
    let user_files = Overlay(o.overlay.unwrap_or(&empty));
    match loader::load_crate(&user_name, path, "", &user_files, &mut map, &mut ids, &mut diags) {
        Some(c) => crates.push(c),
        None => {
            diags.push(Diagnostic::error(ln_diag::codes::build::IO, ln_diag::Span::DUMMY, format!("could not read `{}`", path.display())));
            return Analysis { map, program: None, diags };
        }
    }
    let (provider, base, prefix) = std_provider(o.std_dir);
    for name in ["core", "std"] {
        let root = base.join(name).join("lib.ln");
        match loader::load_crate(name, &root, prefix, provider.as_ref(), &mut map, &mut ids, &mut diags) {
            Some(c) => crates.push(c),
            None => {
                diags.push(Diagnostic::error(
                    ln_diag::codes::build::IO,
                    ln_diag::Span::DUMMY,
                    format!("standard library crate `{name}` not found at {}", root.display()),
                ));
                return Analysis { map, program: None, diags };
            }
        }
    }
    if diags.iter().any(|d| d.is_error()) && !o.keep_going {
        return Analysis { map, program: None, diags };
    }
    let mut opts = ln_sema::Options::host(o.test);
    opts.keep_going = o.keep_going;
    if let Some(os) = o.target_os {
        opts.target_os = os.to_string();
        opts.target_family = if os == "windows" { "windows" } else { "unix" }.to_string();
    }
    let (program, mut d) = ln_sema::analyze(&crates, opts);
    diags.append(&mut d);
    Analysis { map, program, diags }
}

fn sanitize(s: &str) -> String {
    let mut out: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if out.is_empty() || out.chars().next().unwrap().is_ascii_digit() {
        out.insert(0, '_');
    }
    out
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

/// Generate code for an analyzed program and link it into an executable.
pub fn build(analysis: &Analysis, opts: &BuildOptions) -> Result<(), String> {
    let program = analysis.program.as_ref().ok_or("analysis failed")?;
    let obj_path = opts.output.with_extension(if cfg!(windows) { "obj" } else { "o" });
    ln_codegen::compile(program, &analysis.map, &codegen_options(opts), &obj_path)?;
    let res = link::link(&obj_path, &opts.output);
    let _ = std::fs::remove_file(&obj_path);
    res
}

/// Generate code for an analyzed program and return its optimized LLVM IR
/// as text, without writing an object file or linking. `opts.output` is
/// ignored.
pub fn emit_llvm(analysis: &Analysis, opts: &BuildOptions) -> Result<String, String> {
    let program = analysis.program.as_ref().ok_or("analysis failed")?;
    ln_codegen::emit_ir(program, &analysis.map, &codegen_options(opts))
}
