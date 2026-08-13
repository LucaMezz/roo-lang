//! Hand-written tests covering the lexical surface described in
//! `book/src/appendix/grammar.md` and the lexical-structure chapters,
//! beyond what the book's own prose examples happen to exercise:
//! every keyword individually, every operator/punctuation token, every
//! literal form and edge case, and a handful of larger synthetic snippets
//! assembled directly from grammar constructs.

use lexer::{tokenize_all, LexError, NumberKind, Token};

fn lex(src: &str) -> Vec<Token<'_>> {
    tokenize_all(src).unwrap_or_else(|err| panic!("expected {src:?} to lex cleanly, got {err:?}"))
}

fn assert_tokens(src: &str, expected: &[Token]) {
    assert_eq!(lex(src), expected, "unexpected tokens for {src:?}");
}

fn assert_single(src: &str, expected: Token) {
    assert_tokens(src, &[expected]);
}

fn assert_lex_error(src: &str, expected: LexError) {
    match tokenize_all(src) {
        Ok(toks) => panic!("expected {src:?} to fail lexing with {expected:?}, got {toks:?}"),
        Err(err) => assert_eq!(err, expected, "wrong error for {src:?}"),
    }
}

// ---------------------------------------------------------------------
// Strict keywords — appendix/keywords.md's "Strict keywords" table.
// ---------------------------------------------------------------------

mod strict_keywords {
    use super::*;

    macro_rules! keyword_test {
        ($name:ident, $src:literal, $tok:expr) => {
            #[test]
            fn $name() {
                assert_single($src, $tok);
            }
        };
    }

    keyword_test!(as_, "as", Token::As);
    keyword_test!(break_, "break", Token::Break);
    keyword_test!(continue_, "continue", Token::Continue);
    keyword_test!(else_, "else", Token::Else);
    keyword_test!(enum_, "enum", Token::Enum);
    keyword_test!(false_, "false", Token::False);
    keyword_test!(fn_, "fn", Token::Fn);
    keyword_test!(for_, "for", Token::For);
    keyword_test!(if_, "if", Token::If);
    keyword_test!(impl_, "impl", Token::Impl);
    keyword_test!(in_, "in", Token::In);
    keyword_test!(let_, "let", Token::Let);
    keyword_test!(loop_, "loop", Token::Loop);
    keyword_test!(match_, "match", Token::Match);
    keyword_test!(mod_, "mod", Token::Mod);
    keyword_test!(mut_, "mut", Token::Mut);
    keyword_test!(pub_, "pub", Token::Pub);
    keyword_test!(return_, "return", Token::Return);
    keyword_test!(self_lower, "self", Token::SelfLower);
    keyword_test!(self_upper, "Self", Token::SelfUpper);
    keyword_test!(struct_, "struct", Token::Struct);
    keyword_test!(super_, "super", Token::Super);
    keyword_test!(trait_, "trait", Token::Trait);
    keyword_test!(true_, "true", Token::True);
    keyword_test!(type_, "type", Token::Type);
    keyword_test!(use_, "use", Token::Use);
    keyword_test!(where_, "where", Token::Where);
    keyword_test!(while_, "while", Token::While);

    // Reserved, currently unused (appendix/keywords.md).
    keyword_test!(dyn_, "dyn", Token::Dyn);
    keyword_test!(const_, "const", Token::Const);

    #[test]
    fn keywords_dont_match_as_a_prefix_of_a_longer_identifier() {
        // `letter` must be one Identifier, not `let` + `ter`.
        assert_single("letter", Token::Identifier("letter"));
        assert_single("structural", Token::Identifier("structural"));
        assert_single("format", Token::Identifier("format"));
        assert_single("inside", Token::Identifier("inside"));
        assert_single("format_str", Token::Identifier("format_str"));
        assert_single("self_destruct", Token::Identifier("self_destruct"));
    }
}

// ---------------------------------------------------------------------
// Words that are Rust keywords but ordinary identifiers in fig, per
// lexical/identifiers-and-keywords.md's "Not keywords in fig" list.
// ---------------------------------------------------------------------

#[test]
fn removed_rust_keywords_lex_as_plain_identifiers() {
    for word in [
        "unsafe", "move", "static", "extern", "ref", "box", "async", "await", "yield",
        "abstract", "final", "override", "priv", "typeof", "unsized", "virtual", "crate",
    ] {
        assert_single(word, Token::Identifier(word));
    }
}

#[test]
fn wildcard_is_an_ordinary_identifier_token() {
    // "Is `_` a wildcard or a name" is a parser-level distinction, not a
    // lexical one — see lexical/identifiers-and-keywords.md.
    assert_single("_", Token::Identifier("_"));
    assert_single("_unused", Token::Identifier("_unused"));
}

#[test]
fn identifiers_can_start_with_underscore_or_contain_digits() {
    assert_single("_foo", Token::Identifier("_foo"));
    assert_single("foo_bar_42", Token::Identifier("foo_bar_42"));
    assert_single("__", Token::Identifier("__"));
}

// ---------------------------------------------------------------------
// Operators and punctuation — expressions/operators.md +
// appendix/operator-precedence.md.
// ---------------------------------------------------------------------

mod operators_and_punctuation {
    use super::*;

    macro_rules! op_test {
        ($name:ident, $src:literal, $tok:expr) => {
            #[test]
            fn $name() {
                assert_single($src, $tok);
            }
        };
    }

    // Arithmetic
    op_test!(plus, "+", Token::Plus);
    op_test!(minus, "-", Token::Minus);
    op_test!(star, "*", Token::Star);
    op_test!(slash, "/", Token::Slash);
    op_test!(percent, "%", Token::Percent);

    // Comparison
    op_test!(eq_eq, "==", Token::EqEq);
    op_test!(not_eq, "!=", Token::NotEq);
    op_test!(lt, "<", Token::Lt);
    op_test!(gt, ">", Token::Gt);
    op_test!(lt_eq, "<=", Token::LtEq);
    op_test!(gt_eq, ">=", Token::GtEq);

    // Logical
    op_test!(and_and, "&&", Token::AndAnd);
    op_test!(or_or, "||", Token::OrOr);
    op_test!(bang, "!", Token::Bang);

    // Bitwise
    op_test!(amp, "&", Token::Amp);
    op_test!(pipe, "|", Token::Pipe);
    op_test!(caret, "^", Token::Caret);
    op_test!(shl, "<<", Token::Shl);
    op_test!(shr, ">>", Token::Shr);

    // Assignment
    op_test!(eq, "=", Token::Eq);
    op_test!(plus_eq, "+=", Token::PlusEq);
    op_test!(minus_eq, "-=", Token::MinusEq);
    op_test!(star_eq, "*=", Token::StarEq);
    op_test!(slash_eq, "/=", Token::SlashEq);
    op_test!(percent_eq, "%=", Token::PercentEq);
    op_test!(amp_eq, "&=", Token::AmpEq);
    op_test!(pipe_eq, "|=", Token::PipeEq);
    op_test!(caret_eq, "^=", Token::CaretEq);
    op_test!(shl_eq, "<<=", Token::ShlEq);
    op_test!(shr_eq, ">>=", Token::ShrEq);

    // Range
    op_test!(dot_dot, "..", Token::DotDot);
    op_test!(dot_dot_eq, "..=", Token::DotDotEq);

    // Member/path
    op_test!(dot, ".", Token::Dot);
    op_test!(path_sep, "::", Token::PathSep);

    // Error propagation
    op_test!(question, "?", Token::Question);

    // Arrows
    op_test!(arrow, "->", Token::Arrow);
    op_test!(fat_arrow, "=>", Token::FatArrow);

    // Misc
    op_test!(colon, ":", Token::Colon);
    op_test!(semi, ";", Token::Semi);
    op_test!(comma, ",", Token::Comma);
    op_test!(at, "@", Token::At);
    op_test!(pound, "#", Token::Pound);

    // Brackets
    op_test!(lparen, "(", Token::LParen);
    op_test!(rparen, ")", Token::RParen);
    op_test!(lbrace, "{", Token::LBrace);
    op_test!(rbrace, "}", Token::RBrace);
    op_test!(lbracket, "[", Token::LBracket);
    op_test!(rbracket, "]", Token::RBracket);

    #[test]
    fn longest_match_wins_for_compound_operators() {
        use Token::*;
        assert_tokens("<<=", &[ShlEq]);
        assert_tokens("<< =", &[Shl, Eq]);
        assert_tokens("<=>", &[LtEq, Gt]);
        assert_tokens("...", &[DotDot, Dot]); // not real fig syntax, but must still lex
        assert_tokens("..=..", &[DotDotEq, DotDot]);
    }

    #[test]
    fn annotation_shapes_lex_as_separate_tokens_not_a_raw_string() {
        use Token::*;
        assert_tokens(
            "#[replicated]",
            &[Pound, LBracket, Identifier("replicated"), RBracket],
        );
        assert_tokens(
            "#![replicated]",
            &[Pound, Bang, LBracket, Identifier("replicated"), RBracket],
        );
        // A bare `#` doesn't feed into raw-string lexing — only `r#*"`
        // starting from the `r` does.
        assert_tokens("# \"hi\"", &[Pound, StringLiteral("\"hi\"")]);
    }

    #[test]
    fn nested_generics_closing_brackets_lex_as_shr_not_two_gt() {
        // Splitting `>>` back into two `>` for `Vec<Vec<int>>`-style
        // nested generics is a parser concern, not a lexer one — the
        // lexer's job is consistent maximal munch (matches how Rust's
        // own lexer/parser split this responsibility too).
        assert_tokens(
            "Vec<Vec<int>>",
            &[
                Token::Identifier("Vec"),
                Token::Lt,
                Token::Identifier("Vec"),
                Token::Lt,
                Token::Identifier("int"),
                Token::Shr,
            ],
        );
    }

    #[test]
    fn operators_dont_need_surrounding_whitespace() {
        use Token::*;
        assert_tokens(
            "a+b==c&&d",
            &[
                Token::Identifier("a"),
                Plus,
                Token::Identifier("b"),
                EqEq,
                Token::Identifier("c"),
                AndAnd,
                Token::Identifier("d"),
            ],
        );
    }

    #[test]
    fn pipe_is_shared_between_bitwise_or_and_closure_delimiters() {
        // `|x| x + 1` — the lexer just emits Pipe both times; telling
        // "closure start" from "bitwise or" apart is the parser's job.
        assert_tokens(
            "|x| x + 1",
            &[
                Token::Pipe,
                Token::Identifier("x"),
                Token::Pipe,
                Token::Identifier("x"),
                Token::Plus,
                Token::Number(NumberKind::Int("1")),
            ],
        );
    }
}

// ---------------------------------------------------------------------
// Numeric literals — lexical/literals.md.
// ---------------------------------------------------------------------

mod numeric_literals {
    use super::*;
    use NumberKind::*;

    #[test]
    fn plain_decimal() {
        assert_single("42", Token::Number(Int("42")));
        assert_single("0", Token::Number(Int("0")));
    }

    #[test]
    fn decimal_with_underscore_separators() {
        assert_single("1_000_000", Token::Number(Int("1_000_000")));
    }

    #[test]
    fn hex_octal_binary() {
        assert_single("0xFF", Token::Number(Int("0xFF")));
        assert_single("0Xff", Token::Number(Int("0Xff")));
        assert_single("0o17", Token::Number(Int("0o17")));
        assert_single("0O17", Token::Number(Int("0O17")));
        assert_single("0b1010_1010", Token::Number(Int("0b1010_1010")));
        assert_single("0B11", Token::Number(Int("0B11")));
    }

    #[test]
    fn float_basic_and_with_separators() {
        assert_single("3.14159", Token::Number(Float("3.14159")));
        assert_single("1_000.0", Token::Number(Float("1_000.0")));
    }

    #[test]
    fn float_trailing_dot() {
        assert_single("5.", Token::Number(Float("5.")));
    }

    #[test]
    fn float_with_exponent() {
        assert_single("1.5e10", Token::Number(Float("1.5e10")));
        assert_single("2.0e-3", Token::Number(Float("2.0e-3")));
        assert_single("2.0E+3", Token::Number(Float("2.0E+3")));
    }

    #[test]
    fn float_exponent_without_a_dot() {
        // Not shown explicitly in the book, but a natural Rust-like form:
        // an integer-looking mantissa with an exponent is still a float.
        assert_single("5e10", Token::Number(Float("5e10")));
        assert_single("5E-2", Token::Number(Float("5E-2")));
    }

    #[test]
    fn trailing_dot_before_identifier_is_not_absorbed_into_the_float() {
        // `5.sqrt()` — the `.` belongs to member access, not the number.
        assert_tokens(
            "5.sqrt()",
            &[
                Token::Number(Int("5")),
                Token::Dot,
                Token::Identifier("sqrt"),
                Token::LParen,
                Token::RParen,
            ],
        );
    }

    #[test]
    fn trailing_dot_before_underscore_is_not_absorbed_either() {
        assert_tokens(
            "5._foo",
            &[Token::Number(Int("5")), Token::Dot, Token::Identifier("_foo")],
        );
    }

    #[test]
    fn trailing_dot_before_range_is_not_absorbed() {
        assert_tokens(
            "0..5",
            &[Token::Number(Int("0")), Token::DotDot, Token::Number(Int("5"))],
        );
        assert_tokens(
            "0..=5",
            &[Token::Number(Int("0")), Token::DotDotEq, Token::Number(Int("5"))],
        );
    }

    #[test]
    fn tuple_field_access_is_not_confused_with_a_float() {
        assert_tokens(
            "point.0",
            &[Token::Identifier("point"), Token::Dot, Token::Number(Int("0"))],
        );
        assert_tokens(
            "point.1",
            &[Token::Identifier("point"), Token::Dot, Token::Number(Int("1"))],
        );
    }

    #[test]
    fn negative_numbers_are_unary_minus_plus_a_literal_not_one_token() {
        assert_tokens("-17", &[Token::Minus, Token::Number(Int("17"))]);
        assert_tokens("-1.5", &[Token::Minus, Token::Number(Float("1.5"))]);
    }

    #[test]
    fn hex_int_immediately_followed_by_a_dot_does_not_need_disambiguation() {
        // Hex/octal/binary have no float form, so a trailing `.` is
        // always just member access, unconditionally.
        assert_tokens(
            "0xFF.to_string()",
            &[
                Token::Number(Int("0xFF")),
                Token::Dot,
                Token::Identifier("to_string"),
                Token::LParen,
                Token::RParen,
            ],
        );
    }
}

// ---------------------------------------------------------------------
// String, char, and raw string literals — lexical/literals.md.
// ---------------------------------------------------------------------

mod string_and_char_literals {
    use super::*;

    #[test]
    fn plain_string() {
        assert_single("\"hello, world\"", Token::StringLiteral("\"hello, world\""));
    }

    #[test]
    fn empty_string() {
        assert_single("\"\"", Token::StringLiteral("\"\""));
    }

    #[test]
    fn string_with_escaped_quote_is_not_terminated_early() {
        let src = "\"she said \\\"hi\\\"\"";
        assert_single(src, Token::StringLiteral(src));
    }

    #[test]
    fn string_containing_a_literal_newline_is_one_token() {
        let src = "\"line one\nline two\"";
        assert_single(src, Token::StringLiteral(src));
    }

    #[test]
    fn string_with_every_documented_escape() {
        let src = r#""\n\r\t\\\0\"\'""#;
        assert_single(src, Token::StringLiteral(src));
    }

    #[test]
    fn string_with_unicode_escape() {
        let src = "\"\\u{1F980}\"";
        assert_single(src, Token::StringLiteral(src));
    }

    #[test]
    fn string_with_multibyte_content() {
        let src = "\"héllo 🦀 wörld\"";
        assert_single(src, Token::StringLiteral(src));
    }

    #[test]
    fn unterminated_string_is_a_lex_error() {
        assert_lex_error("\"never closed", LexError::UnterminatedString);
    }

    #[test]
    fn char_ascii() {
        assert_single("'a'", Token::CharLiteral("'a'"));
    }

    #[test]
    fn char_escapes() {
        for c in ["'\\n'", "'\\r'", "'\\t'", "'\\\\'", "'\\0'", "'\\\"'", "'\\''"] {
            assert_single(c, Token::CharLiteral(c));
        }
    }

    #[test]
    fn char_unicode_escape() {
        assert_single("'\\u{1F980}'", Token::CharLiteral("'\\u{1F980}'"));
        assert_single("'\\u{0}'", Token::CharLiteral("'\\u{0}'"));
    }

    #[test]
    fn char_multibyte_literal() {
        assert_single("'🦀'", Token::CharLiteral("'🦀'"));
        assert_single("'é'", Token::CharLiteral("'é'"));
    }

    #[test]
    fn unterminated_char_is_a_lex_error() {
        assert_lex_error("'a", LexError::UnterminatedChar);
        assert_lex_error("'\\n", LexError::UnterminatedChar);
    }

    #[test]
    fn malformed_unicode_escape_is_a_lex_error() {
        assert_lex_error("'\\u{}'", LexError::UnterminatedChar);
        assert_lex_error("'\\u{FFFF'", LexError::UnterminatedChar);
    }

    #[test]
    fn raw_string_no_hashes() {
        let src = "r\"C:\\Users\\name\"";
        assert_single(src, Token::RawStringLiteral(src));
    }

    #[test]
    fn raw_string_one_hash_with_embedded_quotes() {
        let src = "r#\"she said \"hello\"\"#";
        assert_single(src, Token::RawStringLiteral(src));
    }

    #[test]
    fn raw_string_multiple_hashes() {
        let src = "r##\"contains \"#\" inside\"##";
        assert_single(src, Token::RawStringLiteral(src));
    }

    #[test]
    fn raw_string_backslashes_are_literal_not_escapes() {
        let src = "r\"no \\n escape here\"";
        assert_single(src, Token::RawStringLiteral(src));
    }

    #[test]
    fn unterminated_raw_string_is_a_lex_error() {
        assert_lex_error("r\"never closed", LexError::UnterminatedRawString);
        assert_lex_error("r#\"never closed\"", LexError::UnterminatedRawString);
    }
}

// ---------------------------------------------------------------------
// Comments — lexical/comments.md.
// ---------------------------------------------------------------------

mod comments {
    use super::*;

    #[test]
    fn line_comment_is_skipped() {
        assert_tokens("// a comment\nlet x = 1;", &lex("let x = 1;"));
    }

    #[test]
    fn line_comment_trailing_code() {
        assert_tokens(
            "let x = 1; // trailing",
            &[Token::Let, Token::Identifier("x"), Token::Eq, Token::Number(NumberKind::Int("1")), Token::Semi],
        );
    }

    #[test]
    fn block_comment_is_skipped() {
        assert_tokens("/* a comment */let x = 1;", &lex("let x = 1;"));
    }

    #[test]
    fn multiline_block_comment_is_skipped() {
        assert_tokens("/*\n * spans\n * lines\n */\nlet x = 1;", &lex("let x = 1;"));
    }

    #[test]
    fn nested_block_comments() {
        assert_tokens(
            "/* outer /* inner */ still outer */let x = 1;",
            &lex("let x = 1;"),
        );
    }

    #[test]
    fn triple_nested_block_comments() {
        assert_tokens("/* a /* b /* c */ b */ a */ok", &[Token::Identifier("ok")]);
    }

    #[test]
    fn unterminated_block_comment_is_a_lex_error() {
        assert_lex_error("/* never closed", LexError::UnterminatedBlockComment);
        assert_lex_error("/* outer /* inner */ still unterminated", LexError::UnterminatedBlockComment);
    }

    #[test]
    fn inner_doc_comment_is_a_real_token() {
        let src = "//! module docs";
        assert_single(src, Token::InnerDocComment(src));
    }

    #[test]
    fn outer_doc_comment_is_a_real_token() {
        let src = "/// item docs";
        assert_single(src, Token::OuterDocComment(src));
    }

    #[test]
    fn doc_comments_are_distinguished_from_plain_comments() {
        assert_tokens(
            "//! inner\n/// outer\n// plain\nfn f() {}",
            &[
                Token::InnerDocComment("//! inner"),
                Token::OuterDocComment("/// outer"),
                Token::Fn,
                Token::Identifier("f"),
                Token::LParen,
                Token::RParen,
                Token::LBrace,
                Token::RBrace,
            ],
        );
    }
}

// ---------------------------------------------------------------------
// Larger synthetic snippets, assembled directly from grammar.md
// constructs not necessarily combined this way anywhere in the book.
// ---------------------------------------------------------------------

mod composite_constructs {
    use super::*;

    #[test]
    fn generic_trait_with_default_type_param_and_associated_type() {
        lex(r#"
            trait Add<Rhs = Self> {
                type Output;
                fn add(self, rhs: Rhs) -> Self::Output;
            }
        "#);
    }

    #[test]
    fn where_clause_generic_function() {
        lex(r#"
            fn process<T, U>(a: T, b: U) -> bool
            where
                T: PartialOrd + Display,
                U: Display,
            {
                true
            }
        "#);
    }

    #[test]
    fn full_pattern_matching_grammar() {
        lex(r#"
            match value {
                0 => "zero",
                1 | 2 | 3 => "small",
                4..=10 => "medium",
                n @ 11..=100 => print(n),
                Point { x: 0, y } => print(y),
                Point { x, .. } if x > 0 => "positive x",
                (a, b, _) => print(a + b),
                [first, second, ..] => print(first),
                Shape::Circle(radius) => print(radius),
                _ => "other",
            }
        "#);
    }

    #[test]
    fn let_else_and_while_let() {
        // Deliberately no loop label here — fig has no Rust-style
        // `'outer` syntax at all (fig labels are plain identifiers, with
        // no `'` sigil), so a bare `'` outside a char literal isn't valid
        // fig lexically either. Labels are covered separately below.
        lex(r#"
            let Some(x) = maybe else {
                return;
            };

            while let Some(item) = queue.pop() {
                process(item);
            }
        "#);
    }

    #[test]
    fn fig_style_plain_identifier_loop_label() {
        lex(r#"
            outer: for x in 0..5 {
                for y in 0..5 {
                    if x * y > 6 {
                        break outer;
                    }
                    continue outer;
                }
            }
        "#);
    }

    #[test]
    fn full_module_tree_with_visibility_and_reexport() {
        lex(r#"
            mod shapes {
                pub mod circle {
                    pub struct Circle { pub radius: float }
                }

                pub use circle::Circle;

                pub(super) fn helper() {}
            }

            use shapes::Circle;
            use shapes::{Circle as Round, circle};
            use shapes::*;
        "#);
    }

    #[test]
    fn ambient_module_with_bodyless_functions() {
        lex(r#"
            mod engine {
                pub struct Vector3 { x: float, y: float, z: float }

                impl Vector3 {
                    pub fn new(x: float, y: float, z: float) -> Vector3;
                    pub fn length(self) -> float;
                }

                pub fn delta_time() -> float;
            }
        "#);
    }

    #[test]
    fn struct_variants_named_tuple_unit() {
        lex(r#"
            struct Point { x: float, y: float }
            struct Pair(int, int);
            struct EntityId(pub int);
            struct Marker;
        "#);
    }

    #[test]
    fn enum_with_mixed_variant_shapes_and_generics() {
        lex(r#"
            enum Either<L, R> {
                Left(L),
                Right(R),
            }

            enum Shape {
                Circle(float),
                Rectangle { width: float, height: float },
                Point,
            }
        "#);
    }

    #[test]
    fn closures_all_forms() {
        lex(r#"
            let short = |x| x + 1;
            let typed = |x: int| -> int { x + 1 };
            let no_args = || 42;
            let multi_statement = |x: int| {
                let doubled = x * 2;
                doubled + 1
            };
        "#);
    }

    #[test]
    fn all_compound_assignment_operators_in_one_function() {
        lex(r#"
            fn f(mut x: int) {
                x += 1;
                x -= 1;
                x *= 2;
                x /= 2;
                x %= 2;
                x &= 1;
                x |= 1;
                x ^= 1;
                x <<= 1;
                x >>= 1;
            }
        "#);
    }

    #[test]
    fn casting_and_error_propagation() {
        lex(r#"
            fn load(path: String) -> Result<Config, String> {
                let text = read_file(path)?;
                let ratio = count as float / total as float;
                Result::Ok(parse(text)?)
            }
        "#);
    }

    #[test]
    fn gradual_typing_any_and_unannotated_positions() {
        lex(r#"
            let mut selected: any = "";
            fn describe(name: String, value) -> any {
                print(value);
                name
            }
            struct Config {
                max_retries: int,
                payload,
                label: any,
            }
        "#);
    }

    #[test]
    fn arrays_tuples_and_indexing() {
        lex(r#"
            let numbers: [int] = [1, 2, 3];
            let point: (int, int) = (3, 4);
            let mixed = (1, "two", 3.0);
            numbers[0] = 99;
            let x = point.0;
            let nested: [[int]] = [[1, 2], [3, 4]];
        "#);
    }

    #[test]
    fn operator_overloading_impl_with_explicit_rhs() {
        lex(r#"
            impl Mul<float> for Vector2 {
                type Output = Vector2;

                fn mul(self, rhs: float) -> Vector2 {
                    Vector2 { x: self.x * rhs, y: self.y * rhs }
                }
            }
        "#);
    }

    #[test]
    fn a_whole_tiny_program_top_to_bottom() {
        lex(r#"
            //! A tiny synthetic program exercising most of the grammar
            //! summary in one pass.

            enum Option<T> {
                Some(T),
                None,
            }

            trait Shape {
                fn area(self) -> float;

                fn describe(self) -> String {
                    "a shape"
                }
            }

            struct Circle { radius: float }

            impl Shape for Circle {
                fn area(self) -> float {
                    3.14159 * self.radius * self.radius
                }
            }

            fn largest<T: Shape>(shapes: [T]) -> Option<T> {
                if shapes.len() == 0 {
                    return Option::None;
                }
                let mut best = shapes[0];
                for shape in shapes {
                    if shape.area() > best.area() {
                        best = shape;
                    }
                }
                Option::Some(best)
            }

            let circles = [Circle { radius: 1.0 }, Circle { radius: 2.5 }];
            match largest(circles) {
                Option::Some(c) => print(c.describe()),
                Option::None => print("none"),
            }
        "#);
    }
}
