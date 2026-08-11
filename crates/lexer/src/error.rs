//! Errors the lexer can produce.

/// A lexical error: something in the source text couldn't be turned into a
/// token at all (as opposed to a later parse error over an otherwise valid
/// token stream).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LexError {
    /// The default error logos reports when nothing matched at all.
    #[default]
    InvalidToken,
    /// A `"..."` string literal with no closing quote before the end of
    /// input.
    UnterminatedString,
    /// A `'...'` char literal with no closing quote before the end of
    /// input, or with content that isn't exactly one character/escape.
    UnterminatedChar,
    /// A `/* ... */` block comment with no matching close before the end
    /// of input.
    UnterminatedBlockComment,
    /// An `r"..."`/`r#"..."#` raw string with no matching close before the
    /// end of input.
    UnterminatedRawString,
    /// A `\u{...}` escape that isn't a well-formed hex-digit sequence in
    /// braces.
    InvalidUnicodeEscape,
}
