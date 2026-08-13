//! `pub`/`pub(...)` visibility parsing.

use crate::*;
use ast::*;
use lexer::Token;

fn visibility_kind<'src>() -> impl FigParser<'src, VisibilityKind> {
    choice((
        just(Token::Pub).map(|_| VisibilityKind::Public),
        just(Token::Pub)
            .ignore_then(
                path(ty())
                    .delimited_by(just(Token::LParen), just(Token::RParen))
                    .map(Box::new),
            )
            .map(|path| VisibilityKind::Restricted { path }),
        empty().map(|_| VisibilityKind::Inherited),
    ))
}

pub fn visibility<'src>() -> impl FigParser<'src, Visibility> {
    visibility_kind().map_with(|kind, e| Visibility {
        kind,
        span: span(e),
    })
}
