use ast::Span;
use diagnostics_derive::Diagnose;

#[derive(Diagnose)]
#[diagnose(code = 3, level = "error")]
pub struct ArgumentCountMismatch {
    #[diagnose(span)]
    pub span: Span,
    pub expected: usize,
    pub found: usize,
}
