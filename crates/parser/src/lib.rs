//! `parser` turns the token stream produced by `lexer` into an AST.
//!
//! Built as a chumsky parser over `lexer`'s `Token` type — [`Token`] slices
//! are a valid chumsky [`Input`] out of the box, and [`select!`]
//! deconstructs tokens into AST nodes without needing a separate
//! `just(Token::Foo)` for every case.
//!
//! Split into one module per grammar concern (`literal`, `pat`, `expr`,
//! ...), listed alphabetically below per rustfmt. The dependency graph
//! between them isn't a strict layering — most modules call into `ty`,
//! `path`, and `ident` at minimum — so every module glob-imports the
//! whole crate (`use crate::*;`) rather than naming specific siblings.
//! Top-level items (`fn`/`struct`/`impl`/...) aren't implemented yet.

use chumsky::input::MapExtra;
use chumsky::prelude::*;
use lexer::Token;

mod annotation;
mod expr;
mod ident;
mod literal;
mod operator;
mod pat;
mod path;
mod stmt;
mod ty;
mod visibility;

pub use annotation::*;
pub use expr::*;
pub use ident::*;
pub use literal::*;
pub use operator::*;
pub use pat::*;
pub use path::*;
pub use stmt::*;
pub use ty::*;
pub use visibility::*;

/// The concrete `MapExtra` shape every `select!`/`.map_with` site in this
/// parser sees — pinning it down as a real function signature (rather
/// than relying on inference inside a `select!` closure body) is what
/// makes `span(e)` just work at each call site.
pub(crate) type Extra<'src, 'b> =
    MapExtra<'src, 'b, &'src [Token<'src>], extra::Err<Simple<'src, Token<'src>>>>;

/// Stands in for a parser function's return type — every fig parser shares
/// the same input (`&[Token]`) and error type, and only the AST node it
/// produces (`O`) actually varies. `type FigParser<'src, O> = impl
/// Parser<...>;` would say this more directly, but `impl Trait` in type
/// aliases is unstable (`type_alias_impl_trait`) — this blanket-impl'd
/// marker trait is the standard stable-Rust substitute, the same way a
/// marker trait stands in for the also-unstable `trait_alias` feature.
pub trait FigParser<'src, O>:
    Parser<'src, &'src [Token<'src>], O, extra::Err<Simple<'src, Token<'src>>>> + Clone
{
}

impl<'src, O, T> FigParser<'src, O> for T where
    T: Parser<'src, &'src [Token<'src>], O, extra::Err<Simple<'src, Token<'src>>>> + Clone
{
}

/// Converts chumsky's span for the current match into an [`ast::Span`].
pub(crate) fn span(e: &mut Extra) -> ast::Span {
    let s = e.span();
    ast::Span {
        start: s.start(),
        end: s.end(),
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    use lexer::Token;

    pub(crate) fn tokens(source: &str) -> Vec<Token<'_>> {
        lexer::tokenize_all(source).expect("test input should lex cleanly")
    }
}
