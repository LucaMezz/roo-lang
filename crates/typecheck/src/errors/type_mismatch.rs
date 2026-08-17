use ast::Span;
use diagnostics::{ArgValue, Note, Related, ToArgValue};
use diagnostics_derive::Diagnose;

use crate::render::Type;

#[derive(Diagnose)]
#[diagnose(code = 1, level = "error")]
pub struct TypeMismatch {
    #[diagnose(span)]
    pub span: Span,
    pub expected: Type,
    pub found: Type,
    #[diagnose(emphasize_in = "expected")]
    pub expected_highlight: Type,
    #[diagnose(emphasize_in = "found")]
    pub found_highlight: Type,
    #[diagnose(related)]
    pub expected_due_to: Option<Related>,
    #[diagnose(note)]
    pub generic_note: Option<Note>,
    #[diagnose(related)]
    pub expected_provenance: Option<Related>,
    #[diagnose(related)]
    pub found_provenance: Option<Related>,
}

pub fn generic_note(name: String, other: &Type) -> Note {
    Note {
        message_id: "type-mismatch-generic-note",
        args: vec![
            ("name", ArgValue::Text(name)),
            ("other", other.to_arg_value()),
        ],
    }
}

pub fn expected_due_to(span: Span) -> Related {
    Related {
        span,
        message_id: "type-mismatch-expected-due-to",
        args: Vec::new(),
    }
}

pub fn expected_because_of(span: Span) -> Related {
    Related {
        span,
        message_id: "type-mismatch-expected-because-of",
        args: Vec::new(),
    }
}

pub fn provenance(span: Span, side: &'static str, kind: &Type) -> Related {
    Related {
        span,
        message_id: "type-mismatch-provenance",
        args: vec![
            ("side", ArgValue::Text(side.to_owned())),
            ("kind", kind.to_arg_value()),
        ],
    }
}
