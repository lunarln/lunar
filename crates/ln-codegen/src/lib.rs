//! LLVM code generation (in progress).
use std::path::{Path, PathBuf};
#[derive(Clone, Debug)]
pub struct Options { pub opt_level: u32, pub test: bool, pub overflow_checks: bool, pub emit_llvm: Option<PathBuf> }
pub fn compile(_p: &ln_sema::Program, _m: &ln_diag::SourceMap, _o: &Options, _out: &Path) -> Result<(), String> { Err("codegen not implemented yet".into()) }
