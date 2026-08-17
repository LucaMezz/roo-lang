use ast::Span;
use diagnostics_derive::Diagnose;

use crate::render::Type;

#[derive(Diagnose)]
#[diagnose(code = 5, level = "error")]
pub struct NotCallable {
    #[diagnose(span)]
    pub span: Span,
    pub found: Type,
}
