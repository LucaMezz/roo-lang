use ast::Span;
use diagnostics::{Note, ToArgValue};
use diagnostics_derive::Diagnose;

use crate::render::Type;

#[derive(Diagnose)]
#[diagnose(code = 2, level = "error")]
pub struct CyclicType {
    #[diagnose(span)]
    pub span: Span,
    pub expected: Type,
    pub found: Type,
    #[diagnose(note)]
    pub notes: Vec<Note>,
}

impl CyclicType {
    pub fn new(span: Span, expected: Type, found: Type) -> Self {
        let notes = vec![
            Note {
                message_id: "cyclic-type-expected",
                args: vec![("expected", expected.to_arg_value())],
            },
            Note {
                message_id: "cyclic-type-found",
                args: vec![("found", found.to_arg_value())],
            },
        ];
        Self {
            span,
            expected,
            found,
            notes,
        }
    }
}
