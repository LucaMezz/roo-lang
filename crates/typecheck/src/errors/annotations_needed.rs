use ast::Span;
use diagnostics::Note;
use diagnostics_derive::Diagnose;

#[derive(Diagnose)]
#[diagnose(code = 6, level = "error")]
pub struct AnnotationsNeeded {
    #[diagnose(span)]
    pub span: Span,
    #[diagnose(note)]
    pub notes: Vec<Note>,
}

impl AnnotationsNeeded {
    pub fn new(span: Span) -> Self {
        let notes = vec![];
        Self { span, notes }
    }
}
