//! The [`Token`] type and the callbacks that lex the handful of constructs
//! too irregular for a plain regex: nested block comments, string/char/raw
//! string literals, and numeric literals (which need lookahead to decide
//! where a trailing `.` stops belonging to the number).

use logos::{Lexer, Logos, Skip};

use crate::error::LexError;

/// A decimal, hex, octal, or binary numeric literal's raw source text,
/// tagged with whether it's an integer or a float.
///
/// The lexer keeps the original text (digit separators, base prefix, and
/// all) rather than parsing it to a number — turning `"1_000"` into a
/// concrete integer type is a later stage's job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberKind<'src> {
    /// An integer literal: decimal, `0x`/`0X` hex, `0o`/`0O` octal, or
    /// `0b`/`0B` binary.
    Int(&'src str),
    /// A float literal: has a fractional part, an exponent, or both.
    Float(&'src str),
}

/// A single fig token, borrowing its text directly from the source it was
/// lexed from.
#[derive(Logos, Debug, Clone, Copy, PartialEq, Eq)]
#[logos(error = LexError)]
#[logos(skip r"[ \t\r\n\f]+")]
pub enum Token<'src> {
    // ---------------------------------------------------------------
    // Strict keywords
    // ---------------------------------------------------------------
    /// `as`
    #[token("as")]
    As,
    /// `break`
    #[token("break")]
    Break,
    /// `continue`
    #[token("continue")]
    Continue,
    /// `else`
    #[token("else")]
    Else,
    /// `enum`
    #[token("enum")]
    Enum,
    /// `false`
    #[token("false")]
    False,
    /// `fn`
    #[token("fn")]
    Fn,
    /// `for`
    #[token("for")]
    For,
    /// `if`
    #[token("if")]
    If,
    /// `impl`
    #[token("impl")]
    Impl,
    /// `in`
    #[token("in")]
    In,
    /// `let`
    #[token("let")]
    Let,
    /// `loop`
    #[token("loop")]
    Loop,
    /// `match`
    #[token("match")]
    Match,
    /// `mod`
    #[token("mod")]
    Mod,
    /// `mut`
    #[token("mut")]
    Mut,
    /// `pub`
    #[token("pub")]
    Pub,
    /// `return`
    #[token("return")]
    Return,
    /// `self`
    #[token("self")]
    SelfLower,
    /// `Self`
    #[token("Self")]
    SelfUpper,
    /// `struct`
    #[token("struct")]
    Struct,
    /// `super`
    #[token("super")]
    Super,
    /// `trait`
    #[token("trait")]
    Trait,
    /// `true`
    #[token("true")]
    True,
    /// `type`
    #[token("type")]
    Type,
    /// `use`
    #[token("use")]
    Use,
    /// `where`
    #[token("where")]
    Where,
    /// `while`
    #[token("while")]
    While,

    // ---------------------------------------------------------------
    // Reserved, currently unused (see book/src/appendix/keywords.md)
    // ---------------------------------------------------------------
    /// `dyn`
    #[token("dyn")]
    Dyn,
    /// `const`
    #[token("const")]
    Const,

    // ---------------------------------------------------------------
    // Identifiers
    // ---------------------------------------------------------------
    /// Any identifier, including the wildcard `_` — whether `_` is "the
    /// wildcard pattern" or an ordinary name is a parser-level distinction,
    /// not a lexical one (mirroring how rustc's own lexer treats it).
    #[regex(r"[a-zA-Z_][a-zA-Z0-9_]*")]
    Identifier(&'src str),

    // ---------------------------------------------------------------
    // Literals
    // ---------------------------------------------------------------
    /// An integer or float literal. See [`NumberKind`].
    #[regex(r"0[xX][0-9a-fA-F_]+", |lex| NumberKind::Int(lex.slice()))]
    #[regex(r"0[oO][0-7_]+", |lex| NumberKind::Int(lex.slice()))]
    #[regex(r"0[bB][01_]+", |lex| NumberKind::Int(lex.slice()))]
    #[regex(r"[0-9][0-9_]*", lex_decimal_number)]
    Number(NumberKind<'src>),

    /// A `'...'` char literal, source text including the quotes.
    #[token("'", lex_char)]
    CharLiteral(&'src str),

    /// A `"..."` string literal, source text including the quotes.
    #[token("\"", lex_string)]
    StringLiteral(&'src str),

    /// An `r"..."`/`r#"..."#`/... raw string literal, source text
    /// including the `r`, every `#`, and both quotes.
    #[regex("r#*\"", lex_raw_string)]
    RawStringLiteral(&'src str),

    /// A `//!` inner doc comment, source text including the `//!` and
    /// everything up to (not including) the newline.
    #[regex(r"//![^\n]*", priority = 10, allow_greedy = true)]
    InnerDocComment(&'src str),

    /// A `///` outer doc comment, source text including the `///` and
    /// everything up to (not including) the newline.
    #[regex(r"///[^\n]*", priority = 10, allow_greedy = true)]
    OuterDocComment(&'src str),

    // ---------------------------------------------------------------
    // Operators and punctuation
    // ---------------------------------------------------------------
    /// `+`
    #[token("+")]
    Plus,
    /// `-`
    #[token("-")]
    Minus,
    /// `*`
    #[token("*")]
    Star,
    /// `/`
    #[token("/")]
    Slash,
    /// `%`
    #[token("%")]
    Percent,

    /// `==`
    #[token("==")]
    EqEq,
    /// `!=`
    #[token("!=")]
    NotEq,
    /// `<`
    #[token("<")]
    Lt,
    /// `>`
    #[token(">")]
    Gt,
    /// `<=`
    #[token("<=")]
    LtEq,
    /// `>=`
    #[token(">=")]
    GtEq,

    /// `&&`
    #[token("&&")]
    AndAnd,
    /// `||`
    #[token("||")]
    OrOr,
    /// `!`
    #[token("!")]
    Bang,

    /// `&`
    #[token("&")]
    Amp,
    /// `|`
    #[token("|")]
    Pipe,
    /// `^`
    #[token("^")]
    Caret,
    /// `<<`
    #[token("<<")]
    Shl,
    /// `>>`
    #[token(">>")]
    Shr,

    /// `=`
    #[token("=")]
    Eq,
    /// `+=`
    #[token("+=")]
    PlusEq,
    /// `-=`
    #[token("-=")]
    MinusEq,
    /// `*=`
    #[token("*=")]
    StarEq,
    /// `/=`
    #[token("/=")]
    SlashEq,
    /// `%=`
    #[token("%=")]
    PercentEq,
    /// `&=`
    #[token("&=")]
    AmpEq,
    /// `|=`
    #[token("|=")]
    PipeEq,
    /// `^=`
    #[token("^=")]
    CaretEq,
    /// `<<=`
    #[token("<<=")]
    ShlEq,
    /// `>>=`
    #[token(">>=")]
    ShrEq,

    /// `..`
    #[token("..")]
    DotDot,
    /// `..=`
    #[token("..=")]
    DotDotEq,

    /// `.`
    #[token(".")]
    Dot,
    /// `::`
    #[token("::")]
    PathSep,

    /// `?`
    #[token("?")]
    Question,

    /// `->`
    #[token("->")]
    Arrow,
    /// `=>`
    #[token("=>")]
    FatArrow,

    /// `:`
    #[token(":")]
    Colon,
    /// `;`
    #[token(";")]
    Semi,
    /// `,`
    #[token(",")]
    Comma,
    /// `@`
    #[token("@")]
    At,

    /// `(`
    #[token("(")]
    LParen,
    /// `)`
    #[token(")")]
    RParen,
    /// `{`
    #[token("{")]
    LBrace,
    /// `}`
    #[token("}")]
    RBrace,
    /// `[`
    #[token("[")]
    LBracket,
    /// `]`
    #[token("]")]
    RBracket,

    // ---------------------------------------------------------------
    // Trivia — never emitted as a real token, but an unterminated block
    // comment is still a genuine lex error, not silently ignored.
    // ---------------------------------------------------------------
    /// A `//` line comment (not `///`/`//!`), or a `/* ... */` block
    /// comment (which may nest). Both are skipped entirely.
    #[regex(r"//[^\n]*", logos::skip, allow_greedy = true)]
    #[token("/*", lex_block_comment)]
    Skipped,
}

/// Lexes a run of decimal digits into either an int or a float, handling
/// the ambiguity a trailing `.` creates: `5.` is a float, but the `.` in
/// `x.5` (field/method access) or `0..5` (a range) is not part of the
/// number at all. See `decisions/` for why this needs a callback instead
/// of a plain regex.
fn lex_decimal_number<'src>(lex: &mut Lexer<'src, Token<'src>>) -> NumberKind<'src> {
    let remainder = lex.remainder();
    let bytes = remainder.as_bytes();
    let mut consumed = 0usize;
    let mut is_float = false;

    if bytes.first() == Some(&b'.') {
        let next = bytes.get(1).copied();
        let dot_belongs_to_number = !matches!(
            next,
            Some(b'.') | Some(b'_') | Some(b'a'..=b'z') | Some(b'A'..=b'Z')
        );
        if dot_belongs_to_number {
            is_float = true;
            consumed += 1;
            while matches!(bytes.get(consumed), Some(b'0'..=b'9') | Some(b'_')) {
                consumed += 1;
            }
        }
    }

    if let Some(&e) = bytes.get(consumed) {
        if e == b'e' || e == b'E' {
            let mut peek = consumed + 1;
            if matches!(bytes.get(peek), Some(b'+') | Some(b'-')) {
                peek += 1;
            }
            if matches!(bytes.get(peek), Some(b'0'..=b'9')) {
                is_float = true;
                let mut p = peek;
                while matches!(bytes.get(p), Some(b'0'..=b'9') | Some(b'_')) {
                    p += 1;
                }
                consumed = p;
            }
        }
    }

    lex.bump(consumed);
    if is_float {
        NumberKind::Float(lex.slice())
    } else {
        NumberKind::Int(lex.slice())
    }
}

/// Length in bytes of the UTF-8 encoding of the scalar value starting with
/// lead byte `b`.
fn utf8_len(b: u8) -> usize {
    if b & 0b1000_0000 == 0 {
        1
    } else if b & 0b1110_0000 == 0b1100_0000 {
        2
    } else if b & 0b1111_0000 == 0b1110_0000 {
        3
    } else if b & 0b1111_1000 == 0b1111_0000 {
        4
    } else {
        1
    }
}

/// Consumes one char-literal "body" (either an escape sequence or a single
/// Unicode scalar value) from `bytes` starting at `i`, returning the index
/// just past it, or `None` if the body is malformed/truncated.
fn consume_char_body(bytes: &[u8], mut i: usize) -> Option<usize> {
    match bytes.get(i)? {
        b'\\' => {
            i += 1;
            match *bytes.get(i)? {
                b'u' => {
                    i += 1;
                    if bytes.get(i) != Some(&b'{') {
                        return None;
                    }
                    i += 1;
                    let hex_start = i;
                    while bytes.get(i).is_some_and(u8::is_ascii_hexdigit) {
                        i += 1;
                    }
                    if i == hex_start || bytes.get(i) != Some(&b'}') {
                        return None;
                    }
                    i += 1;
                }
                _ => i += 1,
            }
        }
        &b => i += utf8_len(b),
    }
    Some(i)
}

/// Lexes a `'...'` char literal. The opening `'` has already been consumed
/// as the triggering token.
fn lex_char<'src>(lex: &mut Lexer<'src, Token<'src>>) -> Result<&'src str, LexError> {
    let remainder = lex.remainder();
    let bytes = remainder.as_bytes();

    let after_body = consume_char_body(bytes, 0).ok_or(LexError::UnterminatedChar)?;
    if bytes.get(after_body) != Some(&b'\'') {
        return Err(LexError::UnterminatedChar);
    }

    lex.bump(after_body + 1);
    Ok(lex.slice())
}

/// Lexes a `"..."` string literal. The opening `"` has already been
/// consumed as the triggering token.
fn lex_string<'src>(lex: &mut Lexer<'src, Token<'src>>) -> Result<&'src str, LexError> {
    let remainder = lex.remainder();
    let bytes = remainder.as_bytes();
    let mut i = 0;

    loop {
        match bytes.get(i) {
            None => return Err(LexError::UnterminatedString),
            Some(b'"') => {
                i += 1;
                break;
            }
            Some(b'\\') => i += 2,
            Some(_) => i += 1,
        }
    }

    lex.bump(i);
    Ok(lex.slice())
}

/// Lexes an `r"..."`/`r#"..."#`/... raw string literal. The `r`, every
/// opening `#`, and the opening `"` have already been consumed as the
/// triggering token — `lex.slice()` at entry is exactly that prefix.
fn lex_raw_string<'src>(lex: &mut Lexer<'src, Token<'src>>) -> Result<&'src str, LexError> {
    let hash_count = lex.slice().bytes().filter(|&b| b == b'#').count();
    let remainder = lex.remainder();
    let bytes = remainder.as_bytes();
    let mut i = 0;

    let end = loop {
        match bytes.get(i) {
            None => return Err(LexError::UnterminatedRawString),
            Some(b'"') => {
                let mut j = i + 1;
                let mut matched = 0;
                while matched < hash_count && bytes.get(j) == Some(&b'#') {
                    j += 1;
                    matched += 1;
                }
                if matched == hash_count {
                    break j;
                }
                i += 1;
            }
            Some(_) => i += 1,
        }
    };

    lex.bump(end);
    Ok(lex.slice())
}

/// Lexes a `/* ... */` block comment, which may nest. The opening `/*` has
/// already been consumed as the triggering token.
fn lex_block_comment<'src>(lex: &mut Lexer<'src, Token<'src>>) -> Result<Skip, LexError> {
    let remainder = lex.remainder();
    let bytes = remainder.as_bytes();
    let mut depth = 1usize;
    let mut i = 0;

    while depth > 0 {
        match (bytes.get(i), bytes.get(i + 1)) {
            (Some(b'/'), Some(b'*')) => {
                depth += 1;
                i += 2;
            }
            (Some(b'*'), Some(b'/')) => {
                depth -= 1;
                i += 2;
            }
            (Some(_), _) => i += 1,
            (None, _) => return Err(LexError::UnterminatedBlockComment),
        }
    }

    lex.bump(i);
    Ok(Skip)
}
