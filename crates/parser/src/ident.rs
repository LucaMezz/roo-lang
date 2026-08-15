//! Bare identifiers and loop labels.

use crate::*;
use ast::*;
use lexer::Token;

/// An ordinary identifier — deliberately *not* `_`, which is never a real
/// name (you can't `fn _() {}` any more than you can in Rust) and has its
/// own dedicated AST nodes instead (`PatKind::Wild`, `ExprKind::Underscore`).
/// Excluding it here means a future pattern/expression parser can't
/// accidentally fall through to treating `_` as an ordinary binding just
/// by forgetting to special-case it — the `_` case has to be handled
/// deliberately, since this parser will never produce it.
pub fn ident<'src>() -> impl FigParser<'src, Ident> {
    select! {
        Token::Identifier(name) = e if name != "_" => Ident { name: name.to_owned(), span: span(e) },
    }
}

/// `outer:` — a loop label. Plain-identifier labels, not Rust's `'outer`
/// lifetime-sigil form (decisions/0002) — so this is just `ident()`
/// followed by the `:` the concrete syntax requires, with the colon
/// itself discarded (`Label` has nothing to store it in).
pub fn label<'src>() -> impl FigParser<'src, Label> {
    ident()
        .then_ignore(just(Token::Colon))
        .map(|ident| Label { ident })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::tokens;

    #[test]
    fn parses_an_ident_with_its_span() {
        let tokens = tokens("foo");
        let parsed = ident().parse(tokens).into_result().expect("should parse");
        assert_eq!(parsed.name, "foo");
        assert_eq!(parsed.span, ast::Span { start: 0, end: 3 });
    }

    #[test]
    fn rejects_underscore_as_an_ordinary_identifier() {
        let tokens = tokens("_");
        assert!(ident().parse(tokens).into_result().is_err());
    }

    #[test]
    fn parses_a_label() {
        let tokens = tokens("outer:");
        let parsed = label().parse(tokens).into_result().expect("should parse");
        assert_eq!(parsed.ident.name, "outer");
    }

    #[test]
    fn rejects_an_identifier_with_no_colon() {
        let tokens = tokens("outer");
        assert!(label().parse(tokens).into_result().is_err());
    }

    #[test]
    fn rejects_underscore_as_a_label() {
        let tokens = tokens("_:");
        assert!(label().parse(tokens).into_result().is_err());
    }
}
