use ast::Span;
use diagnostics::{ArgValue, Note, Related, ToArgValue};
use diagnostics_derive::Diagnose;

use crate::types::Type;

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

#[derive(Diagnose)]
#[diagnose(code = 3, level = "error")]
pub struct ArgumentCountMismatch {
    #[diagnose(span)]
    pub span: Span,
    pub expected: usize,
    pub found: usize,
}

#[derive(Diagnose)]
#[diagnose(code = 4, level = "error")]
pub struct GenericArgumentCountMismatch {
    #[diagnose(span)]
    pub span: Span,
    pub expected: usize,
    pub found: usize,
}

#[derive(Diagnose)]
#[diagnose(code = 5, level = "error")]
pub struct NotCallable {
    #[diagnose(span)]
    pub span: Span,
    pub found: Type,
}

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

#[derive(Diagnose)]
#[diagnose(code = 7, level = "error")]
pub struct UnresolvedImport {
    #[diagnose(span)]
    pub span: Span,
    pub path: String,
}

impl UnresolvedImport {
    pub fn new(span: Span, path: String) -> Self {
        Self { span, path }
    }
}

#[derive(Diagnose)]
#[diagnose(code = 8, level = "error")]
pub struct InvalidGlobTarget {
    #[diagnose(span)]
    pub span: Span,
    pub path: String,
    pub found: String,
}

impl InvalidGlobTarget {
    pub fn new(span: Span, path: String, found: String) -> Self {
        Self { span, path, found }
    }
}

#[derive(Diagnose)]
#[diagnose(code = 9, level = "error")]
pub struct UnresolvedType {
    #[diagnose(span)]
    pub span: Span,
    pub path: String,
}

impl UnresolvedType {
    pub fn new(span: Span, path: String) -> Self {
        Self { span, path }
    }
}

#[derive(Diagnose)]
#[diagnose(code = 10, level = "error")]
pub struct UnresolvedValue {
    #[diagnose(span)]
    pub span: Span,
    pub path: String,
}

impl UnresolvedValue {
    pub fn new(span: Span, path: String) -> Self {
        Self { span, path }
    }
}

#[derive(Diagnose)]
#[diagnose(code = 11, level = "error")]
pub struct AlreadyDefined {
    #[diagnose(span)]
    pub span: Span,
    pub name: String,
    #[diagnose(related)]
    pub original: Option<Related>,
}

impl AlreadyDefined {
    pub fn new(span: Span, name: String, original_span: Span) -> Self {
        Self {
            span,
            name,
            original: Some(Related {
                span: original_span,
                message_id: "already-defined-original",
                args: Vec::new(),
            }),
        }
    }
}

#[derive(Diagnose)]
#[diagnose(code = 12, level = "error")]
pub struct UnknownField {
    #[diagnose(span)]
    pub span: Span,
    pub name: String,
    pub struct_name: String,
}

impl UnknownField {
    pub fn new(span: Span, name: String, struct_name: String) -> Self {
        Self {
            span,
            name,
            struct_name,
        }
    }
}

#[derive(Diagnose)]
#[diagnose(code = 13, level = "error")]
pub struct MissingField {
    #[diagnose(span)]
    pub span: Span,
    pub name: String,
    pub struct_name: String,
}

impl MissingField {
    pub fn new(span: Span, name: String, struct_name: String) -> Self {
        Self {
            span,
            name,
            struct_name,
        }
    }
}

#[derive(Diagnose)]
#[diagnose(code = 14, level = "error")]
pub struct InvalidTupleIndex {
    #[diagnose(span)]
    pub span: Span,
    pub name: String,
    pub found: Type,
}

impl InvalidTupleIndex {
    pub fn new(span: Span, name: String, found: Type) -> Self {
        Self { span, name, found }
    }
}

#[derive(Diagnose)]
#[diagnose(code = 15, level = "error")]
pub struct TupleIndexOutOfBounds {
    #[diagnose(span)]
    pub span: Span,
    pub index: usize,
    pub len: usize,
    pub found: Type,
}

impl TupleIndexOutOfBounds {
    pub fn new(span: Span, index: usize, len: usize, found: Type) -> Self {
        Self {
            span,
            index,
            len,
            found,
        }
    }
}

#[derive(Diagnose)]
#[diagnose(code = 16, level = "error")]
pub struct InvalidFieldAccess {
    #[diagnose(span)]
    pub span: Span,
    pub found: Type,
}

#[derive(Diagnose)]
#[diagnose(code = 17, level = "error")]
pub struct MissingTraitItem {
    #[diagnose(span)]
    pub span: Span,
    pub kind: String,
    pub name: String,
    pub trait_name: String,
}

impl MissingTraitItem {
    pub fn new(span: Span, kind: String, name: String, trait_name: String) -> Self {
        Self {
            span,
            kind,
            name,
            trait_name,
        }
    }
}

#[derive(Diagnose)]
#[diagnose(code = 18, level = "error")]
pub struct MissingSelfParam {
    #[diagnose(span)]
    pub span: Span,
    pub name: String,
    pub trait_name: String,
    #[diagnose(related)]
    pub trait_declared_at: Option<Related>,
}

impl MissingSelfParam {
    pub fn new(span: Span, name: String, trait_name: String, trait_span: Span) -> Self {
        Self {
            span,
            name,
            trait_name,
            trait_declared_at: Some(Related {
                span: trait_span,
                message_id: "missing-self-param-declared-here",
                args: Vec::new(),
            }),
        }
    }
}

#[derive(Diagnose)]
#[diagnose(code = 19, level = "error")]
pub struct UnexpectedSelfParam {
    #[diagnose(span)]
    pub span: Span,
    pub name: String,
    pub trait_name: String,
    #[diagnose(related)]
    pub trait_declared_at: Option<Related>,
}

impl UnexpectedSelfParam {
    pub fn new(span: Span, name: String, trait_name: String, trait_span: Span) -> Self {
        Self {
            span,
            name,
            trait_name,
            trait_declared_at: Some(Related {
                span: trait_span,
                message_id: "unexpected-self-param-declared-here",
                args: Vec::new(),
            }),
        }
    }
}

#[derive(Diagnose)]
#[diagnose(code = 20, level = "error")]
pub struct SelfOutsideImplOrTrait {
    #[diagnose(span)]
    pub span: Span,
}

impl SelfOutsideImplOrTrait {
    pub fn new(span: Span) -> Self {
        Self { span }
    }
}

diagnostics::catalog! {
    pub(crate) enum TypeCheckDiagnostic {
        TypeMismatch,
        CyclicType,
        ArgumentCountMismatch,
        GenericArgumentCountMismatch,
        NotCallable,
        AnnotationsNeeded,
        UnresolvedImport,
        InvalidGlobTarget,
        UnresolvedType,
        UnresolvedValue,
        AlreadyDefined,
        UnknownField,
        MissingField,
        InvalidTupleIndex,
        TupleIndexOutOfBounds,
        InvalidFieldAccess,
        MissingTraitItem,
        MissingSelfParam,
        UnexpectedSelfParam,
        SelfOutsideImplOrTrait,
    }
}

static EN_US: std::sync::LazyLock<diagnostics::Catalog> = std::sync::LazyLock::new(|| {
    diagnostics::Catalog::new("en-US", &[include_str!("../locales/en-US/typecheck.ftl")])
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locale {
    EnUs,
}

pub(crate) fn catalog(locale: Locale) -> &'static diagnostics::Catalog {
    match locale {
        Locale::EnUs => &EN_US,
    }
}

#[derive(Default)]
pub(crate) struct Diagnostics(Vec<TypeCheckDiagnostic>);

impl Diagnostics {
    pub(crate) fn push(&mut self, diagnostic: impl Into<TypeCheckDiagnostic>) {
        self.0.push(diagnostic.into());
    }

    pub(crate) fn into_vec(self) -> Vec<TypeCheckDiagnostic> {
        self.0
    }
}

// Test-only: reads the tuple field private to this module, so it
// can't live in `tests.rs` itself. Kept in its own impl block, out of
// the production API above, so it reads as test support rather than
// part of `Diagnostics`'s real interface. Used by
// `TypeCheckContext::diagnostics` (in `tests.rs`) to read diagnostics
// off a `TypeCheckContext` non-destructively, unlike `into_vec`.
#[cfg(test)]
impl Diagnostics {
    pub(crate) fn as_slice(&self) -> &[TypeCheckDiagnostic] {
        &self.0
    }
}
