//! `lunar lsp`: a minimal Language Server Protocol server over stdio.
//!
//! Supports diagnostics (on open/change), hover and go-to-definition. Every
//! change re-analyzes the crate the file belongs to (queued changes are
//! coalesced first). A file is analyzed as part of the crate of another open
//! file that loads it as a module (`mod foo;`), otherwise as its own root.

use crate::ide;
use crate::{Analysis, AnalyzeOptions, analyze_with, path_key};
use ln_diag::{Diagnostic, FileId, Severity, Span};
use lsp_server::{Connection, ErrorCode, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::{DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification as _, PublishDiagnostics};
use lsp_types::request::{GotoDefinition, HoverRequest, Request as _};
use lsp_types::{
    DiagnosticRelatedInformation, DiagnosticSeverity, GotoDefinitionResponse, Hover, HoverContents, HoverProviderCapability, Location, MarkupContent,
    MarkupKind, NumberOrString, OneOf, Position, PublishDiagnosticsParams, Range, ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind, Url,
};
use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};

type Error = Box<dyn std::error::Error + Send + Sync>;

/// Run the server on stdin/stdout until the client shuts it down.
pub fn run() -> Result<(), Error> {
    let (conn, io) = Connection::stdio();
    let caps = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        definition_provider: Some(OneOf::Left(true)),
        ..Default::default()
    };
    conn.initialize(serde_json::to_value(caps)?)?;
    let mut server = Server::new()?;
    server.main_loop(&conn)?;
    drop(conn);
    io.join()?;
    Ok(())
}

/// URI scheme for the embedded standard library, named by module path:
/// `lunar-std:core::string` is `core/string.ln`. Editors show these as
/// virtual, read-only, pathless documents whose text comes from the
/// `lunar/stdSource` request; nothing is written to disk.
pub const STD_SCHEME: &str = "lunar-std";

/// Custom request: `{ "uri": "lunar-std:core::string" }` -> the file's text.
pub const STD_SOURCE_METHOD: &str = "lunar/stdSource";

struct Server {
    /// Open documents by canonical path key.
    open: HashMap<PathBuf, String>,
    /// Analyses by crate root (canonical key), with the root's real path.
    crates: HashMap<PathBuf, (PathBuf, Analysis)>,
    /// Which crate root each user file was last analyzed under.
    file_root: HashMap<PathBuf, PathBuf>,
    /// URIs with published diagnostics, per crate root (to clear them later).
    published: HashMap<PathBuf, HashSet<Url>>,
}

impl Server {
    fn new() -> Result<Self, Error> {
        Ok(Server { open: HashMap::new(), crates: HashMap::new(), file_root: HashMap::new(), published: HashMap::new() })
    }

    fn main_loop(&mut self, conn: &Connection) -> Result<(), Error> {
        while let Ok(msg) = conn.receiver.recv() {
            // Coalesce everything already queued before re-analyzing.
            let mut batch = vec![msg];
            while let Ok(m) = conn.receiver.try_recv() {
                batch.push(m);
            }
            let mut dirty: Vec<PathBuf> = Vec::new();
            for msg in batch {
                match msg {
                    Message::Request(req) => {
                        if conn.handle_shutdown(&req)? {
                            return Ok(());
                        }
                        // Answer against up-to-date analyses.
                        self.reanalyze(conn, std::mem::take(&mut dirty))?;
                        let resp = self.request(req);
                        conn.sender.send(Message::Response(resp))?;
                    }
                    Message::Notification(n) => {
                        if let Some(path) = self.notification(conn, n)? {
                            if !dirty.contains(&path) {
                                dirty.push(path);
                            }
                        }
                    }
                    Message::Response(_) => {}
                }
            }
            self.reanalyze(conn, dirty)?;
        }
        Ok(())
    }

    /// Apply a document notification; returns the file that changed.
    fn notification(&mut self, conn: &Connection, n: Notification) -> Result<Option<PathBuf>, Error> {
        match n.method.as_str() {
            DidOpenTextDocument::METHOD => {
                let p: lsp_types::DidOpenTextDocumentParams = serde_json::from_value(n.params)?;
                let Some(path) = to_path(&p.text_document.uri) else { return Ok(None) };
                self.open.insert(path_key(&path), p.text_document.text);
                Ok(Some(path))
            }
            DidChangeTextDocument::METHOD => {
                let p: lsp_types::DidChangeTextDocumentParams = serde_json::from_value(n.params)?;
                let Some(path) = to_path(&p.text_document.uri) else { return Ok(None) };
                if let Some(c) = p.content_changes.into_iter().last() {
                    self.open.insert(path_key(&path), c.text);
                }
                Ok(Some(path))
            }
            DidCloseTextDocument::METHOD => {
                let p: lsp_types::DidCloseTextDocumentParams = serde_json::from_value(n.params)?;
                let Some(path) = to_path(&p.text_document.uri) else { return Ok(None) };
                let key = path_key(&path);
                self.open.remove(&key);
                // Drop crates that no longer have any open file.
                let roots: Vec<PathBuf> = self.crates.keys().cloned().collect();
                for root in roots {
                    if !self.file_root.iter().any(|(f, r)| *r == root && self.open.contains_key(f)) {
                        self.crates.remove(&root);
                        self.file_root.retain(|_, r| *r != root);
                        for uri in self.published.remove(&root).unwrap_or_default() {
                            publish(conn, uri, Vec::new())?;
                        }
                    }
                }
                // Other open files of its crate still see the on-disk text.
                Ok(self.file_root.get(&key).is_some().then_some(path))
            }
            _ => Ok(None),
        }
    }

    fn reanalyze(&mut self, conn: &Connection, changed: Vec<PathBuf>) -> Result<(), Error> {
        let mut roots: Vec<PathBuf> = Vec::new();
        for path in changed {
            let key = path_key(&path);
            let root = match self.file_root.get(&key) {
                Some(r) => self.crates.get(r).map(|(p, _)| p.clone()).unwrap_or(path),
                // Not part of any analyzed crate yet: a module file opened on
                // its own belongs to the crate of the file that declares it.
                None => crate_root_of(&path, &|p: &Path| self.open.get(&path_key(p)).cloned().or_else(|| std::fs::read_to_string(p).ok())),
            };
            if !roots.iter().any(|r| path_key(r) == path_key(&root)) {
                roots.push(root);
            }
        }
        for root in roots {
            self.analyze_root(conn, &root)?;
        }
        Ok(())
    }

    fn analyze_root(&mut self, conn: &Connection, root: &Path) -> Result<(), Error> {
        let root_key = path_key(root);
        let opts = AnalyzeOptions { keep_going: true, overlay: Some(&self.open), ..Default::default() };
        let analysis = match std::panic::catch_unwind(AssertUnwindSafe(|| analyze_with(root, &opts))) {
            Ok(a) => a,
            Err(_) => {
                eprintln!("lunar lsp: the compiler panicked while analyzing {}", root.display());
                return Ok(());
            }
        };
        // Record which user files belong to this crate. A file that was its
        // own root but is now a module of this crate stops being a root.
        let mut files: Vec<(FileId, PathBuf)> = Vec::new();
        for (id, f) in analysis.map.files() {
            if let Some(p) = &f.path
                && std_rel_path(f).is_none()
            {
                files.push((id, p.clone()));
            }
        }
        for (_, p) in &files {
            let k = path_key(p);
            if k != root_key && self.crates.contains_key(&k) {
                self.crates.remove(&k);
                for uri in self.published.remove(&k).unwrap_or_default() {
                    publish(conn, uri, Vec::new())?;
                }
            }
            self.file_root.insert(k, root_key.clone());
        }
        // Diagnostics per file.
        let mut per_file: HashMap<FileId, Vec<lsp_types::Diagnostic>> = files.iter().map(|(id, _)| (*id, Vec::new())).collect();
        let root_file = ide::file_for_path(&analysis.map, root);
        for d in &analysis.diags {
            let file = if d.span.is_dummy() { root_file } else { Some(d.span.file) };
            if let Some(file) = file
                && let Some(list) = per_file.get_mut(&file)
            {
                list.push(to_lsp_diag(&analysis, d));
            }
        }
        let mut now = HashSet::new();
        for (id, path) in &files {
            if let Ok(uri) = Url::from_file_path(path) {
                now.insert(uri.clone());
                publish(conn, uri, per_file.remove(id).unwrap_or_default())?;
            }
        }
        for stale in self.published.get(&root_key).cloned().unwrap_or_default().difference(&now) {
            publish(conn, stale.clone(), Vec::new())?;
        }
        self.published.insert(root_key.clone(), now);
        self.crates.insert(root_key, (root.to_path_buf(), analysis));
        Ok(())
    }

    /// The analysis and file id for a document, plus the byte offset of a position.
    fn locate(&self, uri: &Url, pos: Position) -> Option<(&Analysis, FileId, u32)> {
        if uri.scheme() == STD_SCHEME {
            // Any analysis has the same standard library.
            let module = uri.path();
            let (a, file) = self.crates.values().find_map(|(_, a)| {
                let id = a.map.files().find(|(_, f)| std_rel_path(f).is_some_and(|r| std_module_path(&r) == module)).map(|(id, _)| id)?;
                Some((a, id))
            })?;
            let offset = ide::offset_of(&a.map.get(file).text, pos.line, pos.character);
            return Some((a, file, offset));
        }
        let path = to_path(uri)?;
        let key = path_key(&path);
        let root = self.file_root.get(&key).cloned()?;
        let (_, a) = self.crates.get(&root)?;
        let file = ide::file_for_path(&a.map, &path)?;
        let offset = ide::offset_of(&a.map.get(file).text, pos.line, pos.character);
        Some((a, file, offset))
    }

    fn request(&self, req: Request) -> Response {
        let id = req.id.clone();
        let result = match req.method.as_str() {
            HoverRequest::METHOD => self.hover(req).map(|h| serde_json::to_value(h).unwrap()),
            GotoDefinition::METHOD => self.definition(req).map(|d| serde_json::to_value(d).unwrap()),
            STD_SOURCE_METHOD => std_source_request(req),
            _ => return error_response(id, ErrorCode::MethodNotFound, format!("unsupported request `{}`", req.method)),
        };
        match result {
            Ok(v) => Response::new_ok(id, v),
            Err(e) => error_response(id, ErrorCode::InvalidParams, e.to_string()),
        }
    }

    fn hover(&self, req: Request) -> Result<Option<Hover>, Error> {
        let p: lsp_types::HoverParams = serde_json::from_value(req.params)?;
        let tp = p.text_document_position_params;
        let Some((a, file, offset)) = self.locate(&tp.text_document.uri, tp.position) else { return Ok(None) };
        Ok(ide::hover(a, file, offset).map(|(span, text)| Hover {
            contents: HoverContents::Markup(MarkupContent { kind: MarkupKind::Markdown, value: text }),
            range: Some(range(a, span)),
        }))
    }

    fn definition(&self, req: Request) -> Result<Option<GotoDefinitionResponse>, Error> {
        let p: lsp_types::GotoDefinitionParams = serde_json::from_value(req.params)?;
        let tp = p.text_document_position_params;
        let Some((a, file, offset)) = self.locate(&tp.text_document.uri, tp.position) else { return Ok(None) };
        Ok(ide::definition(a, file, offset).and_then(|span| location(a, span)).map(GotoDefinitionResponse::Scalar))
    }
}

fn error_response(id: RequestId, code: ErrorCode, msg: String) -> Response {
    Response::new_err(id, code as i32, msg)
}

fn to_path(uri: &Url) -> Option<PathBuf> {
    uri.to_file_path().ok()
}

fn range(a: &Analysis, span: Span) -> Range {
    let text = &a.map.get(span.file).text;
    let (l0, c0) = ide::position_of(text, span.start);
    let (l1, c1) = ide::position_of(text, span.end);
    Range::new(Position::new(l0, c0), Position::new(l1, c1))
}

/// The path of an embedded standard library file relative to the std root
/// (`core/string.ln`), or `None` for other files.
fn std_rel_path(f: &ln_diag::SourceFile) -> Option<String> {
    f.name.strip_prefix("<std>/").map(str::to_string)
}

/// `core/string.ln` -> `core::string`, `std/lib.ln` -> `std`,
/// `std/sys/mod.ln` -> `std::sys`.
fn std_module_path(rel: &str) -> String {
    let p = rel.trim_end_matches(".ln");
    let p = p.strip_suffix("/lib").or_else(|| p.strip_suffix("/mod")).unwrap_or(p);
    p.replace('/', "::")
}

fn location(a: &Analysis, span: Span) -> Option<Location> {
    let f = a.map.get(span.file);
    let uri = match std_rel_path(f) {
        Some(rel) => Url::parse(&format!("{STD_SCHEME}:{}", std_module_path(&rel))).ok()?,
        None => Url::from_file_path(f.path.as_ref()?).ok()?,
    };
    Some(Location::new(uri, range(a, span)))
}

fn std_source_request(req: Request) -> Result<serde_json::Value, Error> {
    let uri = req.params.get("uri").and_then(|u| u.as_str()).ok_or("missing `uri`")?;
    let url = Url::parse(uri)?;
    if url.scheme() != STD_SCHEME {
        return Err(format!("not a `{STD_SCHEME}:` URI: {uri}").into());
    }
    let module = url.path();
    let text = crate::std_files().find(|(rel, _)| std_module_path(rel) == module).map(|(_, text)| text);
    let text = text.ok_or_else(|| format!("no standard library module `{module}`"))?;
    Ok(serde_json::Value::String(text.to_string()))
}

fn to_lsp_diag(a: &Analysis, d: &Diagnostic) -> lsp_types::Diagnostic {
    let mut message = d.message.clone();
    if let Some(l) = &d.primary_label {
        message.push_str(&format!(" ({l})"));
    }
    for n in &d.notes {
        message.push_str(&format!("\nnote: {n}"));
    }
    for h in &d.helps {
        message.push_str(&format!("\nhelp: {h}"));
    }
    let related: Vec<DiagnosticRelatedInformation> = d
        .labels
        .iter()
        .filter(|l| !l.span.is_dummy())
        .filter_map(|l| Some(DiagnosticRelatedInformation { location: location(a, l.span)?, message: l.message.clone() }))
        .collect();
    let r = if d.span.is_dummy() { Range::default() } else { range(a, d.span) };
    lsp_types::Diagnostic {
        range: r,
        severity: Some(match d.severity {
            Severity::Error => DiagnosticSeverity::ERROR,
            Severity::Warning => DiagnosticSeverity::WARNING,
        }),
        code: Some(NumberOrString::String(d.code.to_string())),
        source: Some("lunar".into()),
        message,
        related_information: (!related.is_empty()).then_some(related),
        ..Default::default()
    }
}

fn publish(conn: &Connection, uri: Url, diagnostics: Vec<lsp_types::Diagnostic>) -> Result<(), Error> {
    let params = PublishDiagnosticsParams { uri, diagnostics, version: None };
    conn.sender.send(Message::Notification(Notification::new(PublishDiagnostics::METHOD.to_string(), params)))?;
    Ok(())
}

/// The root of the crate that `file` belongs to. `dir/foo.ln` and
/// `dir/foo/mod.ln` are module `foo`, declared by `mod foo;` in a file whose
/// child modules live in `dir`: a root file or `mod.ln` in `dir`, or
/// `dir.ln` next to `dir`. The declaring file may itself be a module, so
/// the search repeats upwards. A file nobody declares is its own root.
pub fn crate_root_of(file: &Path, read: &dyn Fn(&Path) -> Option<String>) -> PathBuf {
    let mut cur = file.to_path_buf();
    for _ in 0..32 {
        match declaring_file(&cur, read) {
            Some(parent) if path_key(&parent) != path_key(&cur) => cur = parent,
            _ => break,
        }
    }
    cur
}

fn declaring_file(file: &Path, read: &dyn Fn(&Path) -> Option<String>) -> Option<PathBuf> {
    let is_mod_ln = file.file_name()? == "mod.ln";
    let (name, dir) = if is_mod_ln {
        let d = file.parent()?;
        (d.file_name()?.to_string_lossy().into_owned(), d.parent()?.to_path_buf())
    } else {
        (file.file_stem()?.to_string_lossy().into_owned(), file.parent()?.to_path_buf())
    };
    let mut candidates: Vec<PathBuf> = Vec::new();
    // `dir.ln`, a non-owner file whose children live in `dir/`.
    if let (Some(parent), Some(dname)) = (dir.parent(), dir.file_name()) {
        candidates.push(parent.join(format!("{}.ln", dname.to_string_lossy())));
    }
    // Files in `dir` that own it: `mod.ln` and crate roots.
    if let Ok(entries) = std::fs::read_dir(&dir) {
        let mut files: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "ln")).collect();
        files.sort();
        candidates.extend(files);
    }
    candidates.into_iter().find(|c| path_key(c) != path_key(file) && read(c).is_some_and(|text| declares_mod(&text, &name)))
}

/// Whether `text` has a `mod name;` declaration (possibly `pub`).
fn declares_mod(text: &str, name: &str) -> bool {
    text.lines().any(|line| {
        let t = line.trim();
        let t = t.strip_prefix("pub ").unwrap_or(t).trim_start();
        t.strip_prefix("mod ").is_some_and(|rest| rest.trim_start().strip_prefix(name).is_some_and(|r| r.trim_start().starts_with(';')))
    })
}
