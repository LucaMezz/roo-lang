mod annotations_needed;
mod argument_count_mismatch;
mod cyclic_type;
mod generic_argument_count_mismatch;
mod not_callable;
mod type_mismatch;

pub(crate) use annotations_needed::AnnotationsNeeded;
pub(crate) use argument_count_mismatch::ArgumentCountMismatch;
pub(crate) use cyclic_type::CyclicType;
pub(crate) use generic_argument_count_mismatch::GenericArgumentCountMismatch;
pub(crate) use not_callable::NotCallable;
pub(crate) use type_mismatch::TypeMismatch;
pub(crate) use type_mismatch::{expected_because_of, expected_due_to, generic_note, provenance};

diagnostics::catalog! {
    pub(crate) enum TypeCheckDiagnostic {
        TypeMismatch,
        CyclicType,
        ArgumentCountMismatch,
        GenericArgumentCountMismatch,
        NotCallable,
        AnnotationsNeeded,
    }
}

static EN_US: std::sync::LazyLock<diagnostics::Catalog> = std::sync::LazyLock::new(|| {
    diagnostics::Catalog::new(
        "en-US",
        &[include_str!("../../locales/en-US/typecheck.ftl")],
    )
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

    #[cfg(test)]
    pub(crate) fn as_slice(&self) -> &[TypeCheckDiagnostic] {
        &self.0
    }
}
