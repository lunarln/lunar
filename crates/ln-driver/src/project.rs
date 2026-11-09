//! Projects: `lunar.toml` manifests, workspaces, the package graph,
//! `lunar.lock`, and fetching git dependencies.
//!
//! A *package* is a directory with a `lunar.toml` that has a `[package]`
//! section. It contains up to two craters: a library (`src/lib.ln`) that
//! other packages can depend on, and a program (`src/main.ln`). Tests live in
//! those craters (`#[test]`) and in `tests/*.ln`, each of which is a crater
//! that can use the library.
//!
//! A *workspace* is a `lunar.toml` with a `[workspace]` section listing member
//! packages. Members share one `lunar.lock` and one `target/` at the
//! workspace root. A package that isn't a member of any workspace is a
//! workspace of its own.
//!
//! Dependencies come from a path or a git repository. Registry dependencies
//! (`json = "1.2"`) are parsed, but the registry does not exist yet.
//!
//! Errors carry a `manifest::` code and, where possible, the position in
//! `lunar.toml` they are about, so they render like compiler diagnostics.

use crate::CraterSpec;
use ln_diag::codes::manifest as code;
use ln_diag::{Code, Diagnostic, SourceMap, Span};
use semver::{Version, VersionReq};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::Command;
use toml::Spanned;

pub const MANIFEST: &str = "lunar.toml";
pub const LOCKFILE: &str = "lunar.lock";

// ===================================================================== errors

/// A project error: a `manifest::` code, a message, and the place in a
/// manifest or lockfile it is about.
#[derive(Clone, Debug)]
pub struct ProjectError {
    pub code: Code,
    pub message: String,
    pub at: Option<(PathBuf, Range<usize>)>,
    pub label: Option<String>,
    pub notes: Vec<String>,
    pub helps: Vec<String>,
}

pub type Result<T> = std::result::Result<T, ProjectError>;

impl ProjectError {
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        ProjectError { code, message: message.into(), at: None, label: None, notes: Vec::new(), helps: Vec::new() }
    }

    pub fn at(mut self, file: &Path, span: Range<usize>) -> Self {
        self.at = Some((file.to_path_buf(), span));
        self
    }

    pub fn label(mut self, l: impl Into<String>) -> Self {
        self.label = Some(l.into());
        self
    }

    pub fn note(mut self, n: impl Into<String>) -> Self {
        self.notes.push(n.into());
        self
    }

    pub fn help(mut self, h: impl Into<String>) -> Self {
        self.helps.push(h.into());
        self
    }

    /// The error as a diagnostic, adding the file it points into to `map`.
    pub fn to_diagnostic(&self, map: &mut SourceMap) -> Diagnostic {
        let span = match &self.at {
            Some((path, range)) => match std::fs::read_to_string(path) {
                Ok(text) => {
                    let end = range.end.min(text.len());
                    let start = range.start.min(end);
                    let file = map.add(path.display().to_string(), Some(path.clone()), text);
                    Span::new(file, start as u32, end as u32)
                }
                Err(_) => Span::DUMMY,
            },
            None => Span::DUMMY,
        };
        let mut d = Diagnostic::error(self.code, span, self.message.clone());
        if let Some(l) = &self.label {
            d = d.with_label(l.clone());
        }
        for n in &self.notes {
            d = d.with_note(n.clone());
        }
        for h in &self.helps {
            d = d.with_help(h.clone());
        }
        d
    }
}

impl std::fmt::Display for ProjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// An I/O failure while handling `path`.
fn io_error(path: &Path, e: std::io::Error) -> ProjectError {
    ProjectError::new(ln_diag::codes::build::IO, format!("could not access `{}`: {e}", path.display()))
}

// ===================================================================== manifest

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    package: Option<RawPackage>,
    workspace: Option<RawWorkspace>,
    #[serde(default)]
    dependencies: BTreeMap<Spanned<String>, Spanned<RawDep>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPackage {
    name: Spanned<String>,
    version: Spanned<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    authors: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWorkspace {
    #[serde(default)]
    members: Vec<Spanned<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawDep {
    Version(String),
    Detailed(RawDetailedDep),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDetailedDep {
    version: Option<String>,
    path: Option<String>,
    git: Option<String>,
    tag: Option<String>,
    branch: Option<String>,
    rev: Option<String>,
    package: Option<String>,
}

/// A parsed `lunar.toml`: a package, a workspace, or both.
#[derive(Clone, Debug)]
pub struct ManifestFile {
    pub path: PathBuf,
    pub package: Option<Manifest>,
    /// Workspace member patterns, with their positions.
    pub workspace: Option<Vec<(String, Range<usize>)>>,
}

/// A package's manifest.
#[derive(Clone, Debug)]
pub struct Manifest {
    /// The directory containing the manifest.
    pub dir: PathBuf,
    /// The manifest file.
    pub path: PathBuf,
    pub name: String,
    pub version: Version,
    pub deps: Vec<Dependency>,
}

#[derive(Clone, Debug)]
pub struct Dependency {
    /// The name code uses for it (its key in `[dependencies]`).
    pub alias: String,
    /// The depended-on package's own name.
    pub package: String,
    pub req: Option<VersionReq>,
    pub source: DepSource,
    /// Where the entry is in the manifest.
    pub span: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DepSource {
    Registry,
    Path(PathBuf),
    Git { url: String, reference: GitRef },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GitRef {
    Tag(String),
    Branch(String),
    Rev(String),
    DefaultBranch,
}

impl GitRef {
    fn query(&self) -> String {
        match self {
            GitRef::Tag(t) => format!("?tag={t}"),
            GitRef::Branch(b) => format!("?branch={b}"),
            GitRef::Rev(r) => format!("?rev={r}"),
            GitRef::DefaultBranch => String::new(),
        }
    }
}

/// Package names: lowercase letters, digits, `-` and `_`, starting with a
/// letter. In code, `-` becomes `_`.
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase()) && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// The name a package's crater has in code.
pub fn crater_name(package: &str) -> String {
    package.replace('-', "_")
}

/// Names a dependency cannot take: they already mean something in paths.
const RESERVED: &[&str] = &["core", "std", "crater", "self", "super", "Self"];

const NAME_RULE: &str = "names use lowercase letters, digits, `-` and `_`, and start with a letter";

impl ManifestFile {
    pub fn load(dir: &Path) -> Result<ManifestFile> {
        let path = dir.join(MANIFEST);
        let text = std::fs::read_to_string(&path).map_err(|e| io_error(&path, e))?;
        Self::parse(dir, &text)
    }

    pub fn parse(dir: &Path, text: &str) -> Result<ManifestFile> {
        let path = dir.join(MANIFEST);
        let raw: RawManifest = toml::from_str(text).map_err(|e| {
            let err = ProjectError::new(code::PARSE, format!("invalid `{MANIFEST}`: {}", e.message()));
            match e.span() {
                Some(s) => err.at(&path, s),
                None => err,
            }
        })?;
        if raw.package.is_none() && raw.workspace.is_none() {
            return Err(ProjectError::new(code::PARSE, format!("`{}` has neither a `[package]` nor a `[workspace]` section", path.display()))
                .help("start it with `[package]`, `name = \"...\"` and `version = \"0.1.0\"`"));
        }
        let workspace = raw.workspace.map(|w| w.members.into_iter().map(|m| (m.get_ref().clone(), m.span())).collect());
        let package = match raw.package {
            Some(p) => Some(Self::package(dir, &path, p, raw.dependencies)?),
            None => {
                if let Some((k, _)) = raw.dependencies.iter().next() {
                    return Err(ProjectError::new(code::PARSE, "a workspace without a `[package]` cannot have `[dependencies]`")
                        .at(&path, k.span())
                        .help("declare dependencies in the member packages"));
                }
                None
            }
        };
        Ok(ManifestFile { path, package, workspace })
    }

    fn package(dir: &Path, path: &Path, p: RawPackage, deps: BTreeMap<Spanned<String>, Spanned<RawDep>>) -> Result<Manifest> {
        let name = p.name.get_ref().clone();
        if !valid_name(&name) {
            return Err(ProjectError::new(code::BAD_NAME, format!("package name `{name}` is not valid")).at(path, p.name.span()).note(NAME_RULE));
        }
        if matches!(crater_name(&name).as_str(), "core" | "std" | "crater") {
            return Err(ProjectError::new(code::BAD_NAME, format!("`{name}` cannot be a package name: it is the name of a built-in crater"))
                .at(path, p.name.span())
                .help("choose another name"));
        }
        let version = Version::parse(p.version.get_ref()).map_err(|e| {
            ProjectError::new(code::BAD_VERSION, format!("package version `{}` is not a valid version: {e}", p.version.get_ref()))
                .at(path, p.version.span())
                .help("versions look like `1.2.3`")
        })?;
        let _ = (&p.description, &p.license, &p.authors);
        let mut out = Vec::new();
        for (key, value) in deps {
            let alias = key.get_ref().clone();
            let key_span = key.span();
            let span = key_span.start..value.span().end;
            let bad = |msg: String| ProjectError::new(code::BAD_DEPENDENCY, msg).at(path, span.clone());
            if !valid_name(&alias) {
                return Err(ProjectError::new(code::BAD_NAME, format!("dependency name `{alias}` is not valid")).at(path, key_span).note(NAME_RULE));
            }
            if RESERVED.contains(&crater_name(&alias).as_str()) {
                return Err(ProjectError::new(code::BAD_NAME, format!("`{alias}` cannot be a dependency name: it already means something in paths"))
                    .at(path, key_span)
                    .help(format!("rename it, keeping the package: `my-{alias} = {{ ..., package = \"{alias}\" }}`")));
            }
            let parse_req = |s: &str| {
                VersionReq::parse(s).map_err(|e| {
                    ProjectError::new(code::BAD_VERSION, format!("dependency `{alias}`: `{s}` is not a valid version requirement: {e}"))
                        .at(path, span.clone())
                        .help("requirements look like `1.2`, `^1.2.3`, `=1.2.4` or `>=1, <3`")
                })
            };
            let dep = match value.into_inner() {
                RawDep::Version(req) => {
                    Dependency { alias: alias.clone(), package: alias.clone(), req: Some(parse_req(&req)?), source: DepSource::Registry, span }
                }
                RawDep::Detailed(t) => {
                    let req = t.version.as_deref().map(parse_req).transpose()?;
                    let package = t.package.clone().unwrap_or_else(|| alias.clone());
                    let refs = [t.tag.is_some(), t.branch.is_some(), t.rev.is_some()].iter().filter(|x| **x).count();
                    if refs > 1 {
                        return Err(bad(format!("dependency `{alias}` gives more than one of `tag`, `branch` and `rev`")).help("keep one of them"));
                    }
                    let source = match (t.path, t.git) {
                        (Some(_), Some(_)) => return Err(bad(format!("dependency `{alias}` has both `path` and `git`")).help("keep one of them")),
                        (Some(p), None) => {
                            if refs > 0 {
                                return Err(bad(format!("dependency `{alias}`: `tag`, `branch` and `rev` only apply to `git` dependencies")));
                            }
                            DepSource::Path(PathBuf::from(p))
                        }
                        (None, Some(url)) => {
                            let reference = match (t.tag, t.branch, t.rev) {
                                (Some(x), _, _) => GitRef::Tag(x),
                                (_, Some(x), _) => GitRef::Branch(x),
                                (_, _, Some(x)) => GitRef::Rev(x),
                                _ => GitRef::DefaultBranch,
                            };
                            DepSource::Git { url, reference }
                        }
                        (None, None) => {
                            if req.is_none() {
                                return Err(bad(format!("dependency `{alias}` does not say where it comes from"))
                                    .help("give it a `path`, a `git` URL, or a registry `version`"));
                            }
                            DepSource::Registry
                        }
                    };
                    Dependency { alias: alias.clone(), package, req, source, span }
                }
            };
            out.push(dep);
        }
        Ok(Manifest { dir: dir.to_path_buf(), path: path.to_path_buf(), name, version, deps: out })
    }
}

impl Manifest {
    /// The package manifest in `dir`.
    pub fn load(dir: &Path) -> Result<Manifest> {
        let file = ManifestFile::load(dir)?;
        file.package.ok_or_else(|| {
            ProjectError::new(code::PARSE, format!("`{}` describes a workspace, not a package", file.path.display()))
                .help("run the command inside a member package, or pick one with `-p <name>`")
        })
    }

    pub fn lib_root(&self) -> Option<PathBuf> {
        let p = self.dir.join("src").join("lib.ln");
        p.is_file().then_some(p)
    }

    pub fn main_root(&self) -> Option<PathBuf> {
        let p = self.dir.join("src").join("main.ln");
        p.is_file().then_some(p)
    }

    /// Integration tests: `tests/*.ln`.
    pub fn test_files(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = std::fs::read_dir(self.dir.join("tests"))
            .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "ln")).collect())
            .unwrap_or_default();
        out.sort();
        out
    }
}

/// The nearest directory at or above `start` that has a `lunar.toml`.
pub fn find_project(start: &Path) -> Option<PathBuf> {
    let start = std::path::absolute(start).ok()?;
    start.ancestors().find(|d| d.join(MANIFEST).is_file()).map(Path::to_path_buf)
}

// ===================================================================== workspaces

/// A set of packages built together, sharing `lunar.lock` and `target/`.
#[derive(Clone, Debug)]
pub struct Workspace {
    /// The directory with the shared lockfile and `target/`.
    pub root: PathBuf,
    /// Member package directories (the root's own package first, if any).
    pub members: Vec<PathBuf>,
    /// The member the command started in, if any.
    pub current: Option<PathBuf>,
}

/// The workspace around `start`: the nearest manifest, and the workspace
/// that lists it as a member (or itself).
pub fn find_workspace(start: &Path) -> Result<Workspace> {
    let Some(dir) = find_project(start) else {
        let shown = std::path::absolute(start).unwrap_or_else(|_| start.to_path_buf());
        return Err(ProjectError::new(code::NOT_FOUND, format!("no `{MANIFEST}` in `{}` or any parent directory", shown.display()))
            .help("create a package with `lunar new <name>` or `lunar init`, or pass a .ln file"));
    };
    let dir = canonical(&dir)?;
    let file = ManifestFile::load(&dir)?;
    if let Some(members) = &file.workspace {
        let current = file.package.is_some().then(|| dir.clone());
        return Ok(Workspace { members: expand_members(&dir, file.package.is_some(), members)?, root: dir, current });
    }
    // A package: look for a workspace above that lists it.
    for up in dir.ancestors().skip(1) {
        if !up.join(MANIFEST).is_file() {
            continue;
        }
        let Ok(f) = ManifestFile::load(up) else { continue };
        let Some(members) = &f.workspace else { continue };
        let members = expand_members(up, f.package.is_some(), members)?;
        if members.iter().any(|m| m == &dir) {
            return Ok(Workspace { root: up.to_path_buf(), members, current: Some(dir) });
        }
    }
    Ok(Workspace { root: dir.clone(), members: vec![dir.clone()], current: Some(dir) })
}

/// Member directories from `members` patterns (`app`, `crates/*`).
fn expand_members(root: &Path, root_is_package: bool, patterns: &[(String, Range<usize>)]) -> Result<Vec<PathBuf>> {
    let manifest = root.join(MANIFEST);
    let mut out = Vec::new();
    if root_is_package {
        out.push(root.to_path_buf());
    }
    for (pat, span) in patterns {
        if let Some(parent) = pat.strip_suffix("/*") {
            let dir = root.join(parent);
            let mut found: Vec<PathBuf> = std::fs::read_dir(&dir)
                .map_err(|_| {
                    ProjectError::new(code::NOT_FOUND, format!("workspace member pattern `{pat}`: `{}` is not a directory", dir.display()))
                        .at(&manifest, span.clone())
                })?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.join(MANIFEST).is_file())
                .collect();
            found.sort();
            for p in found {
                out.push(canonical(&p)?);
            }
        } else {
            let dir = root.join(pat);
            if !dir.join(MANIFEST).is_file() {
                return Err(ProjectError::new(code::NOT_FOUND, format!("workspace member `{pat}` has no `{MANIFEST}`"))
                    .at(&manifest, span.clone())
                    .help(format!("create it with `lunar new {pat}`, or remove it from `members`")));
            }
            out.push(canonical(&dir)?);
        }
    }
    out.dedup();
    Ok(out)
}

// ===================================================================== scaffolding

/// Create a package in `dir` (which must not contain a manifest yet).
pub fn init(dir: &Path, lib: bool) -> Result<String> {
    let manifest = dir.join(MANIFEST);
    if manifest.exists() {
        return Err(ProjectError::new(code::PARSE, format!("`{}` already exists", manifest.display())));
    }
    let name = dir.file_name().map(|n| n.to_string_lossy().to_lowercase().replace(' ', "-")).unwrap_or_default();
    if !valid_name(&name) {
        return Err(ProjectError::new(code::BAD_NAME, format!("`{name}` is not a valid package name")).note(NAME_RULE));
    }
    let src = dir.join("src");
    std::fs::create_dir_all(&src).map_err(|e| io_error(&src, e))?;
    write_new(&manifest, &format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n\n[dependencies]\n"))?;
    if lib {
        let code = "/// Add two numbers.\npub fn add(a: i32, b: i32): i32 {\n    return a + b;\n}\n\n#[test]\nfn adds() {\n    assert_eq(&add(2, 2), &4, \"2 + 2\");\n}\n";
        write_new(&src.join("lib.ln"), code)?;
    } else {
        write_new(&src.join("main.ln"), "fn main() {\n    println(\"Hello, world!\");\n}\n")?;
    }
    if !dir.join(".gitignore").exists() {
        write_new(&dir.join(".gitignore"), "/target\n")?;
    }
    // Like Cargo: start a repository unless already inside one.
    let in_repo = Command::new("git").args(["rev-parse", "--is-inside-work-tree"]).current_dir(dir).output().is_ok_and(|o| o.status.success());
    if !in_repo {
        let _ = Command::new("git").args(["init", "--quiet"]).current_dir(dir).status();
    }
    Ok(name)
}

fn write_new(path: &Path, text: &str) -> Result<()> {
    std::fs::write(path, text).map_err(|e| io_error(path, e))
}

// ===================================================================== lockfile

#[derive(Debug, Default, Deserialize)]
struct RawLock {
    #[serde(default)]
    #[allow(dead_code)]
    version: Option<u32>,
    #[serde(default)]
    package: Vec<LockedPackage>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct LockedPackage {
    pub name: String,
    pub version: String,
    pub source: String,
}

/// `lunar.lock`: the exact versions and commits of every dependency that
/// isn't a workspace member.
#[derive(Debug, Default)]
pub struct Lock {
    pub packages: Vec<LockedPackage>,
}

impl Lock {
    pub fn load(root: &Path) -> Result<Lock> {
        let path = root.join(LOCKFILE);
        let Ok(text) = std::fs::read_to_string(&path) else { return Ok(Lock::default()) };
        let raw: RawLock = toml::from_str(&text).map_err(|e| {
            let err = ProjectError::new(code::LOCK, format!("`{LOCKFILE}` is invalid: {}", e.message()))
                .help(format!("delete `{LOCKFILE}` and run `lunar update` to regenerate it"));
            match e.span() {
                Some(s) => err.at(&path, s),
                None => err,
            }
        })?;
        Ok(Lock { packages: raw.package })
    }

    /// The locked commit of a git dependency.
    fn git_commit(&self, url: &str, reference: &GitRef) -> Option<String> {
        let prefix = format!("git+{url}{}#", reference.query());
        self.packages.iter().find_map(|p| p.source.strip_prefix(&prefix).map(str::to_string))
    }

    fn render(&self) -> String {
        let mut out = String::from("# Generated by lunar. Do not edit by hand.\nversion = 1\n");
        let mut pkgs = self.packages.clone();
        pkgs.sort();
        pkgs.dedup();
        for p in pkgs {
            out.push_str(&format!("\n[[package]]\nname = \"{}\"\nversion = \"{}\"\nsource = \"{}\"\n", p.name, p.version, p.source));
        }
        out
    }

    /// Write the lockfile if its contents changed.
    pub fn save(&self, root: &Path) -> Result<()> {
        let path = root.join(LOCKFILE);
        let text = self.render();
        if std::fs::read_to_string(&path).ok().as_deref() == Some(text.as_str()) {
            return Ok(());
        }
        // A workspace without dependencies needs no lockfile.
        if self.packages.is_empty() && !path.exists() {
            return Ok(());
        }
        std::fs::write(&path, text).map_err(|e| io_error(&path, e))
    }
}

// ===================================================================== resolution

/// Where a package in the graph comes from.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SourceId {
    Path(PathBuf),
    Git { url: String, reference: GitRef, commit: String },
}

#[derive(Clone, Debug)]
pub struct Package {
    pub manifest: Manifest,
    pub source: SourceId,
    /// Its dependencies: (name in code, package index).
    pub deps: Vec<(String, usize)>,
}

/// The resolved package graph. The workspace members come first, in
/// workspace order.
#[derive(Clone, Debug)]
pub struct Graph {
    pub workspace: Workspace,
    pub packages: Vec<Package>,
    pub members: usize,
}

impl Graph {
    /// The member package named `name`.
    pub fn member(&self, name: &str) -> Option<usize> {
        self.packages[..self.members].iter().position(|p| p.manifest.name == name)
    }

    /// The member the command started in.
    pub fn current(&self) -> Option<usize> {
        let cur = self.workspace.current.as_ref()?;
        self.packages[..self.members].iter().position(|p| &p.manifest.dir == cur)
    }
}

/// Resolve the dependency graph of the workspace. Locked git commits are
/// reused unless `update` is set; the lockfile is rewritten to match.
pub fn resolve(ws: Workspace, update: bool, log: &mut dyn FnMut(&str)) -> Result<Graph> {
    let lock = if update { Lock::default() } else { Lock::load(&ws.root)? };
    let mut packages: Vec<Package> = Vec::new();
    let mut index: HashMap<SourceId, usize> = HashMap::new();
    let mut names: HashMap<String, PathBuf> = HashMap::new();
    for dir in &ws.members {
        let m = Manifest::load(dir)?;
        if let Some(other) = names.insert(m.name.clone(), dir.clone()) {
            return Err(ProjectError::new(code::WORKSPACE, format!("two workspace members are named `{}`", m.name))
                .note(format!("`{}` and `{}`", other.display(), dir.display()))
                .help("member package names must be unique"));
        }
        let id = SourceId::Path(dir.clone());
        index.insert(id.clone(), packages.len());
        packages.push(Package { manifest: m, source: id, deps: Vec::new() });
    }
    let members = packages.len();
    let mut queue: Vec<usize> = (0..members).collect();
    while let Some(i) = queue.pop() {
        let manifest = packages[i].manifest.clone();
        for dep in &manifest.deps {
            let at = |e: ProjectError| e.at(&manifest.path, dep.span.clone());
            let (id, dir) = match &dep.source {
                DepSource::Registry => {
                    return Err(at(ProjectError::new(
                        code::REGISTRY_UNAVAILABLE,
                        format!("`{}` comes from the package registry, which is not available yet", dep.alias),
                    ))
                    .help("use a `path` or `git` dependency for now"));
                }
                DepSource::Path(p) => {
                    let dir = manifest.dir.join(p);
                    if !dir.join(MANIFEST).is_file() {
                        return Err(at(ProjectError::new(code::NOT_FOUND, format!("dependency `{}`: no `{MANIFEST}` in `{}`", dep.alias, dir.display()))));
                    }
                    let dir = canonical(&dir)?;
                    (SourceId::Path(dir.clone()), dir)
                }
                DepSource::Git { url, reference } => {
                    let commit = match lock.git_commit(url, reference) {
                        Some(c) => c,
                        None => {
                            log(&format!("Fetching {url}"));
                            git::resolve(url, reference).map_err(|e| at(ProjectError::new(code::GIT, format!("dependency `{}`: {e}", dep.alias))))?
                        }
                    };
                    let dir = git::checkout(url, &commit).map_err(|e| at(ProjectError::new(code::GIT, format!("dependency `{}`: {e}", dep.alias))))?;
                    (SourceId::Git { url: url.clone(), reference: reference.clone(), commit }, dir)
                }
            };
            let j = match index.get(&id) {
                Some(&j) => j,
                None => {
                    let m = Manifest::load(&dir)?;
                    let j = packages.len();
                    index.insert(id.clone(), j);
                    packages.push(Package { manifest: m, source: id, deps: Vec::new() });
                    queue.push(j);
                    j
                }
            };
            let found = &packages[j].manifest;
            if found.name != dep.package {
                return Err(at(ProjectError::new(
                    code::WRONG_PACKAGE,
                    format!("dependency `{}` points at package `{}`, expected `{}`", dep.alias, found.name, dep.package),
                ))
                .help(format!("if this is intended, keep the name `{}` in code with `package = \"{}\"`", dep.alias, found.name)));
            }
            if let Some(req) = &dep.req
                && !req.matches(&found.version)
            {
                return Err(at(ProjectError::new(
                    code::VERSION_MISMATCH,
                    format!(
                        "`{}` requires `{}` {req}, but {} has version {}",
                        manifest.name,
                        dep.package,
                        describe_source(&packages[j].source, &ws.root),
                        found.version
                    ),
                )));
            }
            packages[i].deps.push((crater_name(&dep.alias), j));
        }
    }
    let new_lock = Lock {
        packages: packages[members..]
            .iter()
            .map(|p| LockedPackage { name: p.manifest.name.clone(), version: p.manifest.version.to_string(), source: lock_source(&p.source, &ws.root) })
            .collect(),
    };
    new_lock.save(&ws.root)?;
    Ok(Graph { workspace: ws, packages, members })
}

fn canonical(p: &Path) -> Result<PathBuf> {
    let c = std::fs::canonicalize(p).map_err(|e| io_error(p, e))?;
    // Avoid Windows' `\\?\` prefix in paths shown to people.
    let s = c.to_string_lossy();
    Ok(PathBuf::from(s.strip_prefix(r"\\?\").unwrap_or(&s)))
}

fn lock_source(id: &SourceId, root: &Path) -> String {
    match id {
        SourceId::Path(p) => {
            let rel = relative(p, root).unwrap_or_else(|| p.clone());
            format!("path+{}", rel.to_string_lossy().replace('\\', "/"))
        }
        SourceId::Git { url, reference, commit } => format!("git+{url}{}#{commit}", reference.query()),
    }
}

fn describe_source(id: &SourceId, root: &Path) -> String {
    match id {
        SourceId::Path(_) => format!("the package at `{}`", lock_source(id, root).trim_start_matches("path+")),
        SourceId::Git { url, commit, .. } => format!("{url} at {}", &commit[..commit.len().min(12)]),
    }
}

/// `path` relative to `base`, if `path` is reachable from it with `..`.
fn relative(path: &Path, base: &Path) -> Option<PathBuf> {
    let p: Vec<_> = path.components().collect();
    let b: Vec<_> = base.components().collect();
    let common = p.iter().zip(&b).take_while(|(x, y)| x == y).count();
    if common == 0 {
        return None;
    }
    let mut out = PathBuf::new();
    for _ in common..b.len() {
        out.push("..");
    }
    for c in &p[common..] {
        out.push(c);
    }
    Some(if out.as_os_str().is_empty() { PathBuf::from(".") } else { out })
}

// ===================================================================== craters

/// What to build from a package.
#[derive(Clone, Debug)]
pub enum Target {
    /// `src/main.ln`.
    Main,
    /// `src/lib.ln` (for checking and its unit tests).
    Lib,
    /// An integration test file `tests/*.ln`.
    TestFile(PathBuf),
}

/// The craters to analyze for `target` of package `pkg`: the target crater
/// first, then the libraries of every package it depends on, directly or not.
pub fn craters(graph: &Graph, pkg: usize, target: &Target) -> Result<Vec<CraterSpec>> {
    let p = &graph.packages[pkg];
    let own = crater_name(&p.manifest.name);
    let missing = |what: &str, file: &str| ProjectError::new(code::NO_TARGET, format!("package `{}` has no {what} (`src/{file}`)", p.manifest.name));
    let (path, uses_own_lib) = match target {
        Target::Main => (p.manifest.main_root().ok_or_else(|| missing("program", "main.ln"))?, true),
        Target::Lib => (p.manifest.lib_root().ok_or_else(|| missing("library", "lib.ln"))?, false),
        Target::TestFile(f) => (f.clone(), true),
    };
    let mut specs = vec![CraterSpec { name: own.clone(), root: path, deps: Vec::new() }];
    // Package index -> index of its library crater in `specs`.
    let mut lib_of: HashMap<usize, usize> = HashMap::new();
    let mut deps = Vec::new();
    if uses_own_lib && p.manifest.lib_root().is_some() {
        deps.push((own, lib_crater(graph, pkg, &mut specs, &mut lib_of)?));
    }
    for (alias, j) in &p.deps {
        deps.push((alias.clone(), lib_crater(graph, *j, &mut specs, &mut lib_of)?));
    }
    specs[0].deps = deps;
    Ok(specs)
}

fn lib_crater(graph: &Graph, pkg: usize, specs: &mut Vec<CraterSpec>, lib_of: &mut HashMap<usize, usize>) -> Result<usize> {
    if let Some(&i) = lib_of.get(&pkg) {
        return Ok(i);
    }
    let p = &graph.packages[pkg];
    let root = p
        .manifest
        .lib_root()
        .ok_or_else(|| ProjectError::new(code::NO_TARGET, format!("package `{}` has no library (`src/lib.ln`) to depend on", p.manifest.name)))?;
    let i = specs.len();
    specs.push(CraterSpec { name: crater_name(&p.manifest.name), root, deps: Vec::new() });
    lib_of.insert(pkg, i);
    let mut deps = Vec::new();
    for (alias, j) in &p.deps {
        deps.push((alias.clone(), lib_crater(graph, *j, specs, lib_of)?));
    }
    specs[i].deps = deps;
    Ok(i)
}

// ===================================================================== editing

/// Add or replace a dependency in the package manifest in `dir`, keeping its
/// formatting and comments.
pub fn add_dependency(dir: &Path, alias: &str, spec: &DepSpecArgs) -> Result<()> {
    let path = dir.join(MANIFEST);
    if !valid_name(alias) {
        return Err(ProjectError::new(code::BAD_NAME, format!("`{alias}` is not a valid dependency name")).note(NAME_RULE));
    }
    let text = std::fs::read_to_string(&path).map_err(|e| io_error(&path, e))?;
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| {
        let err = ProjectError::new(code::PARSE, format!("invalid `{MANIFEST}`: {}", e.message()));
        match e.span() {
            Some(s) => err.at(&path, s),
            None => err,
        }
    })?;
    let deps = doc.entry("dependencies").or_insert(toml_edit::table());
    let Some(table) = deps.as_table_like_mut() else {
        return Err(ProjectError::new(code::PARSE, format!("`dependencies` in `{}` is not a table", path.display())));
    };
    table.insert(alias, spec.to_value());
    let new = doc.to_string();
    // Validate before writing.
    let file = ManifestFile::parse(dir, &new)?;
    if file.package.is_none() {
        return Err(
            ProjectError::new(code::PARSE, format!("`{}` describes a workspace, not a package", path.display())).help("add dependencies to a member package")
        );
    }
    std::fs::write(&path, new).map_err(|e| io_error(&path, e))
}

/// Remove a dependency from the package manifest in `dir`.
pub fn remove_dependency(dir: &Path, alias: &str) -> Result<()> {
    let path = dir.join(MANIFEST);
    let text = std::fs::read_to_string(&path).map_err(|e| io_error(&path, e))?;
    let mut doc: toml_edit::DocumentMut =
        text.parse().map_err(|e: toml_edit::TomlError| ProjectError::new(code::PARSE, format!("invalid `{MANIFEST}`: {}", e.message())))?;
    let removed = doc.get_mut("dependencies").and_then(|d| d.as_table_like_mut()).and_then(|t| t.remove(alias));
    if removed.is_none() {
        return Err(ProjectError::new(code::NOT_FOUND, format!("`{alias}` is not a dependency of this package")));
    }
    std::fs::write(&path, doc.to_string()).map_err(|e| io_error(&path, e))
}

/// A dependency as given to `lunar add`.
#[derive(Clone, Debug, Default)]
pub struct DepSpecArgs {
    pub version: Option<String>,
    pub path: Option<String>,
    pub git: Option<String>,
    pub tag: Option<String>,
    pub branch: Option<String>,
    pub rev: Option<String>,
    pub package: Option<String>,
}

impl DepSpecArgs {
    fn to_value(&self) -> toml_edit::Item {
        let simple = self.path.is_none() && self.git.is_none() && self.package.is_none();
        if simple && let Some(v) = &self.version {
            return toml_edit::value(v.as_str());
        }
        let mut t = toml_edit::InlineTable::new();
        let fields = [
            ("version", &self.version),
            ("path", &self.path),
            ("git", &self.git),
            ("tag", &self.tag),
            ("branch", &self.branch),
            ("rev", &self.rev),
            ("package", &self.package),
        ];
        for (k, v) in fields {
            if let Some(v) = v {
                t.insert(k, v.as_str().into());
            }
        }
        toml_edit::Item::Value(toml_edit::Value::InlineTable(t))
    }
}

// ===================================================================== git

mod git {
    use super::GitRef;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    type Result<T> = std::result::Result<T, String>;

    /// `$LUNAR_HOME`, or `~/.lunar`.
    pub fn home() -> PathBuf {
        if let Ok(h) = std::env::var("LUNAR_HOME") {
            return PathBuf::from(h);
        }
        let user = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        user.join(".lunar")
    }

    /// A stable short name for a repository URL.
    fn slug(url: &str) -> String {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in url.bytes() {
            h = (h ^ b as u64).wrapping_mul(0x100000001b3);
        }
        let tail: String = url
            .trim_end_matches('/')
            .trim_end_matches(".git")
            .rsplit('/')
            .next()
            .unwrap_or("repo")
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        format!("{tail}-{h:016x}")
    }

    fn run(args: &[&str], dir: Option<&Path>) -> Result<String> {
        let mut cmd = Command::new("git");
        cmd.args(args);
        if let Some(d) = dir {
            cmd.current_dir(d);
        }
        let out = cmd.output().map_err(|e| format!("could not run git: {e} (is git installed?)"))?;
        if !out.status.success() {
            return Err(format!("`git {}` failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// The local bare mirror of `url`, fetched up to date.
    fn mirror(url: &str, fetch: bool) -> Result<PathBuf> {
        let db = home().join("git").join("db").join(slug(url));
        if !db.join("HEAD").exists() {
            std::fs::create_dir_all(db.parent().unwrap()).map_err(|e| e.to_string())?;
            run(&["clone", "--bare", "--quiet", url, &db.to_string_lossy()], None)?;
        } else if fetch {
            run(&["fetch", "--quiet", "--force", "--tags", "origin", "+refs/heads/*:refs/heads/*"], Some(&db))?;
        }
        Ok(db)
    }

    /// The commit a reference names.
    pub fn resolve(url: &str, reference: &GitRef) -> Result<String> {
        let db = mirror(url, true)?;
        let spec = match reference {
            GitRef::Tag(t) => format!("refs/tags/{t}^{{commit}}"),
            GitRef::Branch(b) => format!("refs/heads/{b}^{{commit}}"),
            GitRef::Rev(r) => format!("{r}^{{commit}}"),
            GitRef::DefaultBranch => "HEAD^{commit}".to_string(),
        };
        run(&["rev-parse", "--verify", "--quiet", &spec], Some(&db)).map_err(|_| format!("{url} has no {}", describe(reference)))
    }

    fn describe(r: &GitRef) -> String {
        match r {
            GitRef::Tag(t) => format!("tag `{t}`"),
            GitRef::Branch(b) => format!("branch `{b}`"),
            GitRef::Rev(r) => format!("commit `{r}`"),
            GitRef::DefaultBranch => "default branch".into(),
        }
    }

    /// A checkout of `commit`, made once and then reused.
    pub fn checkout(url: &str, commit: &str) -> Result<PathBuf> {
        let dir = home().join("git").join("checkouts").join(slug(url)).join(&commit[..commit.len().min(12)]);
        if dir.join(".lunar-ok").exists() {
            return Ok(dir);
        }
        let db = mirror(url, false)?;
        if run(&["cat-file", "-e", &format!("{commit}^{{commit}}")], Some(&db)).is_err() {
            mirror(url, true)?;
        }
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.parent().unwrap()).map_err(|e| e.to_string())?;
        run(&["clone", "--quiet", "--no-checkout", &db.to_string_lossy(), &dir.to_string_lossy()], None)?;
        run(&["checkout", "--quiet", commit], Some(&dir))?;
        std::fs::write(dir.join(".lunar-ok"), "").map_err(|e| e.to_string())?;
        Ok(dir)
    }
}

pub use git::home as lunar_home;
