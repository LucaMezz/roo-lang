//! Literal parsing (`Lit`/`LitKind`) — char/string escape processing and
//! integer/float literal text handling live here too.

use crate::*;
use ast::*;
use lexer::{NumberKind, Token};

fn process_escape<'src>(chars: &mut impl Iterator<Item = char>) -> char {
    chars.next();
    match chars.next() {
        Some('n') => '\n',
        Some('r') => '\r',
        Some('t') => '\t',
        Some('\\') => '\\',
        Some('0') => '\0',
        Some('"') => '"',
        Some('\'') => '\'',
        Some('u') => {
            // Consume the `{`.
            chars.next();
            let mut hex = String::new();
            for c in chars.by_ref() {
                if c == '}' {
                    break;
                }
                hex.push(c);
            }
            match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                Some(c) => c,
                None => todo!(), // invalid scalar value (out of range or a surrogate)
            }
        }
        Some(_c) => {
            /* unknown escape */
            todo!()
        }
        None => {
            /* unterminated */
            todo!()
        }
    }
}

fn process_char<'src>(text: &'src str) -> char {
    let mut chars = text.chars().skip(1).peekable();

    if let Some('\\') = chars.peek() {
        process_escape(&mut chars)
    } else {
        chars.next().unwrap()
    }
}

fn process_string<'src>(text: &'src str) -> String {
    let mut chars = text.chars().skip(1).peekable();
    let mut result = String::new();

    while let Some(&c) = chars.peek() {
        if c == '"' {
            // The closing quote — the lexer guarantees this is unescaped
            // and terminal, so stop without consuming it into the output.
            break;
        }
        if c == '\\' {
            result.push(process_escape(&mut chars));
        } else {
            result.push(c);
            chars.next();
        }
    }

    result
}

/// Strips a raw string literal's `r`/`#`*/`"` delimiters. No escape
/// processing — that's the entire point of a raw string (`r"C:\Users"`,
/// the `\U` is two literal characters, not an escape).
fn process_raw_string<'src>(text: &'src str) -> String {
    let hash_count = text[1..].bytes().take_while(|&b| b == b'#').count();
    let start = 1 + hash_count + 1; // 'r', the opening '#'s, the opening '"'
    let end = text.len() - hash_count - 1; // the closing '"', the closing '#'s
    text[start..end].to_owned()
}

/// Parses an integer literal's raw text (decimal, or `0x`/`0o`/`0b`
/// prefixed, digit separators and all — see [`NumberKind`]) into its value.
fn process_int<'src>(text: &'src str) -> u128 {
    let (digits, radix) = match text.get(0..2) {
        Some("0x" | "0X") => (&text[2..], 16),
        Some("0o" | "0O") => (&text[2..], 8),
        Some("0b" | "0B") => (&text[2..], 2),
        _ => (text, 10),
    };

    let digits: String = digits.chars().filter(|&c| c != '_').collect();

    match u128::from_str_radix(&digits, radix) {
        Ok(n) => n,
        Err(_) => todo!(), // literal's value overflows u128
    }
}

fn process_number<'src>(lit: NumberKind<'src>) -> LitKind {
    match lit {
        NumberKind::Int(text) => LitKind::Int(process_int(text)),
        // Kept raw, same as `LitKind::Float` itself — see decisions and
        // `NumberKind`'s doc comment: an actual `f64`/`f32` doesn't have
        // well-behaved `Eq`/`Hash` (NaN, -0.0 vs 0.0), so the value is
        // left as text rather than parsed here.
        NumberKind::Float(text) => LitKind::Float(text.to_owned()),
    }
}

pub fn literal<'src>() -> impl FigParser<'src, Lit> {
    select! {
        Token::CharLiteral(lit) = e => Lit { kind: LitKind::Char(process_char(lit)), span: span(e) },
        Token::StringLiteral(lit) = e => Lit { kind: LitKind::Str(process_string(lit)), span: span(e) },
        Token::RawStringLiteral(lit) = e => Lit { kind: LitKind::Str(process_raw_string(lit)), span: span(e) },
        Token::Number(lit) = e => Lit { kind: process_number(lit), span: span(e) },
        Token::True = e => Lit { kind: LitKind::Bool(true), span: span(e) },
        Token::False = e => Lit { kind: LitKind::Bool(false), span: span(e) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::tokens;

    #[test]
    fn parses_a_plain_char_literal() {
        let tokens = tokens("'a'");
        let parsed = literal().parse(tokens).into_result().expect("should parse");
        assert_eq!(parsed.kind, LitKind::Char('a'));
        assert_eq!(parsed.span, ast::Span { start: 0, end: 3 });
    }

    #[test]
    fn parses_a_char_literal_with_a_simple_escape() {
        let tokens = tokens(r"'\n'");
        let parsed = literal().parse(tokens).into_result().expect("should parse");
        assert_eq!(parsed.kind, LitKind::Char('\n'));
    }

    #[test]
    fn parses_a_char_literal_with_a_unicode_escape() {
        let tokens = tokens(r"'\u{1F980}'");
        let parsed = literal().parse(tokens).into_result().expect("should parse");
        assert_eq!(parsed.kind, LitKind::Char('🦀'));
    }

    #[test]
    fn parses_a_plain_string_literal() {
        let tokens = tokens(r#""hello, world""#);
        let parsed = literal().parse(tokens).into_result().expect("should parse");
        assert_eq!(parsed.kind, LitKind::Str("hello, world".to_owned()));
        assert_eq!(parsed.span, ast::Span { start: 0, end: 14 });
    }

    #[test]
    fn parses_a_string_literal_with_escapes() {
        let tokens = tokens(r#""a\nb\tc""#);
        let parsed = literal().parse(tokens).into_result().expect("should parse");
        assert_eq!(parsed.kind, LitKind::Str("a\nb\tc".to_owned()));
    }

    #[test]
    fn parses_a_raw_string_literal_without_processing_escapes() {
        let tokens = tokens(r#"r"C:\Users\name""#);
        let parsed = literal().parse(tokens).into_result().expect("should parse");
        assert_eq!(parsed.kind, LitKind::Str(r"C:\Users\name".to_owned()));
    }

    #[test]
    fn parses_a_hashed_raw_string_literal_containing_a_quote() {
        let tokens = tokens(r###"r#"she said "hi""#"###);
        let parsed = literal().parse(tokens).into_result().expect("should parse");
        assert_eq!(parsed.kind, LitKind::Str(r#"she said "hi""#.to_owned()));
    }

    #[test]
    fn parses_int_literals_across_bases_and_separators() {
        let cases = [
            ("42", 42u128),
            ("1_000", 1000),
            ("0xFF", 255),
            ("0x_FF", 255),
            ("0o17", 15),
            ("0b101", 5),
        ];
        for (src, expected) in cases {
            let tokens = tokens(src);
            let parsed = literal().parse(tokens).into_result().expect("should parse");
            assert_eq!(parsed.kind, LitKind::Int(expected), "input: {src}");
        }
    }

    #[test]
    fn parses_a_float_literal_as_raw_text() {
        let tokens = tokens("3.14");
        let parsed = literal().parse(tokens).into_result().expect("should parse");
        assert_eq!(parsed.kind, LitKind::Float("3.14".to_owned()));
    }

    #[test]
    fn parses_the_true_literal() {
        let tokens = tokens("true");
        let parsed = literal().parse(tokens).into_result().expect("should parse");
        assert_eq!(parsed.kind, LitKind::Bool(true));
    }

    #[test]
    fn parses_the_false_literal() {
        let tokens = tokens("false");
        let parsed = literal().parse(tokens).into_result().expect("should parse");
        assert_eq!(parsed.kind, LitKind::Bool(false));
    }
}
