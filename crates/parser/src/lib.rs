//! `parser` turns the token stream produced by `lexer` into an AST.
//!
//! Built as a chumsky parser over `lexer`'s `Token` type. `lexer` pairs
//! each token with the byte range in the source it came from, but a
//! bare `&[(Token, Range<usize>)]` isn't directly usable as a chumsky
//! [`Input`] with the right `Span`/`Token` types — `select!`/`just`
//! need to match on `Token` alone, not `(Token, Range)` pairs. [`input`]
//! bridges that gap, via chumsky's [`IterInput`](chumsky::input::IterInput),
//! so spans produced anywhere in this parser (via [`span`]) are real
//! source byte offsets, not token-stream positions.
//!
//! Split into one module per grammar concern (`literal`, `pat`, `expr`,
//! ...), listed alphabetically below per rustfmt. The dependency graph
//! between them isn't a strict layering — most modules call into `ty`,
//! `path`, and `ident` at minimum — so every module glob-imports the
//! whole crate (`use crate::*;`) rather than naming specific siblings.
//! `module()` (in `module`) is the top-level entry point for parsing a
//! whole file.

use std::ops::Range;
use std::vec;

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

/// The concrete input type every parser in this crate parses over: a
/// token stream that carries real source byte spans alongside each
/// token, rather than token-stream positions.
pub type ParserInput<'src> = IterInput<vec::IntoIter<(Token<'src>, SimpleSpan)>, SimpleSpan>;

/// Wraps token+span pairs from [`lexer::tokenize_all`] into the
/// [`ParserInput`] chumsky actually parses over. This is how every
/// entry point into this crate (`module()`, `expr()`, ...) should be
/// fed its input.
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

/// The concrete `MapExtra` shape every `select!`/`.map_with` site in this
/// parser sees — pinning it down as a real function signature (rather
/// than relying on inference inside a `select!` closure body) is what
/// makes `span(e)` just work at each call site.
pub(crate) type Extra<'src, 'b> =
    MapExtra<'src, 'b, ParserInput<'src>, extra::Err<Simple<'src, Token<'src>>>>;

/// Stands in for a parser function's return type — every roo parser shares
/// the same input ([`ParserInput`]) and error type, and only the AST node
/// it produces (`O`) actually varies. `type RooParser<'src, O> = impl
/// Parser<...>;` would say this more directly, but `impl Trait` in type
/// aliases is unstable (`type_alias_impl_trait`) — this blanket-impl'd
/// marker trait is the standard stable-Rust substitute, the same way a
/// marker trait stands in for the also-unstable `trait_alias` feature.
pub trait RooParser<'src, O>:
    Parser<'src, ParserInput<'src>, O, extra::Err<Simple<'src, Token<'src>>>> + Clone
{
}

impl<'src, O, T> RooParser<'src, O> for T where
    T: Parser<'src, ParserInput<'src>, O, extra::Err<Simple<'src, Token<'src>>>> + Clone
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
    pub(crate) fn tokens(source: &str) -> crate::ParserInput<'_> {
        crate::input(lexer::tokenize_all(source).expect("test input should lex cleanly"))
    }
}
