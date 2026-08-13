//! `parser` turns the token stream produced by `lexer` into an AST.
//!
//! Just a starting point: a parser for the smallest possible expression, a
//! single literal, to show the shape of a chumsky parser over `lexer`'s
//! `Token` type — [`Token`] slices are a valid chumsky [`Input`] out of the
//! box, and [`select!`] deconstructs tokens into AST nodes without needing
//! a separate `just(Token::Foo)` for every case. Everything past this
//! (operators, precedence, statements, items) is still to be built.

use ast::*;
use chumsky::input::MapExtra;
use chumsky::prelude::*;
use lexer::{NumberKind, Token};

/// The concrete `MapExtra` shape every `select!`/`.map_with` site in this
/// parser sees — pinning it down as a real function signature (rather
/// than relying on inference inside a `select!` closure body) is what
/// makes `span(e)` just work at each call site.
type Extra<'src, 'b> =
    MapExtra<'src, 'b, &'src [Token<'src>], extra::Err<Simple<'src, Token<'src>>>>;

/// Stands in for a parser function's return type — every fig parser shares
/// the same input (`&[Token]`) and error type, and only the AST node it
/// produces (`O`) actually varies. `type FigParser<'src, O> = impl
/// Parser<...>;` would say this more directly, but `impl Trait` in type
/// aliases is unstable (`type_alias_impl_trait`) — this blanket-impl'd
/// marker trait is the standard stable-Rust substitute, the same way a
/// marker trait stands in for the also-unstable `trait_alias` feature.
pub trait FigParser<'src, O>:
    Parser<'src, &'src [Token<'src>], O, extra::Err<Simple<'src, Token<'src>>>> + Clone
{
}

impl<'src, O, T> FigParser<'src, O> for T where
    T: Parser<'src, &'src [Token<'src>], O, extra::Err<Simple<'src, Token<'src>>>> + Clone
{
}

/// Converts chumsky's span for the current match into an [`ast::Span`].
fn span(e: &mut Extra) -> ast::Span {
    let s = e.span();
    ast::Span {
        start: s.start(),
        end: s.end(),
    }
}

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
    }
}

/// A binary operator — `+ - * / % && || ^ & | << >> == != < > <= >=`.
/// Doesn't include `=`/compound-assign operators; those are
/// [`assign_op`]'s job, since `ExprKind::Assign`/`AssignOp` are separate
/// from `ExprKind::Binary`/`BinOp` in the AST.
pub fn bin_op<'src>() -> impl FigParser<'src, BinOp> {
    select! {
        Token::Plus = e => BinOpKind::Add,
        Token::Minus = e => BinOpKind::Sub,
        Token::Star = e => BinOpKind::Mul,
        Token::Slash = e => BinOpKind::Div,
        Token::Percent = e => BinOpKind::Rem,
        Token::AndAnd = e => BinOpKind::And,
        Token::OrOr = e => BinOpKind::Or,
        Token::Caret = e => BinOpKind::BitXor,
        Token::Amp = e => BinOpKind::BitAnd,
        Token::Pipe = e => BinOpKind::BitOr,
        Token::Shl = e => BinOpKind::Shl,
        Token::Shr = e => BinOpKind::Shr,
        Token::EqEq = e => BinOpKind::Eq,
        Token::Lt = e => BinOpKind::Lt,
        Token::LtEq = e => BinOpKind::Le,
        Token::NotEq = e => BinOpKind::Ne,
        Token::GtEq = e => BinOpKind::Ge,
        Token::Gt = e => BinOpKind::Gt,
    }
    .map_with(|kind, e| BinOp {
        kind,
        span: span(e),
    })
}

/// A compound-assignment operator — `+= -= *= /= %= &= |= ^= <<= >>=`.
/// Plain `=` isn't here — that's `ExprKind::Assign`, not `AssignOp`; there
/// is no `AssignOpKind::Assign` variant to produce.
pub fn assign_op<'src>() -> impl FigParser<'src, AssignOp> {
    select! {
        Token::PlusEq = e => AssignOpKind::AddAssign,
        Token::MinusEq = e => AssignOpKind::SubAssign,
        Token::StarEq = e => AssignOpKind::MulAssign,
        Token::SlashEq = e => AssignOpKind::DivAssign,
        Token::PercentEq = e => AssignOpKind::RemAssign,
        Token::AmpEq = e => AssignOpKind::BitAndAssign,
        Token::PipeEq = e => AssignOpKind::BitOrAssign,
        Token::CaretEq = e => AssignOpKind::BitXorAssign,
        Token::ShlEq = e => AssignOpKind::ShlAssign,
        Token::ShrEq = e => AssignOpKind::ShrAssign,
    }
    .map_with(|kind, e| AssignOp {
        kind,
        span: span(e),
    })
}

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

/// Parses a unary operator
pub fn un_op<'src>() -> impl FigParser<'src, UnOp> {
    select! {
        Token::Bang => UnOp::Not,
        Token::Minus => UnOp::Neg,
    }
}

// `Ty` is the root of a mutually-recursive cluster: Ty -> TyKind -> (Path,
// FnTy) -> GenericArgs -> GenericArg -> AssocItemConstraint -> Ty (and Ty
// is self-recursive too, via Array/Tup/Paren/Fn). Each node here used to
// be its own zero-argument `pub fn foo() -> impl FigParser<...>` that
// called `ty()` directly, but Rust can't resolve a cycle of *opaque*
// (`impl Trait`) return types — every one of those functions' hidden
// concrete type would depend on the others', with no base case
// (`error[E0720]: cannot resolve opaque type`).
//
// The fix: only `ty()` ties the knot, via `recursive()`, which hands back
// a `Recursive<...>` — a concrete, nameable type, not an opaque one. Every
// other function in the cluster stops calling `ty()` itself and instead
// takes the in-progress `ty` parser as a parameter, so the cycle no
// longer exists at the function-signature level at all — only inside
// `ty()`'s own closure.

pub fn fn_ret_ty<'src>(ty: impl FigParser<'src, Ty> + 'src) -> impl FigParser<'src, FnRetTy> {
    just(Token::Arrow)
        .ignore_then(ty)
        .map(Box::new)
        .map(FnRetTy::Ty)
        .or(empty().map_with(|_, e| FnRetTy::Default(span(e))))
}

fn fn_ty<'src>(ty: impl FigParser<'src, Ty> + 'src) -> impl FigParser<'src, FnTy> {
    just(Token::Identifier("Fn"))
        .ignore_then(
            ty.clone()
                .map(Box::new)
                .separated_by(just(Token::Comma))
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LParen), just(Token::RParen)),
        )
        .then(fn_ret_ty(ty))
        .map(|(inputs, output)| FnTy { inputs, output })
}

fn ty_kind<'src>(ty: impl FigParser<'src, Ty> + 'src) -> impl FigParser<'src, TyKind> {
    just(Token::Bang)
        .map(|_| TyKind::Never)
        .or(just(Token::Identifier("_")).map(|_| TyKind::Infer))
        .or(fn_ty(ty.clone()).map(Box::new).map(TyKind::Fn))
        .or(path(ty.clone()).map(TyKind::Path))
        .or(just(Token::LBracket)
            .ignore_then(ty.clone())
            .then_ignore(just(Token::RBracket))
            .map(Box::new)
            .map(TyKind::Array))
        .or(just(Token::LParen)
            .ignore_then(
                ty.clone()
                    .map(Box::new)
                    .then_ignore(just(Token::Comma))
                    .repeated()
                    .collect::<Vec<_>>()
                    .then(ty.clone().map(Box::new).or_not()),
            )
            .then_ignore(just(Token::RParen))
            .map(|(mut elems, last)| match (elems.is_empty(), last) {
                (true, Some(only)) => TyKind::Paren(only),
                (_, last) => {
                    elems.extend(last);
                    TyKind::Tup(elems)
                }
            }))
}

pub fn ty<'src>() -> impl FigParser<'src, Ty> {
    recursive(|ty| {
        ty_kind(ty).map_with(|kind, e| Ty {
            kind,
            span: span(e),
        })
    })
}

// A second, independent cycle lives entirely inside generic-arg lists:
// AssocItemConstraint -> GenericArgs -> GenericArg -> AssocItemConstraint
// (a constraint's own name can itself carry generics, e.g.
// `Elem<T> = Wrapper<T>>`). Same problem as the Ty cluster above, same
// fix: `generic_arg` ties this knot via `recursive()`, and the actual
// field-building logic lives in these `_with` helpers so both the tied
// closure and the standalone `pub` functions below can share it without
// the `pub` functions calling each other's opaque return types directly.

fn assoc_item_constraint_with<'src>(
    ty: impl FigParser<'src, Ty> + 'src,
    generic_arg: impl FigParser<'src, GenericArg> + 'src,
) -> impl FigParser<'src, AssocItemConstraint> {
    ident()
        .then(generic_args_with(generic_arg).or_not())
        .then_ignore(just(Token::Eq))
        .then(ty)
        .map_with(|((ident, gen_args), ty), e| AssocItemConstraint {
            ident,
            gen_args,
            ty,
            span: span(e),
        })
}

fn generic_args_with<'src>(
    generic_arg: impl FigParser<'src, GenericArg> + 'src,
) -> impl FigParser<'src, GenericArgs> {
    // grammar.md: `"<" type ("," type)* ">"` — at least one arg, and
    // actually delimited by `<`/`>` (missing entirely before this fix,
    // which meant this matched zero args and consumed nothing, silently
    // leaving any real `<...>` in the input untouched).
    generic_arg
        .separated_by(just(Token::Comma))
        .at_least(1)
        .collect::<Vec<_>>()
        .delimited_by(just(Token::Lt), just(Token::Gt))
        .map_with(|args, e| GenericArgs {
            args,
            span: span(e),
        })
}

pub fn generic_arg<'src>(ty: impl FigParser<'src, Ty> + 'src) -> impl FigParser<'src, GenericArg> {
    recursive(move |generic_arg| {
        assoc_item_constraint_with(ty.clone(), generic_arg.clone())
            .map(GenericArg::Constraint)
            .or(ty.clone().map(GenericArg::Arg))
    })
}

pub fn generic_args<'src>(
    ty: impl FigParser<'src, Ty> + 'src,
) -> impl FigParser<'src, GenericArgs> {
    generic_args_with(generic_arg(ty))
}

pub fn assoc_item_constraint<'src>(
    ty: impl FigParser<'src, Ty> + 'src,
) -> impl FigParser<'src, AssocItemConstraint> {
    assoc_item_constraint_with(ty.clone(), generic_arg(ty))
}

pub fn path_segment<'src>(
    ty: impl FigParser<'src, Ty> + 'src,
) -> impl FigParser<'src, PathSegment> {
    ident()
        .then(generic_args(ty).or_not())
        .map(|(ident, args)| PathSegment { ident, args })
}

pub fn path<'src>(ty: impl FigParser<'src, Ty> + 'src) -> impl FigParser<'src, Path> {
    path_segment(ty)
        .separated_by(just(Token::PathSep))
        .at_least(1)
        .collect::<Vec<_>>()
        .map_with(|segments, e| Path {
            segments,
            span: span(e),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(source: &str) -> Vec<Token<'_>> {
        lexer::tokenize_all(source).expect("test input should lex cleanly")
    }

    #[test]
    fn parses_an_ident_with_its_span() {
        let tokens = tokens("foo");
        let parsed = ident().parse(&tokens).into_result().expect("should parse");
        assert_eq!(parsed.name, "foo");
        assert_eq!(parsed.span, ast::Span { start: 0, end: 1 });
    }

    #[test]
    fn rejects_underscore_as_an_ordinary_identifier() {
        let tokens = tokens("_");
        assert!(ident().parse(&tokens).into_result().is_err());
    }

    #[test]
    fn parses_a_label() {
        let tokens = tokens("outer:");
        let parsed = label().parse(&tokens).into_result().expect("should parse");
        assert_eq!(parsed.ident.name, "outer");
    }

    #[test]
    fn rejects_an_identifier_with_no_colon() {
        let tokens = tokens("outer");
        assert!(label().parse(&tokens).into_result().is_err());
    }

    #[test]
    fn rejects_underscore_as_a_label() {
        let tokens = tokens("_:");
        assert!(label().parse(&tokens).into_result().is_err());
    }

    #[test]
    fn parses_a_plain_char_literal() {
        let tokens = tokens("'a'");
        let parsed = literal()
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.kind, LitKind::Char('a'));
        assert_eq!(parsed.span, ast::Span { start: 0, end: 1 });
    }

    #[test]
    fn parses_a_char_literal_with_a_simple_escape() {
        let tokens = tokens(r"'\n'");
        let parsed = literal()
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.kind, LitKind::Char('\n'));
    }

    #[test]
    fn parses_a_char_literal_with_a_unicode_escape() {
        let tokens = tokens(r"'\u{1F980}'");
        let parsed = literal()
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.kind, LitKind::Char('🦀'));
    }

    #[test]
    fn parses_a_plain_string_literal() {
        let tokens = tokens(r#""hello, world""#);
        let parsed = literal()
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.kind, LitKind::Str("hello, world".to_owned()));
        assert_eq!(parsed.span, ast::Span { start: 0, end: 1 });
    }

    #[test]
    fn parses_a_string_literal_with_escapes() {
        let tokens = tokens(r#""a\nb\tc""#);
        let parsed = literal()
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.kind, LitKind::Str("a\nb\tc".to_owned()));
    }

    #[test]
    fn parses_a_raw_string_literal_without_processing_escapes() {
        let tokens = tokens(r#"r"C:\Users\name""#);
        let parsed = literal()
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.kind, LitKind::Str(r"C:\Users\name".to_owned()));
    }

    #[test]
    fn parses_a_hashed_raw_string_literal_containing_a_quote() {
        let tokens = tokens(r###"r#"she said "hi""#"###);
        let parsed = literal()
            .parse(&tokens)
            .into_result()
            .expect("should parse");
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
            let parsed = literal()
                .parse(&tokens)
                .into_result()
                .expect("should parse");
            assert_eq!(parsed.kind, LitKind::Int(expected), "input: {src}");
        }
    }

    #[test]
    fn parses_a_float_literal_as_raw_text() {
        let tokens = tokens("3.14");
        let parsed = literal()
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.kind, LitKind::Float("3.14".to_owned()));
    }

    #[test]
    fn parses_every_binary_operator() {
        let cases = [
            ("+", BinOpKind::Add),
            ("-", BinOpKind::Sub),
            ("*", BinOpKind::Mul),
            ("/", BinOpKind::Div),
            ("%", BinOpKind::Rem),
            ("&&", BinOpKind::And),
            ("||", BinOpKind::Or),
            ("^", BinOpKind::BitXor),
            ("&", BinOpKind::BitAnd),
            ("|", BinOpKind::BitOr),
            ("<<", BinOpKind::Shl),
            (">>", BinOpKind::Shr),
            ("==", BinOpKind::Eq),
            ("<", BinOpKind::Lt),
            ("<=", BinOpKind::Le),
            ("!=", BinOpKind::Ne),
            (">=", BinOpKind::Ge),
            (">", BinOpKind::Gt),
        ];
        for (src, expected) in cases {
            let tokens = tokens(src);
            let parsed = bin_op().parse(&tokens).into_result().expect("should parse");
            assert_eq!(parsed.kind, expected, "input: {src}");
            assert_eq!(parsed.span, ast::Span { start: 0, end: 1 }, "input: {src}");
        }
    }

    #[test]
    fn parses_every_compound_assignment_operator() {
        let cases = [
            ("+=", AssignOpKind::AddAssign),
            ("-=", AssignOpKind::SubAssign),
            ("*=", AssignOpKind::MulAssign),
            ("/=", AssignOpKind::DivAssign),
            ("%=", AssignOpKind::RemAssign),
            ("&=", AssignOpKind::BitAndAssign),
            ("|=", AssignOpKind::BitOrAssign),
            ("^=", AssignOpKind::BitXorAssign),
            ("<<=", AssignOpKind::ShlAssign),
            (">>=", AssignOpKind::ShrAssign),
        ];
        for (src, expected) in cases {
            let tokens = tokens(src);
            let parsed = assign_op()
                .parse(&tokens)
                .into_result()
                .expect("should parse");
            assert_eq!(parsed.kind, expected, "input: {src}");
            assert_eq!(parsed.span, ast::Span { start: 0, end: 1 }, "input: {src}");
        }
    }

    #[test]
    fn rejects_plain_eq_as_an_assign_op() {
        let tokens = tokens("=");
        assert!(assign_op().parse(&tokens).into_result().is_err());
    }

    #[test]
    fn parses_both_unary_operators() {
        let cases = [("!", UnOp::Not), ("-", UnOp::Neg)];
        for (src, expected) in cases {
            let tokens = tokens(src);
            let parsed = un_op().parse(&tokens).into_result().expect("should parse");
            assert_eq!(parsed, expected, "input: {src}");
        }
    }

    #[test]
    fn parses_the_never_type() {
        let tokens = tokens("!");
        let parsed = ty().parse(&tokens).into_result().expect("should parse");
        assert!(matches!(parsed.kind, TyKind::Never));
    }

    #[test]
    fn parses_the_infer_type() {
        let tokens = tokens("_");
        let parsed = ty().parse(&tokens).into_result().expect("should parse");
        assert!(matches!(parsed.kind, TyKind::Infer));
    }

    #[test]
    fn parses_a_simple_path_type() {
        let tokens = tokens("int");
        let parsed = ty().parse(&tokens).into_result().expect("should parse");
        let TyKind::Path(path) = parsed.kind else {
            panic!("expected TyKind::Path, got {:?}", parsed.kind);
        };
        assert_eq!(path.segments.len(), 1);
        assert_eq!(path.segments[0].ident.name, "int");
        assert!(path.segments[0].args.is_none());
    }

    #[test]
    fn parses_an_array_type() {
        let tokens = tokens("[int]");
        let parsed = ty().parse(&tokens).into_result().expect("should parse");
        let TyKind::Array(elem) = parsed.kind else {
            panic!("expected TyKind::Array, got {:?}", parsed.kind);
        };
        assert!(matches!(elem.kind, TyKind::Path(_)));
    }

    #[test]
    fn parses_the_unit_type() {
        let tokens = tokens("()");
        let parsed = ty().parse(&tokens).into_result().expect("should parse");
        let TyKind::Tup(elems) = parsed.kind else {
            panic!("expected TyKind::Tup, got {:?}", parsed.kind);
        };
        assert!(elems.is_empty());
    }

    #[test]
    fn parses_a_parenthesized_type_as_paren_not_a_one_tuple() {
        let tokens = tokens("(int)");
        let parsed = ty().parse(&tokens).into_result().expect("should parse");
        assert!(
            matches!(parsed.kind, TyKind::Paren(_)),
            "got {:?}",
            parsed.kind
        );
    }

    #[test]
    fn parses_a_trailing_comma_as_a_one_tuple() {
        let tokens = tokens("(int,)");
        let parsed = ty().parse(&tokens).into_result().expect("should parse");
        let TyKind::Tup(elems) = parsed.kind else {
            panic!("expected TyKind::Tup, got {:?}", parsed.kind);
        };
        assert_eq!(elems.len(), 1);
    }

    #[test]
    fn parses_a_multi_element_tuple_type() {
        let tokens = tokens("(int, float)");
        let parsed = ty().parse(&tokens).into_result().expect("should parse");
        let TyKind::Tup(elems) = parsed.kind else {
            panic!("expected TyKind::Tup, got {:?}", parsed.kind);
        };
        assert_eq!(elems.len(), 2);
    }

    #[test]
    fn parses_an_fn_type_with_inputs_and_an_explicit_output() {
        let tokens = tokens("Fn(int, int) -> int");
        let parsed = ty().parse(&tokens).into_result().expect("should parse");
        let TyKind::Fn(fn_ty) = parsed.kind else {
            panic!("expected TyKind::Fn, got {:?}", parsed.kind);
        };
        assert_eq!(fn_ty.inputs.len(), 2);
        assert!(matches!(fn_ty.output, FnRetTy::Ty(_)));
    }

    #[test]
    fn parses_an_fn_type_with_no_inputs_and_a_default_output() {
        let tokens = tokens("Fn()");
        let parsed = ty().parse(&tokens).into_result().expect("should parse");
        let TyKind::Fn(fn_ty) = parsed.kind else {
            panic!("expected TyKind::Fn, got {:?}", parsed.kind);
        };
        assert!(fn_ty.inputs.is_empty());
        assert!(matches!(fn_ty.output, FnRetTy::Default(_)));
    }

    #[test]
    fn parses_a_generic_path_type() {
        let tokens = tokens("Vec<int>");
        let parsed = ty().parse(&tokens).into_result().expect("should parse");
        let TyKind::Path(path) = parsed.kind else {
            panic!("expected TyKind::Path, got {:?}", parsed.kind);
        };
        assert_eq!(path.segments.len(), 1);
        assert_eq!(path.segments[0].ident.name, "Vec");
        let args = path.segments[0]
            .args
            .as_ref()
            .expect("should have generic args");
        assert_eq!(args.args.len(), 1);
        assert!(matches!(args.args[0], GenericArg::Arg(_)));
    }

    #[test]
    fn parses_a_nested_array_of_fn_types() {
        let tokens = tokens("[Fn(int) -> int]");
        let parsed = ty().parse(&tokens).into_result().expect("should parse");
        let TyKind::Array(elem) = parsed.kind else {
            panic!("expected TyKind::Array, got {:?}", parsed.kind);
        };
        assert!(matches!(elem.kind, TyKind::Fn(_)));
    }

    #[test]
    fn parses_a_single_segment_path() {
        let tokens = tokens("foo");
        let parsed = path(ty())
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.segments.len(), 1);
        assert_eq!(parsed.segments[0].ident.name, "foo");
    }

    #[test]
    fn parses_a_multi_segment_path() {
        let tokens = tokens("foo::bar::baz");
        let parsed = path(ty())
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        let names: Vec<_> = parsed
            .segments
            .iter()
            .map(|s| s.ident.name.as_str())
            .collect();
        assert_eq!(names, vec!["foo", "bar", "baz"]);
    }

    #[test]
    fn rejects_an_empty_path() {
        let tokens = tokens("");
        assert!(path(ty()).parse(&tokens).into_result().is_err());
    }

    #[test]
    fn parses_a_path_segment_without_generics() {
        let tokens = tokens("foo");
        let parsed = path_segment(ty())
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.ident.name, "foo");
        assert!(parsed.args.is_none());
    }

    #[test]
    fn parses_a_path_segment_with_generics() {
        let tokens = tokens("Vec<int>");
        let parsed = path_segment(ty())
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.ident.name, "Vec");
        assert_eq!(parsed.args.expect("should have generic args").args.len(), 1);
    }

    #[test]
    fn parses_a_single_generic_arg() {
        let tokens = tokens("<int>");
        let parsed = generic_args(ty())
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.args.len(), 1);
    }

    #[test]
    fn parses_multiple_generic_args() {
        let tokens = tokens("<int, float>");
        let parsed = generic_args(ty())
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.args.len(), 2);
    }

    #[test]
    fn rejects_empty_generic_args() {
        let tokens = tokens("<>");
        assert!(generic_args(ty()).parse(&tokens).into_result().is_err());
    }

    #[test]
    fn parses_a_plain_type_as_a_generic_arg() {
        let tokens = tokens("int");
        let parsed = generic_arg(ty())
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert!(matches!(parsed, GenericArg::Arg(_)));
    }

    #[test]
    fn parses_an_assoc_item_constraint_as_a_generic_arg() {
        let tokens = tokens("Item = int");
        let parsed = generic_arg(ty())
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        let GenericArg::Constraint(constraint) = parsed else {
            panic!("expected GenericArg::Constraint, got {:?}", parsed);
        };
        assert_eq!(constraint.ident.name, "Item");
        assert!(constraint.gen_args.is_none());
    }

    #[test]
    fn parses_an_assoc_item_constraint_directly() {
        let tokens = tokens("Item = int");
        let parsed = assoc_item_constraint(ty())
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.ident.name, "Item");
        assert!(matches!(parsed.ty.kind, TyKind::Path(_)));
    }

    #[test]
    fn parses_an_assoc_item_constraint_with_its_own_generics() {
        let tokens = tokens("Elem<T> = int");
        let parsed = assoc_item_constraint(ty())
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.ident.name, "Elem");
        assert_eq!(
            parsed
                .gen_args
                .expect("should have its own generic args")
                .args
                .len(),
            1
        );
    }

    #[test]
    fn parses_an_explicit_fn_return_type() {
        let tokens = tokens("-> int");
        let parsed = fn_ret_ty(ty())
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert!(matches!(parsed, FnRetTy::Ty(_)));
    }

    #[test]
    fn parses_a_default_fn_return_type_when_no_arrow_is_present() {
        let tokens = tokens("");
        let parsed = fn_ret_ty(ty())
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert!(matches!(parsed, FnRetTy::Default(_)));
    }
}
