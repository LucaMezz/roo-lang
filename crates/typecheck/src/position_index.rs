use ast::Span;

use crate::SymbolId;

#[derive(Default)]
pub(crate) struct PositionIndex {
    symbols: Vec<(Span, SymbolId)>,

    primitives: Vec<(Span, &'static str)>,
}

impl PositionIndex {
    pub(crate) fn record_symbol(&mut self, span: Span, symbol: SymbolId) {
        self.symbols.push((span, symbol));
    }

    pub(crate) fn record_primitive(&mut self, span: Span, name: &'static str) {
        self.primitives.push((span, name));
    }

    pub(crate) fn symbol_at(&self, offset: usize) -> Option<SymbolId> {
        self.symbols
            .iter()
            .filter(|(span, _)| span.start <= offset && offset < span.end)
            .min_by_key(|(span, _)| span.end - span.start)
            .map(|(_, symbol)| *symbol)
    }

    pub(crate) fn type_name_at(&self, offset: usize) -> Option<&'static str> {
        self.primitives
            .iter()
            .filter(|(span, _)| span.start <= offset && offset < span.end)
            .min_by_key(|(span, _)| span.end - span.start)
            .map(|(_, name)| *name)
    }
}
