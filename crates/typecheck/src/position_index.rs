//! Contains the [`PositionIndex`] data structure, which maps spans within
//! the source code, to symbols and types discovered during type checking.
use ast::Span;

use crate::SymbolId;

/// An index that maps spans within source code, to symbols discovered and
/// created within type checking. Facilitates queries for implementing the
/// LSP server which provides things like type annotations and docstrings
/// on hover within an IDE.
#[derive(Default)]
pub(crate) struct PositionIndex {
    symbols: Vec<(Span, SymbolId)>,

    primitives: Vec<(Span, &'static str)>,
}

impl PositionIndex {
    /// Records a discovered symbol within type checked source code at the given
    /// span.
    pub(crate) fn record_symbol(&mut self, span: Span, symbol: SymbolId) {
        self.symbols.push((span, symbol));
    }

    /// Records a discovered explicit primitive type within the type checked
    /// source code at the given span.
    pub(crate) fn record_primitive(&mut self, span: Span, name: &'static str) {
        self.primitives.push((span, name));
    }

    /// Queries the index to see if there is a symbol which is known to exist
    /// as the given offset in the source code.
    pub(crate) fn symbol_at(&self, offset: usize) -> Option<SymbolId> {
        self.symbols
            .iter()
            .filter(|(span, _)| span.start <= offset && offset < span.end)
            .min_by_key(|(span, _)| span.end - span.start)
            .map(|(_, symbol)| *symbol)
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
