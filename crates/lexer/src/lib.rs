//! `lexer` turns fig-lang source text into a stream of tokens, using
//! [`logos`] to generate the actual scanning code from the [`Token`]
//! definition.
//!
//! Most tokens are plain fixed strings or simple regexes. A handful of
//! constructs need custom scanning logic instead, because they can't be
//! expressed as a single regular expression: nested block comments,
//! string/char/raw-string literals (which need to track escapes and, for
//! raw strings, a variable number of matching `#` delimiters), and numeric
//! literals (which need one character of lookahead to decide whether a
//! trailing `.` is part of the number or the start of a field access,
//! method call, or range). See `token.rs` for how each of those works.

mod error;
mod token;

pub use error::LexError;
pub use logos::Logos;
pub use token::{NumberKind, Token};

/// Lexes `source` into a stream of tokens.
///
/// Each item is `Ok(token)` for a successfully recognized token, or
/// `Err(LexError)` at the first point the source text couldn't be turned
/// into one (an unterminated string/char/raw-string/block comment, or a
/// character that doesn't start any valid token).
pub fn tokenize(source: &str) -> impl Iterator<Item = Result<Token<'_>, LexError>> {
    Token::lexer(source)
}

/// Lexes `source` and collects every token, returning an error at the
/// first one that fails. Useful for tests and other "just tell me it all
/// lexed cleanly" callers; [`tokenize`] is the streaming version.
pub fn tokenize_all(source: &str) -> Result<Vec<Token<'_>>, LexError> {
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
                Let,
                Identifier("x"),
                Eq,
                Number(NumberKind::Int("5")),
                Semi,
            ])
        );
    }
}
