#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LexError {
    #[default]
    InvalidToken,
    UnterminatedString,
    UnterminatedBlockComment,
    UnterminatedRawString,
    InvalidUnicodeEscape,
}
