#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LexError {
    #[default]
    InvalidToken,
    UnterminatedString,
    UnterminatedChar,
    UnterminatedBlockComment,
    UnterminatedRawString,
    InvalidUnicodeEscape,
}
