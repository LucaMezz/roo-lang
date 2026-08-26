use std::ops::Range;
use std::vec;

use chumsky::extra::SimpleState;
use chumsky::input::{IterInput, MapExtra};
use chumsky::prelude::*;
use lexer::Token;

mod annotation;
mod expr;
mod ident;
mod item;
mod literal;
mod module;
mod operator;
mod pat;
mod path;
mod stmt;
mod ty;
mod use_tree;
mod visibility;

pub use annotation::*;
pub use expr::*;
pub use ident::*;
pub use item::*;
pub use literal::*;
pub use module::*;
pub use operator::*;
pub use pat::*;
pub use path::*;
pub use stmt::*;
pub use ty::*;
pub use use_tree::*;
pub use visibility::*;

pub type ParserInput<'src> = IterInput<vec::IntoIter<(Token<'src>, SimpleSpan)>, SimpleSpan>;

pub fn input(tokens: Vec<(Token<'_>, Range<usize>)>) -> ParserInput<'_> {
    let eoi = tokens
        .last()
        .map(|(_, span)| SimpleSpan::from(span.end..span.end))
        .unwrap_or_else(|| SimpleSpan::from(0..0));
    let tokens: Vec<(Token<'_>, SimpleSpan)> = tokens
        .into_iter()
        .map(|(tok, span)| (tok, SimpleSpan::from(span)))
        .collect();
    IterInput::new(tokens.into_iter(), eoi)
}

pub use intern::{Interner, Symbol};

pub type State = SimpleState<Interner>;

pub(crate) type ParserExtra<'src> = extra::Full<Simple<'src, Token<'src>>, State, ()>;

pub(crate) type Extra<'src, 'b> = MapExtra<'src, 'b, ParserInput<'src>, ParserExtra<'src>>;

pub trait RooParser<'src, O>:
    Parser<'src, ParserInput<'src>, O, ParserExtra<'src>> + Clone
{
}

impl<'src, O, T> RooParser<'src, O> for T where
    T: Parser<'src, ParserInput<'src>, O, ParserExtra<'src>> + Clone
{
}

pub(crate) fn span(e: &mut Extra) -> ast::Span {
    let s = e.span();
    ast::Span {
        start: s.start(),
        end: s.end(),
    }
}

pub(crate) fn intern(e: &mut Extra, name: &str) -> Symbol {
    e.state().intern(name)
}

#[cfg(test)]
pub(crate) mod test_util {
    pub(crate) fn tokens(source: &str) -> crate::ParserInput<'_> {
        crate::input(lexer::tokenize_all(source).expect("test input should lex cleanly"))
    }
}
