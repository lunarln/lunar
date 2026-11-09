//! Loading a crate's module tree from disk or from the embedded standard library.

use ln_ast as ast;
use ln_diag::{Diagnostic, FileId, SourceMap, Span, codes::name};
use ln_parser::IdGen;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Where module source text comes from.
pub trait FileProvider {
    fn read(&self, path: &Path) -> Option<String>;
}

/// Reads from the real file system.
pub struct DiskFiles;

impl FileProvider for DiskFiles {
    fn read(&self, path: &Path) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}

/// An in-memory file set (used for the embedded standard library).
pub struct MemFiles(pub HashMap<PathBuf, String>);

impl FileProvider for MemFiles {
    fn read(&self, path: &Path) -> Option<String> {
        self.0.get(&normalize(path)).cloned()
    }
}

pub fn normalize(p: &Path) -> PathBuf {
    let s = p.to_string_lossy().replace('\\', "/");
    PathBuf::from(s)
}

/// A parsed module and the out-of-line child modules (`mod foo;`) it declares.
#[derive(Debug)]
pub struct ModSource {
    pub file: FileId,
    pub ast: ast::File,
    /// Children keyed by the NodeId-free path of `mod` declarations: the
    /// chain of inline module names leading to the declaration, then its name.
    pub children: HashMap<Vec<String>, ModSource>,
}

#[derive(Debug)]
pub struct CrateSource {
    pub name: String,
    pub root: ModSource,
}

/// Load the crate rooted at `root_path`, following `mod foo;` declarations to
/// `foo.ln` or `foo/mod.ln` relative to the declaring file.
pub fn load_crate(
    name: &str,
    root_path: &Path,
    display_prefix: &str,
    files: &dyn FileProvider,
    map: &mut SourceMap,
    ids: &mut IdGen,
    diags: &mut Vec<Diagnostic>,
) -> Option<CrateSource> {
    let text = files.read(root_path)?;
    let root = load_mod(root_path, text, true, display_prefix, files, map, ids, diags);
    Some(CrateSource { name: name.to_string(), root })
}

#[allow(clippy::too_many_arguments)]
fn load_mod(
    path: &Path,
    text: String,
    is_dir_owner: bool,
    display_prefix: &str,
    files: &dyn FileProvider,
    map: &mut SourceMap,
    ids: &mut IdGen,
    diags: &mut Vec<Diagnostic>,
) -> ModSource {
    let display = format!("{display_prefix}{}", normalize(path).display());
    let file = map.add(display, Some(path.to_path_buf()), text);
    let src = map.get(file).text.clone();
    let (ast, mut d) = ln_parser::parse_file(&src, file, ids);
    diags.append(&mut d);

    // Directory in which child modules of this file live.
    let base_dir = if is_dir_owner {
        path.parent().map(Path::to_path_buf).unwrap_or_default()
    } else {
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        path.parent().map(|p| p.join(&stem)).unwrap_or_else(|| PathBuf::from(stem))
    };

    let mut children = HashMap::new();
    let mut decls = Vec::new();
    collect_mod_decls(&ast.items, &mut Vec::new(), &mut decls);
    for (chain, ident) in decls {
        let mut dir = base_dir.clone();
        for c in &chain {
            dir = dir.join(c);
        }
        let flat = dir.join(format!("{}.ln", ident.name));
        let nested = dir.join(&ident.name).join("mod.ln");
        let (child_path, text, owner) = if let Some(t) = files.read(&flat) {
            (flat, t, false)
        } else if let Some(t) = files.read(&nested) {
            (nested, t, true)
        } else {
            diags.push(Diagnostic::error(name::MODULE_FILE, ident.span, format!("file not found for module `{}`", ident.name)).with_help(format!(
                "create `{}` or `{}`",
                normalize(&flat).display(),
                normalize(&nested).display()
            )));
            continue;
        };
        let child = load_mod(&child_path, text, owner, display_prefix, files, map, ids, diags);
        let mut key = chain.clone();
        key.push(ident.name.clone());
        children.insert(key, child);
    }
    ModSource { file, ast, children }
}

fn collect_mod_decls(items: &[ast::Item], chain: &mut Vec<String>, out: &mut Vec<(Vec<String>, ast::Ident)>) {
    for item in items {
        if let ast::ItemKind::Mod(m) = &item.kind {
            match &m.items {
                None => out.push((chain.clone(), m.name.clone())),
                Some(inner) => {
                    chain.push(m.name.name.clone());
                    collect_mod_decls(inner, chain, out);
                    chain.pop();
                }
            }
        }
    }
}

/// Dummy span helper for synthesized items.
pub fn dummy() -> Span {
    Span::DUMMY
}
