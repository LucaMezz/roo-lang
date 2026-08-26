use crate::*;
use ast::*;
use lexer::Token;

pub fn ident<'src>() -> impl RooParser<'src, Ident> {
    select! {
        Token::Identifier(name) = e if name != "_" => Ident { symbol: intern(e, name), span: span(e) },
    }
}

pub fn label<'src>() -> impl RooParser<'src, Label> {
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
        let mut state = crate::State::default();
        let parsed = ident()
            .parse_with_state(tokens, &mut state)
            .into_result()
            .expect("should parse");
        assert_eq!(state.resolve(parsed.symbol), "foo");
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
        let mut state = crate::State::default();
        let parsed = label()
            .parse_with_state(tokens, &mut state)
            .into_result()
            .expect("should parse");
        assert_eq!(state.resolve(parsed.ident.symbol), "outer");
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

    #[test]
    fn interns_identifier_names_through_parser_state() {
        let tokens = tokens("foo foo bar");
        let mut state = crate::State::default();
        let ((first, second), third) = ident()
            .then(ident())
            .then(ident())
            .parse_with_state(tokens, &mut state)
            .into_result()
            .expect("should parse");

        assert_eq!(first.symbol, second.symbol);
        assert_ne!(first.symbol, third.symbol);
        assert_eq!(state.resolve(first.symbol), "foo");
        assert_eq!(state.resolve(third.symbol), "bar");
    }
}
