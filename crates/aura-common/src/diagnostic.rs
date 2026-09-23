use crate::span::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Note,
    Help,
}

/// A labeled sub-span attached to a diagnostic (e.g. "defined here").
#[derive(Debug, Clone)]
pub struct Label {
    pub span: Span,
    pub message: String,
}

/// A rich compiler diagnostic with optional primary span, extra labels,
/// and free-form notes.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    /// Namespaced error code, e.g. `E2001`. `None` for internal notes.
    pub code: Option<&'static str>,
    pub severity: Severity,
    pub message: String,
    /// Primary span the diagnostic points at.
    pub span: Option<Span>,
    pub labels: Vec<Label>,
    pub notes: Vec<String>,
}

impl Diagnostic {
    pub fn error(code: &'static str, message: impl Into<String>, span: Span) -> Self {
        Self {
            code: Some(code),
            severity: Severity::Error,
            message: message.into(),
            span: Some(span),
            labels: Vec::new(),
            notes: Vec::new(),
        }
    }

    pub fn warning(code: &'static str, message: impl Into<String>, span: Span) -> Self {
        Self {
            severity: Severity::Warning,
            ..Self::error(code, message, span)
        }
    }

    /// Diagnostic not tied to a source location.
    pub fn detached(severity: Severity, message: impl Into<String>) -> Self {
        Self {
            code: None,
            severity,
            message: message.into(),
            span: None,
            labels: Vec::new(),
            notes: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_label(mut self, span: Span, message: impl Into<String>) -> Self {
        self.labels.push(Label {
            span,
            message: message.into(),
        });
        self
    }

    #[must_use]
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// Accumulator for diagnostics produced by a compiler phase.
#[derive(Debug, Default)]
pub struct DiagnosticSink {
    diags: Vec<Diagnostic>,
}

impl DiagnosticSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, diag: Diagnostic) {
        self.diags.push(diag);
    }

    pub fn error(&mut self, code: &'static str, message: impl Into<String>, span: Span) {
        self.push(Diagnostic::error(code, message, span));
    }

    pub fn has_errors(&self) -> bool {
        self.diags.iter().any(Diagnostic::is_error)
    }

    pub fn is_empty(&self) -> bool {
        self.diags.is_empty()
    }

    pub fn len(&self) -> usize {
        self.diags.len()
    }

    pub fn extend(&mut self, other: impl IntoIterator<Item = Diagnostic>) {
        self.diags.extend(other);
    }

    pub fn as_slice(&self) -> &[Diagnostic] {
        &self.diags
    }

    pub fn into_vec(self) -> Vec<Diagnostic> {
        self.diags
    }
}

impl IntoIterator for DiagnosticSink {
    type Item = Diagnostic;
    type IntoIter = std::vec::IntoIter<Diagnostic>;

    fn into_iter(self) -> Self::IntoIter {
        self.diags.into_iter()
    }
}
