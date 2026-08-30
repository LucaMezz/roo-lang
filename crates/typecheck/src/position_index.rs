//! Contains the [`PositionIndex`] data structure, which maps spans within
//! the source code, to defs and types discovered during type checking.
use std::collections::HashMap;

use ast::Span;

use crate::DefId;

/// An index that maps spans within source code, to defs discovered and
/// created within type checking. Facilitates queries for implementing the
/// LSP server which provides things like type annotations and docstrings
/// on hover within an IDE.
#[derive(Default)]
pub(crate) struct PositionIndex {
    defs: Vec<(Span, DefId)>,

    defs_by_span: HashMap<Span, DefId>,

    primitives: Vec<(Span, &'static str)>,
}

impl PositionIndex {
    /// Records a discovered def within type checked source code at the given
    /// span.
    pub(crate) fn record_def(&mut self, span: Span, def: DefId) {
        self.defs.push((span, def));
        self.defs_by_span.insert(span, def);
    }

    /// Records a discovered explicit primitive type within the type checked
    /// source code at the given span.
    pub(crate) fn record_primitive(&mut self, span: Span, name: &'static str) {
        self.primitives.push((span, name));
    }

    /// Queries the index to see if there is a def which is known to exist
    /// as the given offset in the source code.
    pub(crate) fn def_at(&self, offset: usize) -> Option<DefId> {
        self.defs
            .iter()
            .filter(|(span, _)| span.start <= offset && offset < span.end)
            .min_by_key(|(span, _)| span.end - span.start)
            .map(|(_, def)| *def)
    }

    /// Looks up the def which was recorded at exactly the given span.
    pub(crate) fn def_at_span(&self, span: Span) -> Option<DefId> {
        self.defs_by_span.get(&span).copied()
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
