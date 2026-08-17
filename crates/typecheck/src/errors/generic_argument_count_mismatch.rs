use ast::Span;
use diagnostics_derive::Diagnose;

#[derive(Diagnose)]
#[diagnose(code = 4, level = "error")]
pub struct GenericArgumentCountMismatch {
    #[diagnose(span)]
    pub span: Span,
    pub expected: usize,
    pub found: usize,
}
