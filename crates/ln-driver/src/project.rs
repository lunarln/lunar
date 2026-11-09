//! Projects: `lunar.toml` manifests, the package graph, `lunar.lock`, and
//! fetching git dependencies.
//!
//! A *package* is a directory with a `lunar.toml`. It contains up to two
//! craters: a library (`src/lib.ln`) that other packages can depend on, and
//! a program (`src/main.ln`). Tests live in those craters (`#[test]`) and in
//! `tests/*.ln`, each of which is a crater that can use the library.
//!
//! Dependencies come from a path or a git repository. Registry dependencies
//! (`json = "1.2"`) are parsed, but the registry does not exist yet.

use crate::CraterSpec;
use semver::{Version, VersionReq};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const MANIFEST: &str = "lunar.toml";
pub const LOCKFILE: &str = "lunar.lock";

pub type Result<T> = std::result::Result<T, String>;

// ===================================================================== manifest

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    package: RawPackage,
    #[serde(default)]
    dependencies: BTreeMap<String, RawDep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPackage {
    name: String,
    version: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    authors: Vec<String>,
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

/// A parsed `lunar.toml`.
#[derive(Clone, Debug)]
pub struct Manifest {
    /// The directory containing the manifest.
    pub dir: PathBuf,
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

impl Manifest {
    pub fn load(dir: &Path) -> Result<Manifest> {
        let path = dir.join(MANIFEST);
        let text = std::fs::read_to_string(&path).map_err(|e| format!("could not read `{}`: {e}", path.display()))?;
        Self::parse(dir, &text).map_err(|e| format!("invalid `{}`: {e}", path.display()))
    }

    pub fn parse(dir: &Path, text: &str) -> Result<Manifest> {
        let raw: RawManifest = toml::from_str(text).map_err(|e| e.to_string())?;
        if !valid_name(&raw.package.name) {
            return Err(format!("package name `{}` is not valid: use lowercase letters, digits, `-` and `_`, starting with a letter", raw.package.name));
        }
        let version = Version::parse(&raw.package.version).map_err(|e| format!("package version `{}`: {e}", raw.package.version))?;
        let _ = (&raw.package.description, &raw.package.license, &raw.package.authors);
        let mut deps = Vec::new();
        for (alias, d) in raw.dependencies {
            let code_name = crater_name(&alias);
            if RESERVED.contains(&code_name.as_str()) {
                return Err(format!("`{alias}` cannot be used as a dependency name: it already means something in paths"));
            }
            if !valid_name(&alias) {
                return Err(format!("dependency name `{alias}` is not valid: use lowercase letters, digits, `-` and `_`"));
            }
            let parse_req = |s: &str| VersionReq::parse(s).map_err(|e| format!("dependency `{alias}`: version requirement `{s}`: {e}"));
            let dep = match d {
                RawDep::Version(req) => Dependency { alias: alias.clone(), package: alias.clone(), req: Some(parse_req(&req)?), source: DepSource::Registry },
                RawDep::Detailed(t) => {
                    let req = t.version.as_deref().map(parse_req).transpose()?;
                    let package = t.package.clone().unwrap_or_else(|| alias.clone());
                    let refs = [t.tag.is_some(), t.branch.is_some(), t.rev.is_some()].iter().filter(|x| **x).count();
                    if refs > 1 {
                        return Err(format!("dependency `{alias}`: use only one of `tag`, `branch` and `rev`"));
                    }
                    let source = match (t.path, t.git) {
                        (Some(_), Some(_)) => return Err(format!("dependency `{alias}`: `path` and `git` cannot both be given")),
                        (Some(p), None) => {
                            if refs > 0 {
                                return Err(format!("dependency `{alias}`: `tag`, `branch` and `rev` only apply to `git` dependencies"));
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
                                return Err(format!("dependency `{alias}` needs a `version`, `path` or `git`"));
                            }
                            DepSource::Registry
                        }
                    };
                    Dependency { alias: alias.clone(), package, req, source }
                }
            };
            deps.push(dep);
        }
        Ok(Manifest { dir: dir.to_path_buf(), name: raw.package.name, version, deps })
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

/// The directory of the project containing `start` (searching upwards).
pub fn find_project(start: &Path) -> Option<PathBuf> {
    let start = std::path::absolute(start).ok()?;
    start.ancestors().find(|d| d.join(MANIFEST).is_file()).map(Path::to_path_buf)
}

// ===================================================================== scaffolding

/// Create a package in `dir` (which must not contain a manifest yet).
pub fn init(dir: &Path, lib: bool) -> Result<String> {
    if dir.join(MANIFEST).exists() {
        return Err(format!("`{}` already exists", dir.join(MANIFEST).display()));
    }
    let name = dir.file_name().map(|n| n.to_string_lossy().to_lowercase().replace(' ', "-")).unwrap_or_default();
    if !valid_name(&name) {
        return Err(format!("`{name}` is not a valid package name: use lowercase letters, digits, `-` and `_`, starting with a letter"));
    }
    std::fs::create_dir_all(dir.join("src")).map_err(|e| e.to_string())?;
    let manifest = format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n\n[dependencies]\n");
    write_new(&dir.join(MANIFEST), &manifest)?;
    if lib {
        let code = "/// Add two numbers.\npub fn add(a: i32, b: i32): i32 {\n    return a + b;\n}\n\n#[test]\nfn adds() {\n    std::test::assert_eq(add(2, 2), 4, \"2 + 2\");\n}\n";
        write_new(&dir.join("src").join("lib.ln"), code)?;
    } else {
        write_new(&dir.join("src").join("main.ln"), "fn main() {\n    println(\"Hello, world!\");\n}\n")?;
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
    std::fs::write(path, text).map_err(|e| format!("could not write `{}`: {e}", path.display()))
}

// ===================================================================== lockfile

#[derive(Debug, Default, Deserialize)]
struct RawLock {
    #[serde(default)]
    package: Vec<LockedPackage>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct LockedPackage {
    pub name: String,
    pub version: String,
    pub source: String,
}

/// `lunar.lock`: the exact versions and commits of every dependency.
#[derive(Debug, Default)]
pub struct Lock {
    pub packages: Vec<LockedPackage>,
}

impl Lock {
    pub fn load(dir: &Path) -> Result<Lock> {
        let path = dir.join(LOCKFILE);
        let Ok(text) = std::fs::read_to_string(&path) else { return Ok(Lock::default()) };
        let raw: RawLock = toml::from_str(&text).map_err(|e| format!("invalid `{}`: {e}", path.display()))?;
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
    pub fn save(&self, dir: &Path) -> Result<()> {
        let path = dir.join(LOCKFILE);
        let text = self.render();
        if std::fs::read_to_string(&path).ok().as_deref() == Some(text.as_str()) {
            return Ok(());
        }
        // A project without dependencies needs no lockfile.
        if self.packages.is_empty() && !path.exists() {
            return Ok(());
        }
        std::fs::write(&path, text).map_err(|e| format!("could not write `{}`: {e}", path.display()))
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

/// The resolved package graph; package 0 is the project itself.
#[derive(Clone, Debug)]
pub struct Graph {
    pub packages: Vec<Package>,
}

/// Resolve the dependency graph of the project in `root`. Locked git
/// commits are reused unless `update` is set; the lockfile is rewritten to
/// match the result.
pub fn resolve(root: &Path, update: bool, log: &mut dyn FnMut(&str)) -> Result<Graph> {
    let lock = if update { Lock::default() } else { Lock::load(root)? };
    let root_manifest = Manifest::load(root)?;
    let mut packages: Vec<Package> = Vec::new();
    let mut index: HashMap<SourceId, usize> = HashMap::new();
    let root_id = SourceId::Path(canonical(root)?);
    index.insert(root_id.clone(), 0);
    packages.push(Package { manifest: root_manifest, source: root_id, deps: Vec::new() });
    let mut queue = vec![0];
    while let Some(i) = queue.pop() {
        let manifest = packages[i].manifest.clone();
        for dep in &manifest.deps {
            let (id, dir) = match &dep.source {
                DepSource::Registry => {
                    return Err(format!(
                        "`{}` depends on `{}` from the package registry, which is not available yet; use a `path` or `git` dependency for now",
                        manifest.name, dep.alias
                    ));
                }
                DepSource::Path(p) => {
                    let dir = manifest.dir.join(p);
                    if !dir.join(MANIFEST).is_file() {
                        return Err(format!("dependency `{}` of `{}`: no `{MANIFEST}` in `{}`", dep.alias, manifest.name, dir.display()));
                    }
                    let dir = canonical(&dir)?;
                    (SourceId::Path(dir.clone()), dir)
                }
                DepSource::Git { url, reference } => {
                    let commit = match lock.git_commit(url, reference) {
                        Some(c) => c,
                        None => {
                            log(&format!("Fetching {url}"));
                            git::resolve(url, reference)?
                        }
                    };
                    let dir = git::checkout(url, &commit)?;
                    (SourceId::Git { url: url.clone(), reference: reference.clone(), commit }, dir)
                }
            };
            let j = match index.get(&id) {
                Some(&j) => j,
                None => {
                    let m = Manifest::load(&dir)?;
                    if m.name != dep.package {
                        return Err(format!("dependency `{}` of `{}` points at package `{}`, expected `{}`", dep.alias, manifest.name, m.name, dep.package));
                    }
                    let j = packages.len();
                    index.insert(id.clone(), j);
                    packages.push(Package { manifest: m, source: id, deps: Vec::new() });
                    queue.push(j);
                    j
                }
            };
            let found = &packages[j].manifest;
            if let Some(req) = &dep.req
                && !req.matches(&found.version)
            {
                return Err(format!(
                    "`{}` requires `{}` {req}, but {} has version {}",
                    manifest.name,
                    dep.package,
                    describe_source(&packages[j].source, root),
                    found.version
                ));
            }
            packages[i].deps.push((crater_name(&dep.alias), j));
        }
    }
    let new_lock = Lock {
        packages: packages
            .iter()
            .skip(1)
            .map(|p| LockedPackage { name: p.manifest.name.clone(), version: p.manifest.version.to_string(), source: lock_source(&p.source, root) })
            .collect(),
    };
    new_lock.save(root)?;
    Ok(Graph { packages })
}

fn canonical(p: &Path) -> Result<PathBuf> {
    let c = std::fs::canonicalize(p).map_err(|e| format!("`{}`: {e}", p.display()))?;
    // Avoid Windows' `\\?\` prefix in paths shown to people.
    let s = c.to_string_lossy();
    Ok(PathBuf::from(s.strip_prefix(r"\\?\").unwrap_or(&s)))
}

fn lock_source(id: &SourceId, root: &Path) -> String {
    match id {
        SourceId::Path(p) => {
            let root = canonical(root).unwrap_or_else(|_| root.to_path_buf());
            let rel = relative(p, &root).unwrap_or_else(|| p.clone());
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

/// What to build from the project's package.
#[derive(Clone, Debug)]
pub enum Target {
    /// `src/main.ln`.
    Main,
    /// `src/lib.ln` (for checking and its unit tests).
    Lib,
    /// An integration test file `tests/*.ln`.
    TestFile(PathBuf),
}

/// The craters to analyze for `target`: the target crater first, then the
/// libraries of every package it depends on, directly or not.
pub fn craters(graph: &Graph, target: &Target) -> Result<Vec<CraterSpec>> {
    let root = &graph.packages[0];
    let own = crater_name(&root.manifest.name);
    let (path, uses_own_lib) = match target {
        Target::Main => (root.manifest.main_root().ok_or_else(|| format!("package `{}` has no program (src/main.ln)", root.manifest.name))?, true),
        Target::Lib => (root.manifest.lib_root().ok_or_else(|| format!("package `{}` has no library (src/lib.ln)", root.manifest.name))?, false),
        Target::TestFile(p) => (p.clone(), true),
    };
    let mut specs = vec![CraterSpec { name: own.clone(), root: path, deps: Vec::new() }];
    // Package index -> index of its library crater in `specs`.
    let mut lib_of: HashMap<usize, usize> = HashMap::new();
    let mut deps = Vec::new();
    if uses_own_lib && root.manifest.lib_root().is_some() {
        deps.push((own, lib_crater(graph, 0, &mut specs, &mut lib_of)?));
    }
    for (alias, j) in &root.deps {
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
    let root = p.manifest.lib_root().ok_or_else(|| format!("package `{}` has no library (src/lib.ln) to depend on", p.manifest.name))?;
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

/// Add or replace a dependency in the project's manifest, keeping its
/// formatting and comments.
pub fn add_dependency(root: &Path, alias: &str, spec: &DepSpecArgs) -> Result<()> {
    if !valid_name(alias) {
        return Err(format!("`{alias}` is not a valid dependency name"));
    }
    let path = root.join(MANIFEST);
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("invalid `{}`: {e}", path.display()))?;
    let value = spec.to_value();
    let deps = doc.entry("dependencies").or_insert(toml_edit::table());
    let Some(table) = deps.as_table_like_mut() else { return Err("`dependencies` in lunar.toml is not a table".into()) };
    table.insert(alias, value);
    let new = doc.to_string();
    // Validate before writing.
    Manifest::parse(root, &new)?;
    std::fs::write(&path, new).map_err(|e| e.to_string())
}

/// Remove a dependency from the project's manifest.
pub fn remove_dependency(root: &Path, alias: &str) -> Result<()> {
    let path = root.join(MANIFEST);
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("invalid `{}`: {e}", path.display()))?;
    let removed = doc.get_mut("dependencies").and_then(|d| d.as_table_like_mut()).and_then(|t| t.remove(alias));
    if removed.is_none() {
        return Err(format!("`{alias}` is not a dependency of this package"));
    }
    std::fs::write(&path, doc.to_string()).map_err(|e| e.to_string())
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
    use super::{GitRef, Result};
    use std::path::{Path, PathBuf};
    use std::process::Command;

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
