use crate::{Diagnostic, Severity, SourceMap};
use ariadne::{Color, Config, Fmt, IndexType, Label, Report, ReportKind, Source};
use std::io::Write;

struct Cache<'a> {
    map: &'a SourceMap,
    sources: Vec<Option<Source<&'a str>>>,
}

impl<'a> ariadne::Cache<u32> for Cache<'a> {
    type Storage = &'a str;

    fn fetch(&mut self, id: &u32) -> Result<&Source<&'a str>, impl std::fmt::Debug> {
        let idx = *id as usize;
        if self.sources.len() <= idx {
            self.sources.resize_with(idx + 1, || None);
        }
        if self.sources[idx].is_none() {
            self.sources[idx] = Some(Source::from(self.map.get(*id).text.as_str()));
        }
        Ok::<_, String>(self.sources[idx].as_ref().unwrap())
    }

    fn display<'b>(&self, id: &'b u32) -> Option<impl std::fmt::Display + 'b> {
        Some(self.map.get(*id).name.clone())
    }
}

/// Render diagnostics with ariadne to `out`.
pub fn render(map: &SourceMap, diags: &[Diagnostic], color: bool, out: &mut dyn Write) {
    let mut cache = Cache { map, sources: Vec::new() };
    for d in diags {
        let (kind_word, c) = match d.severity {
            Severity::Error => ("err", Color::Red),
            Severity::Warning => ("warn", Color::Yellow),
        };
        let head = format!("{kind_word}({})", d.code);
        let config = Config::default().with_index_type(IndexType::Byte).with_color(color);
        if d.span.is_dummy() {
            render_without_source(d, &head, c, color, out);
            continue;
        }
        let range = d.span.start as usize..d.span.end as usize;
        let mut report = Report::build(ReportKind::Custom(&head, c), (d.span.file, range.clone())).with_config(config).with_message(&d.message);
        let primary = Label::new((d.span.file, range)).with_color(c);
        // Ariadne only underlines labels that carry a message.
        let primary = primary.with_message(d.primary_label.as_deref().unwrap_or("here"));
        report = report.with_label(primary);
        for l in &d.labels {
            if l.span.is_dummy() {
                continue;
            }
            report = report.with_label(Label::new((l.span.file, l.span.start as usize..l.span.end as usize)).with_message(&l.message).with_color(Color::Blue));
        }
        for n in &d.notes {
            report = report.with_note(n);
        }
        for h in &d.helps {
            report = report.with_help(h);
        }
        let _ = report.finish().write(&mut cache, &mut *out);
    }
}

/// A diagnostic that is not about a place in the source (a missing `main`,
/// a linker failure, ...): the header line as ariadne would print it, then
/// its notes and helps. Multi-line text, such as a linker's output, is
/// indented under its `note:` / `help:` prefix.
fn render_without_source(d: &Diagnostic, head: &str, c: Color, color: bool, out: &mut dyn Write) {
    let head = format!("{head}:");
    let head = if color { head.fg(c).to_string() } else { head };
    let _ = writeln!(out, "{head} {}", d.message);
    for (kind, text) in d.notes.iter().map(|n| ("note", n)).chain(d.helps.iter().map(|h| ("help", h))) {
        let mut lines = text.trim_end().lines();
        let _ = writeln!(out, "  {kind}: {}", lines.next().unwrap_or(""));
        let indent = " ".repeat(kind.len() + 4);
        for l in lines {
            let _ = writeln!(out, "{indent}{l}");
        }
    }
}

pub fn render_to_string(map: &SourceMap, diags: &[Diagnostic]) -> String {
    let mut buf = Vec::new();
    render(map, diags, false, &mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}
