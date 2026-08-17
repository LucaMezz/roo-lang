use logos::{Lexer, Logos, Skip};

use crate::error::LexError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberKind<'src> {
    Int(&'src str),
    Float(&'src str),
}

#[derive(Logos, Debug, Clone, Copy, PartialEq, Eq)]
#[logos(error = LexError)]
#[logos(skip r"[ \t\r\n\f]+")]
pub enum Token<'src> {
    #[token("as")]
    As,
    #[token("break")]
    Break,
    #[token("continue")]
    Continue,
    #[token("else")]
    Else,
    #[token("enum")]
    Enum,
    #[token("false")]
    False,
    #[token("fn")]
    Fn,
    #[token("for")]
    For,
    #[token("if")]
    If,
    #[token("impl")]
    Impl,
    #[token("in")]
    In,
    #[token("let")]
    Let,
    #[token("loop")]
    Loop,
    #[token("match")]
    Match,
    #[token("mod")]
    Mod,
    #[token("pub")]
    Pub,
    #[token("return")]
    Return,
    #[token("self")]
    SelfLower,
    #[token("Self")]
    SelfUpper,
    #[token("struct")]
    Struct,
    #[token("super")]
    Super,
    #[token("trait")]
    Trait,
    #[token("true")]
    True,
    #[token("type")]
    Type,
    #[token("use")]
    Use,
    #[token("where")]
    Where,
    #[token("while")]
    While,

    #[token("dyn")]
    Dyn,
    #[token("const")]
    Const,

    #[regex(r"[a-zA-Z_][a-zA-Z0-9_]*")]
    Identifier(&'src str),

    #[regex(r"0[xX][0-9a-fA-F_]+", |lex| NumberKind::Int(lex.slice()))]
    #[regex(r"0[oO][0-7_]+", |lex| NumberKind::Int(lex.slice()))]
    #[regex(r"0[bB][01_]+", |lex| NumberKind::Int(lex.slice()))]
    #[regex(r"[0-9][0-9_]*", lex_decimal_number)]
    Number(NumberKind<'src>),

    #[token("'", lex_char)]
    CharLiteral(&'src str),

    #[token("\"", lex_string)]
    StringLiteral(&'src str),

    #[regex("r#*\"", lex_raw_string)]
    RawStringLiteral(&'src str),

    #[regex(r"//![^\n]*", priority = 10, allow_greedy = true)]
    InnerDocComment(&'src str),

    #[regex(r"///[^\n]*", priority = 10, allow_greedy = true)]
    OuterDocComment(&'src str),

    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Star,
    #[token("/")]
    Slash,
    #[token("%")]
    Percent,

    #[token("==")]
    EqEq,
    #[token("!=")]
    NotEq,
    #[token("<")]
    Lt,
    #[token(">")]
    Gt,
    #[token("<=")]
    LtEq,
    #[token(">=")]
    GtEq,

    #[token("&&")]
    AndAnd,
    #[token("||")]
    OrOr,
    #[token("!")]
    Bang,

    #[token("&")]
    Amp,
    #[token("|")]
    Pipe,
    #[token("^")]
    Caret,
    #[token("<<")]
    Shl,
    #[token(">>")]
    Shr,

    #[token("=")]
    Eq,
    #[token("+=")]
    PlusEq,
    #[token("-=")]
    MinusEq,
    #[token("*=")]
    StarEq,
    #[token("/=")]
    SlashEq,
    #[token("%=")]
    PercentEq,
    #[token("&=")]
    AmpEq,
    #[token("|=")]
    PipeEq,
    #[token("^=")]
    CaretEq,
    #[token("<<=")]
    ShlEq,
    #[token(">>=")]
    ShrEq,

    #[token("..")]
    DotDot,
    #[token("..=")]
    DotDotEq,

    #[token(".")]
    Dot,
    #[token("::")]
    PathSep,

    #[token("?")]
    Question,

    #[token("->")]
    Arrow,
    #[token("=>")]
    FatArrow,

    #[token(":")]
    Colon,
    #[token(";")]
    Semi,
    #[token(",")]
    Comma,
    #[token("@")]
    At,
    #[token("#")]
    Pound,

    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,

    #[regex(r"//[^\n]*", logos::skip, allow_greedy = true)]
    #[token("/*", lex_block_comment)]
    Skipped,
}

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
