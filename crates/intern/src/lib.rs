use std::collections::HashMap;
use std::fmt;

#[derive(Clone, Copy, Eq, PartialEq, Hash)]
pub struct Symbol(u32);

impl fmt::Debug for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Symbol({})", self.0)
    }
}

#[derive(Default, Clone)]
pub struct Interner {
    strings: Vec<Box<str>>,
    ids: HashMap<Box<str>, Symbol>,
}

impl Interner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn intern(&mut self, s: &str) -> Symbol {
        if let Some(&sym) = self.ids.get(s) {
            return sym;
        }

        let boxed: Box<str> = s.into();
        let sym = Symbol(self.strings.len() as u32);
        self.ids.insert(boxed.clone(), sym);
        self.strings.push(boxed);
        sym
    }

    pub fn get(&self, s: &str) -> Option<Symbol> {
        self.ids.get(s).copied()
    }

    pub fn resolve(&self, sym: Symbol) -> &str {
        &self.strings[sym.0 as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interning_the_same_string_returns_the_same_symbol() {
        let mut interner = Interner::new();
        assert_eq!(interner.intern("foo"), interner.intern("foo"));
    }

    #[test]
    fn interning_different_strings_returns_different_symbols() {
        let mut interner = Interner::new();
        assert_ne!(interner.intern("foo"), interner.intern("bar"));
    }

    #[test]
    fn resolves_back_to_the_original_string() {
        let mut interner = Interner::new();
        let sym = interner.intern("foo");
        assert_eq!(interner.resolve(sym), "foo");
    }
}
