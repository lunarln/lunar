use std::path::PathBuf;

/// Index of a file inside a [`SourceMap`].
pub type FileId = u32;

/// A byte range inside one source file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Span {
    pub file: FileId,
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub const DUMMY: Span = Span { file: u32::MAX, start: 0, end: 0 };

    pub const fn new(file: FileId, start: u32, end: u32) -> Self {
        Self { file, start, end }
    }

    /// The smallest span covering both `self` and `other`.
    pub fn to(self, other: Span) -> Span {
        if self.file != other.file {
            return self;
        }
        Span { file: self.file, start: self.start.min(other.start), end: self.end.max(other.end) }
    }

    /// Empty span at the end of `self`.
    pub fn shrink_to_hi(self) -> Span {
        Span { file: self.file, start: self.end, end: self.end }
    }

    pub fn shrink_to_lo(self) -> Span {
        Span { file: self.file, start: self.start, end: self.start }
    }

    pub fn is_dummy(self) -> bool {
        self.file == u32::MAX
    }

    pub fn len(self) -> u32 {
        self.end - self.start
    }

    pub fn is_empty(self) -> bool {
        self.start == self.end
    }
}

/// One loaded source file.
#[derive(Debug)]
pub struct SourceFile {
    /// Display name (path relative to the invocation, or `<std>/...`).
    pub name: String,
    /// Path on disk, if the file came from disk.
    pub path: Option<PathBuf>,
    pub text: String,
    line_starts: Vec<u32>,
}

impl SourceFile {
    /// 1-based line and column of a byte offset.
    pub fn line_col(&self, offset: u32) -> (u32, u32) {
        let line = match self.line_starts.binary_search(&offset) {
            Ok(l) => l,
            Err(l) => l - 1,
        };
        let col = self.text[self.line_starts[line] as usize..offset as usize].chars().count();
        (line as u32 + 1, col as u32 + 1)
    }
}

/// All source files involved in a compilation.
#[derive(Default, Debug)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, name: impl Into<String>, path: Option<PathBuf>, text: impl Into<String>) -> FileId {
        let text = text.into();
        let mut line_starts = vec![0];
        line_starts.extend(text.match_indices('\n').map(|(i, _)| i as u32 + 1));
        self.files.push(SourceFile { name: name.into(), path, text, line_starts });
        (self.files.len() - 1) as FileId
    }

    pub fn get(&self, id: FileId) -> &SourceFile {
        &self.files[id as usize]
    }

    pub fn files(&self) -> impl Iterator<Item = (FileId, &SourceFile)> {
        self.files.iter().enumerate().map(|(i, f)| (i as FileId, f))
    }

    pub fn snippet(&self, span: Span) -> &str {
        if span.is_dummy() {
            return "";
        }
        &self.get(span.file).text[span.start as usize..span.end as usize]
    }

    /// `file:line:col` for a span, used for runtime panic locations.
    pub fn location(&self, span: Span) -> (String, u32, u32) {
        if span.is_dummy() {
            return ("<unknown>".into(), 0, 0);
        }
        let f = self.get(span.file);
        let (l, c) = f.line_col(span.start);
        (f.name.clone(), l, c)
    }
}
