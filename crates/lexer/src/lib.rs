mod error;
mod token;

use std::ops::Range;

pub use error::LexError;
pub use logos::Logos;
pub use token::{NumberKind, Token};

pub fn tokenize(source: &str) -> impl Iterator<Item = Result<(Token<'_>, Range<usize>), LexError>> {
    Token::lexer(source)
        .spanned()
        .map(|(tok, span)| tok.map(|tok| (tok, span)))
}

pub fn tokenize_all(source: &str) -> Result<Vec<(Token<'_>, Range<usize>)>, LexError> {
    tokenize(source).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_lexes_to_nothing() {
        assert_eq!(tokenize_all(""), Ok(vec![]));
    }

    #[test]
    fn whitespace_only_lexes_to_nothing() {
        assert_eq!(tokenize_all("   \t\n\r\n  "), Ok(vec![]));
    }

    #[test]
    fn a_trivial_let_binding_lexes() {
        use Token::*;
        assert_eq!(
            tokenize_all("let x = 5;"),
            Ok(vec![
                (Let, 0..3),
                (Identifier("x"), 4..5),
                (Eq, 6..7),
                (Number(NumberKind::Int("5")), 8..9),
                (Semi, 9..10),
            ])
        );
    }
}
