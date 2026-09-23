//! Pretty-printing of diagnostics with source snippets via `ariadne`.

use std::io;

use ariadne::{ColorGenerator, Label as AriadneLabel, Report, ReportKind};

use crate::diagnostic::{Diagnostic, Severity};
use crate::span::{FileId, Span};

/// Names + contents of all source files known to the compiler.
///
/// `FileId` indexes into `files`; the ariadne cache is keyed by file name so
/// rendered reports show paths rather than numeric ids.
#[derive(Debug, Default)]
pub struct SourceCache {
    files: Vec<SourceEntry>,
}

#[derive(Debug)]
struct SourceEntry {
    name: String,
    text: String,
}

impl SourceCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a source file; returns its `FileId`.
    pub fn add(&mut self, name: impl Into<String>, text: impl Into<String>) -> FileId {
        let id = FileId(u32::try_from(self.files.len()).unwrap_or(u32::MAX - 1));
        self.files.push(SourceEntry {
            name: name.into(),
            text: text.into(),
        });
        id
    }

    pub fn name(&self, id: FileId) -> &str {
        self.files
            .get(id.0 as usize)
            .map_or("<unknown>", |e| e.name.as_str())
    }

    pub fn text(&self, id: FileId) -> &str {
        self.files
            .get(id.0 as usize)
            .map_or("", |e| e.text.as_str())
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

fn ariadne_kind(severity: Severity) -> ReportKind<'static> {
    match severity {
        Severity::Error => ReportKind::Error,
        Severity::Warning => ReportKind::Warning,
        Severity::Note | Severity::Help => ReportKind::Advice,
    }
}

/// Render `diags` with source snippets into `out` (any `io::Write`).
///
/// Diagnostics whose span references an unknown file are rendered as plain
/// `E2xxx: message` lines without a snippet.
///
/// # Errors
/// Propagates `io::Error` from the output writer.
pub fn render_diagnostics(
    diags: &[Diagnostic],
    cache: &SourceCache,
    mut out: impl io::Write,
) -> io::Result<()> {
    let sources: Vec<(String, String)> = cache
        .files
        .iter()
        .map(|e| (e.name.clone(), e.text.clone()))
        .collect();

    for diag in diags {
        let Some(span) = diag.span else {
            render_detached(diag, &mut out)?;
            continue;
        };
        if usize::try_from(span.file.0).map_or(true, |i| i >= cache.len()) {
            render_detached(diag, &mut out)?;
            continue;
        }
        render_one(diag, span, cache, &sources, &mut out)?;
    }
    Ok(())
}

fn render_one(
    diag: &Diagnostic,
    span: Span,
    cache: &SourceCache,
    sources: &[(String, String)],
    out: &mut impl io::Write,
) -> io::Result<()> {
    let file_name = cache.name(span.file).to_string();
    let mut colors = ColorGenerator::new();
    let primary_color = colors.next();

    let anchor = offset_to_anchor(cache.text(span.file), span.start);
    let mut builder = Report::build(
        ariadne_kind(diag.severity),
        (file_name.clone(), anchor..anchor),
    );
    if let Some(code) = diag.code {
        builder = builder.with_code(code);
    }
    builder = builder.with_message(&diag.message).with_label(
        AriadneLabel::new((file_name.clone(), span.range()))
            .with_message(&diag.message)
            .with_color(primary_color),
    );
    for label in &diag.labels {
        builder = builder.with_label(
            AriadneLabel::new((cache.name(label.span.file).to_string(), label.span.range()))
                .with_message(&label.message)
                .with_color(colors.next()),
        );
    }
    for note in &diag.notes {
        builder = builder.with_note(note);
    }
    builder
        .finish()
        .write(ariadne::sources(sources.to_vec()), out)
}

fn render_detached(diag: &Diagnostic, out: &mut impl io::Write) -> io::Result<()> {
    let severity = match diag.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Note => "note",
        Severity::Help => "help",
    };
    match diag.code {
        Some(code) => writeln!(out, "{severity}[{code}]: {}", diag.message),
        None => writeln!(out, "{severity}: {}", diag.message),
    }
}

/// ariadne anchors reports at a char offset; clamp to the text length so a
/// point-span at EOF stays in bounds.
fn offset_to_anchor(text: &str, offset: u32) -> usize {
    (offset as usize).min(text.len())
}
