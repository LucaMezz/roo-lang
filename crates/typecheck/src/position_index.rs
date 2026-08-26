//! Contains the [`PositionIndex`] data structure, which maps spans within
//! the source code, to bindings and types discovered during type checking.
use ast::Span;

use crate::BindingId;

/// An index that maps spans within source code, to bindings discovered and
/// created within type checking. Facilitates queries for implementing the
/// LSP server which provides things like type annotations and docstrings
/// on hover within an IDE.
#[derive(Default)]
pub(crate) struct PositionIndex {
    bindings: Vec<(Span, BindingId)>,

    primitives: Vec<(Span, &'static str)>,
}

impl PositionIndex {
    /// Records a discovered binding within type checked source code at the given
    /// span.
    pub(crate) fn record_binding(&mut self, span: Span, binding: BindingId) {
        self.bindings.push((span, binding));
    }

    /// Records a discovered explicit primitive type within the type checked
    /// source code at the given span.
    pub(crate) fn record_primitive(&mut self, span: Span, name: &'static str) {
        self.primitives.push((span, name));
    }

    /// Queries the index to see if there is a binding which is known to exist
    /// as the given offset in the source code.
    pub(crate) fn binding_at(&self, offset: usize) -> Option<BindingId> {
        self.bindings
            .iter()
            .filter(|(span, _)| span.start <= offset && offset < span.end)
            .min_by_key(|(span, _)| span.end - span.start)
            .map(|(_, binding)| *binding)
    }

    /// Queries the index to see if there is an explitit primitive type at
    /// the given offset in the source code.
    pub(crate) fn type_name_at(&self, offset: usize) -> Option<&'static str> {
        self.primitives
            .iter()
            .filter(|(span, _)| span.start <= offset && offset < span.end)
            .min_by_key(|(span, _)| span.end - span.start)
            .map(|(_, name)| *name)
    }
}
