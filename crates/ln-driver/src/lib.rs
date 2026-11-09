//! The Lunar compiler pipeline: load sources, analyze, generate code, link.

pub mod link;

use ln_diag::{Diagnostic, SourceMap};
use ln_parser::IdGen;
use ln_sema::Program;
use ln_sema::loader::{self, CrateSource, DiskFiles, FileProvider, MemFiles};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/std_files.rs"));
}

/// The standard library sources: from `$LUNAR_STD_DIR` if set (for working
/// on std itself), otherwise the copy embedded in the compiler.
fn std_provider() -> (Box<dyn FileProvider>, PathBuf, &'static str) {
    if let Ok(dir) = std::env::var("LUNAR_STD_DIR") {
        return (Box::new(DiskFiles), PathBuf::from(dir), "");
    }
    let files: HashMap<PathBuf, String> = embedded::STD_FILES.iter().map(|(p, s)| (PathBuf::from(p), s.to_string())).collect();
    (Box::new(MemFiles(files)), PathBuf::new(), "<std>/")
}

/// Result of the front end.
pub struct Analysis {
    pub map: SourceMap,
    pub program: Option<Program>,
    pub diags: Vec<Diagnostic>,
}

/// Parse and analyze `path` together with `core` and `std`.
pub fn analyze(path: &Path, test: bool) -> Analysis {
    let mut map = SourceMap::new();
    let mut diags = Vec::new();
    let mut ids = IdGen::new();
    let user_name = path.file_stem().map(|s| sanitize(&s.to_string_lossy())).unwrap_or_else(|| "main".into());
    let mut crates: Vec<CrateSource> = Vec::new();
    match loader::load_crate(&user_name, path, "", &DiskFiles, &mut map, &mut ids, &mut diags) {
        Some(c) => crates.push(c),
        None => {
            diags.push(Diagnostic::error(ln_diag::codes::build::IO, ln_diag::Span::DUMMY, format!("could not read `{}`", path.display())));
            return Analysis { map, program: None, diags };
        }
    }
    let (provider, base, prefix) = std_provider();
    for name in ["core", "std"] {
        let root = base.join(name).join("lib.ln");
        match loader::load_crate(name, &root, prefix, provider.as_ref(), &mut map, &mut ids, &mut diags) {
            Some(c) => crates.push(c),
            None => {
                diags.push(Diagnostic::error(ln_diag::codes::build::IO, ln_diag::Span::DUMMY, format!("standard library crate `{name}` not found at {}", root.display())));
                return Analysis { map, program: None, diags };
            }
        }
    }
    if diags.iter().any(|d| d.is_error()) {
        return Analysis { map, program: None, diags };
    }
    let (program, mut d) = ln_sema::analyze(&crates, ln_sema::Options::host(test));
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

/// Generate code for an analyzed program and link it into an executable.
pub fn build(analysis: &Analysis, opts: &BuildOptions) -> Result<(), String> {
    let program = analysis.program.as_ref().ok_or("analysis failed")?;
    let obj_path = opts.output.with_extension(if cfg!(windows) { "obj" } else { "o" });
    let cg = ln_codegen::Options {
        opt_level: if opts.release { 2 } else { 0 },
        test: opts.test,
        overflow_checks: opts.overflow_checks,
        emit_llvm: opts.emit_llvm.clone(),
    };
    ln_codegen::compile(program, &analysis.map, &cg, &obj_path)?;
    let res = link::link(&obj_path, &opts.output);
    let _ = std::fs::remove_file(&obj_path);
    res
}
