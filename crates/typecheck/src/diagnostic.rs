use ast::Span;
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Error,
    Warning,
    Note,
    Help,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    primary_span: Span,
    level: Level,
    message: String,
    related: Vec<(Span, String)>,
    notes: Vec<String>,
    emphasis: Vec<Range<usize>>,
}

impl Diagnostic {
    pub fn error(span: Span, message: impl Into<String>) -> Self {
        Self::new(span, Level::Error, message)
    }

    pub fn warning(span: Span, message: impl Into<String>) -> Self {
        Self::new(span, Level::Warning, message)
    }

    pub fn note(span: Span, message: impl Into<String>) -> Self {
        Self::new(span, Level::Note, message)
    }

    pub fn help(span: Span, message: impl Into<String>) -> Self {
        Self::new(span, Level::Help, message)
    }

    pub fn new(span: Span, level: Level, message: impl Into<String>) -> Self {
        Self {
            primary_span: span,
            level,
            message: message.into(),
            related: Vec::new(),
            notes: Vec::new(),
            emphasis: Vec::new(),
        }
    }

    pub fn with_related(mut self, span: Span, message: impl Into<String>) -> Self {
        self.related.push((span, message.into()));
        self
    }

    pub fn with_note(mut self, message: impl Into<String>) -> Self {
        self.notes.push(message.into());
        self
    }

    pub fn with_emphasis(mut self, range: Range<usize>) -> Self {
        self.emphasis.push(range);
        self
    }

    pub fn cyclic_type(span: Span, expected: &str, actual: &str) -> Self {
        Self::error(span, "cyclic type of infinite size")
            .with_note(format!("expected type `{expected}`"))
            .with_note(format!("found type `{actual}`"))
    }

    pub fn level(&self) -> Level {
        self.level
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn span(&self) -> Span {
        self.primary_span
    }

    pub fn related(&self) -> &[(Span, String)] {
        &self.related
    }

    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    pub fn emphasis(&self) -> &[Range<usize>] {
        &self.emphasis
    }
}

#[derive(Debug, Default, Clone)]
pub(crate) struct Diagnostics(Vec<Diagnostic>);

impl Diagnostics {
    pub(crate) fn push(&mut self, diagnostic: Diagnostic) {
        self.0.push(diagnostic);
    }

    #[cfg(test)]
    pub(crate) fn as_slice(&self) -> &[Diagnostic] {
        &self.0
    }

    pub(crate) fn into_vec(self) -> Vec<Diagnostic> {
        self.0
    }
}
