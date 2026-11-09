//! Diagnostics infrastructure for the Lunar compiler.
//!
//! Every error the compiler reports carries a *scoped* code such as
//! `type::array_pointy_items`, rendered as `err(type::array_pointy_items): ...`.

pub mod codes;
mod render;
mod source;

pub use codes::{ALL_CODES, ty};
pub use render::{render, render_to_string};
pub use source::{FileId, SourceFile, SourceMap, Span};

/// How severe a diagnostic is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// A scoped diagnostic code, e.g. `type::array_pointy_items`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Code {
    pub scope: &'static str,
    pub name: &'static str,
}

impl Code {
    pub const fn new(scope: &'static str, name: &'static str) -> Self {
        Self { scope, name }
    }
}

impl std::fmt::Display for Code {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}::{}", self.scope, self.name)
    }
}

/// A secondary label attached to a diagnostic.
#[derive(Clone, Debug)]
pub struct Label {
    pub span: Span,
    pub message: String,
}

/// A single compiler diagnostic.
#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: Code,
    pub message: String,
    pub span: Span,
    pub primary_label: Option<String>,
    pub labels: Vec<Label>,
    pub notes: Vec<String>,
    pub helps: Vec<String>,
}

impl Diagnostic {
    pub fn error(code: Code, span: Span, message: impl Into<String>) -> Self {
        Self { severity: Severity::Error, code, message: message.into(), span, primary_label: None, labels: Vec::new(), notes: Vec::new(), helps: Vec::new() }
    }

    pub fn warning(code: Code, span: Span, message: impl Into<String>) -> Self {
        Self { severity: Severity::Warning, ..Self::error(code, span, message) }
    }

    /// Text shown under the primary span.
    pub fn with_label(mut self, msg: impl Into<String>) -> Self {
        self.primary_label = Some(msg.into());
        self
    }

    pub fn with_secondary(mut self, span: Span, msg: impl Into<String>) -> Self {
        self.labels.push(Label { span, message: msg.into() });
        self
    }

    pub fn with_note(mut self, msg: impl Into<String>) -> Self {
        self.notes.push(msg.into());
        self
    }

    pub fn with_help(mut self, msg: impl Into<String>) -> Self {
        self.helps.push(msg.into());
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// Collects diagnostics produced by a compiler pass.
#[derive(Default, Debug)]
pub struct Diagnostics {
    list: Vec<Diagnostic>,
}

impl Diagnostics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, d: Diagnostic) {
        self.list.push(d);
    }

    pub fn error(&mut self, code: Code, span: Span, message: impl Into<String>) -> &mut Diagnostic {
        self.list.push(Diagnostic::error(code, span, message));
        self.list.last_mut().unwrap()
    }

    pub fn extend(&mut self, other: impl IntoIterator<Item = Diagnostic>) {
        self.list.extend(other);
    }

    pub fn has_errors(&self) -> bool {
        self.list.iter().any(Diagnostic::is_error)
    }

    pub fn error_count(&self) -> usize {
        self.list.iter().filter(|d| d.is_error()).count()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Diagnostic> {
        self.list.iter()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn take(&mut self) -> Vec<Diagnostic> {
        std::mem::take(&mut self.list)
    }

    pub fn into_vec(self) -> Vec<Diagnostic> {
        self.list
    }
}
