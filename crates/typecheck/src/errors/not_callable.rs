use ast::Span;
use diagnostics_derive::Diagnose;

use crate::types::Type;

#[derive(Diagnose)]
#[diagnose(code = 5, level = "error")]
pub struct NotCallable {
    #[diagnose(span)]
    pub span: Span,
    pub found: Type,
}
