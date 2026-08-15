//! Patterns (`Pat`/`PatKind`).

use crate::*;
use ast::*;
use lexer::Token;

pub fn pat_field<'src>(pat: impl RooParser<'src, Pat> + 'src) -> impl RooParser<'src, PatField> {
    let full = ident()
        .then_ignore(just(Token::Colon))
        .then(pat)
        .map(|(ident, pattern)| (ident, Box::new(pattern)));

    let shorthand = ident().map_with(|ident, e| {
        let pat = Box::new(Pat {
            kind: PatKind::Ident(ident.clone(), None),
            span: span(e),
        });
        (ident, pat)
    });

    annotations()
        .then(full.or(shorthand))
        .map_with(|(annotations, (ident, pat)), e| PatField {
            ident,
            pat,
            annotations,
            span: span(e),
        })
}

fn pat_wild<'src>() -> impl RooParser<'src, PatKind> {
    just(Token::Identifier("_")).map(|_| PatKind::Wild)
}

fn pat_rest<'src>() -> impl RooParser<'src, PatKind> {
    just(Token::DotDot).map(|_| PatKind::Rest)
}

fn pat_never<'src>() -> impl RooParser<'src, PatKind> {
    just(Token::Bang).map(|_| PatKind::Never)
}

fn pat_tuple_or_paren<'src>(
    pat: impl RooParser<'src, Pat> + 'src,
) -> impl RooParser<'src, PatKind> {
    pat.clone()
        .then_ignore(just(Token::Comma))
        .repeated()
        .collect::<Vec<_>>()
        .then(pat.or_not())
        .delimited_by(just(Token::LParen), just(Token::RParen))
        .map(|(mut elems, last)| match (elems.is_empty(), last) {
            (true, Some(only)) => PatKind::Paren(Box::new(only)),
            (_, last) => {
                elems.extend(last);
                PatKind::Tuple(elems)
            }
        })
}

fn pat_array<'src>(pat: impl RooParser<'src, Pat> + 'src) -> impl RooParser<'src, PatKind> {
    pat.separated_by(just(Token::Comma))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::LBracket), just(Token::RBracket))
        .map(PatKind::Array)
}

fn pat_tuple_struct<'src>(pat: impl RooParser<'src, Pat> + 'src) -> impl RooParser<'src, PatKind> {
    q_self()
        .map(Box::new)
        .or_not()
        .then(path(ty()))
        .then(
            pat.separated_by(just(Token::Comma))
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LParen), just(Token::RParen)),
        )
        .map(|((qself, path), elems)| PatKind::TupleStruct(qself, path, elems))
}

fn pat_struct<'src>(pat: impl RooParser<'src, Pat> + 'src) -> impl RooParser<'src, PatKind> {
    q_self()
        .map(Box::new)
        .or_not()
        .then(path(ty()))
        .then(
            pat_field(pat)
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .collect::<Vec<_>>()
                .then(just(Token::DotDot).map_with(|_, e| span(e)).or_not())
                .delimited_by(just(Token::LBrace), just(Token::RBrace)),
        )
        .map(|((qself, path), (fields, rest))| PatKind::Struct(qself, path, fields, rest))
}

fn pat_path<'src>() -> impl RooParser<'src, PatKind> {
    q_self()
        .map(Box::new)
        .or_not()
        .then(path(ty()))
        .filter(|(qself, path)| qself.is_some() || path.segments.len() > 1)
        .map(|(qself, path)| PatKind::Path(qself, path))
}

fn pat_ident<'src>(pat: impl RooParser<'src, Pat> + 'src) -> impl RooParser<'src, PatKind> {
    ident()
        .then(just(Token::At).ignore_then(pat).map(Box::new).or_not())
        .map(|(ident, sub)| PatKind::Ident(ident, sub))
}

fn pat_expr_or_range<'src>(
    expr: impl RooParser<'src, Expr> + 'src,
) -> impl RooParser<'src, PatKind> {
    let range_end = choice((
        just(Token::DotDotEq).map(|_| RangeEnd::Included),
        just(Token::DotDot).map(|_| RangeEnd::Excluded),
    ));

    let no_start = range_end
        .clone()
        .then(expr.clone())
        .map_with(|(end_kind, end), e| {
            PatKind::Range(None, Some(Box::new(end)), end_kind, span(e))
        });

    let with_start = expr
        .clone()
        .then(range_end.then(expr.or_not()).or_not())
        .map_with(|(start, tail), e| match tail {
            Some((end_kind, end)) => {
                PatKind::Range(Some(Box::new(start)), end.map(Box::new), end_kind, span(e))
            }
            None => PatKind::Expr(Box::new(start)),
        });

    choice((no_start, with_start))
}

fn pat_kind<'src>(
    pat: impl RooParser<'src, Pat> + 'src,
    expr: impl RooParser<'src, Expr> + 'src,
) -> impl RooParser<'src, PatKind> {
    choice((
        pat_wild(),
        pat_never(),
        pat_tuple_struct(pat.clone()),
        pat_struct(pat.clone()),
        pat_tuple_or_paren(pat.clone()),
        pat_array(pat.clone()),
        pat_path(),
        pat_ident(pat),
        pat_expr_or_range(expr),
        pat_rest(),
    ))
}

/// A single pattern with no top-level `|` or-alternation — required in
/// closure-parameter position (`|a, b| ...`), where a bare `|` right
/// after the last parameter is ambiguous between "another or-pattern
/// alternative" and "closing the parameter list" (real Rust has the
/// identical `PatNoTopAlt`-vs-`Pat` restriction, for the same reason).
/// Nested positions (tuple/struct/array elements, parenthesized
/// sub-patterns, ...) still go through the ordinary or-accepting
/// `pat()`, since only the outermost level next to the closing `|` is
/// actually ambiguous.
pub fn pat_no_top_alt<'src>(expr: impl RooParser<'src, Expr> + 'src) -> impl RooParser<'src, Pat> {
    pat_kind(pat(expr.clone()), expr).map_with(|kind, e| Pat {
        kind,
        span: span(e),
    })
}

pub fn pat<'src>(expr: impl RooParser<'src, Expr> + 'src) -> impl RooParser<'src, Pat> {
    recursive(|pat| {
        pat_kind(pat, expr)
            .map_with(|kind, e| Pat {
                kind,
                span: span(e),
            })
            .separated_by(just(Token::Pipe))
            .at_least(1)
            .collect::<Vec<_>>()
            .map_with(|mut pats, e| {
                if pats.len() == 1 {
                    pats.pop().unwrap()
                } else {
                    Pat {
                        kind: PatKind::Or(pats),
                        span: span(e),
                    }
                }
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::tokens;

    // `pat()` doesn't exist yet, so `pat_field()`'s tests stand in a
    // minimal placeholder pattern parser just to exercise pat_field's
    // own logic (annotations, shorthand vs `name: pattern`) in
    // isolation.
    fn dummy_pat<'src>() -> impl RooParser<'src, Pat> {
        ident().map_with(|ident, e| Pat {
            kind: PatKind::Ident(ident, None),
            span: span(e),
        })
    }

    // `expr()` doesn't exist yet either — `pat_expr_or_range`'s tests
    // stand in a minimal literal-only placeholder, since real pattern
    // range/expr positions are almost always literals anyway.
    fn dummy_expr<'src>() -> impl RooParser<'src, Expr> {
        literal().map_with(|lit, e| Expr {
            kind: ExprKind::Lit(lit),
            span: span(e),
            annotations: Vec::new(),
        })
    }

    #[test]
    fn parses_a_shorthand_pat_field() {
        let tokens = tokens("x");
        let parsed = pat_field(dummy_pat())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.ident.name, "x");
        assert!(parsed.annotations.is_empty());
        let PatKind::Ident(ident, sub) = parsed.pat.kind else {
            panic!("expected PatKind::Ident, got {:?}", parsed.pat.kind);
        };
        assert_eq!(ident.name, "x");
        assert!(sub.is_none());
    }

    #[test]
    fn parses_a_named_pat_field() {
        let tokens = tokens("x: y");
        let parsed = pat_field(dummy_pat())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.ident.name, "x");
        let PatKind::Ident(ident, _) = parsed.pat.kind else {
            panic!("expected PatKind::Ident, got {:?}", parsed.pat.kind);
        };
        assert_eq!(ident.name, "y");
    }

    #[test]
    fn parses_an_annotated_pat_field() {
        let tokens = tokens("#[foo] x: y");
        let parsed = pat_field(dummy_pat())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.annotations.len(), 1);
        assert_eq!(
            parsed.annotations[0].item.path.segments[0].ident.name,
            "foo"
        );
    }

    #[test]
    fn parses_an_annotated_shorthand_pat_field() {
        let tokens = tokens("#[foo] x");
        let parsed = pat_field(dummy_pat())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.annotations.len(), 1);
        assert!(matches!(parsed.pat.kind, PatKind::Ident(..)));
    }

    #[test]
    fn parses_an_empty_struct_pattern() {
        let tokens = tokens("Point {}");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let PatKind::Struct(qself, path, fields, rest) = parsed.kind else {
            panic!("expected PatKind::Struct, got {:?}", parsed.kind);
        };
        assert!(qself.is_none());
        assert_eq!(path.segments[0].ident.name, "Point");
        assert!(fields.is_empty());
        assert!(rest.is_none());
    }

    #[test]
    fn parses_a_struct_pattern_with_shorthand_fields() {
        let tokens = tokens("Point { x, y }");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let PatKind::Struct(_, _, fields, rest) = parsed.kind else {
            panic!("expected PatKind::Struct, got {:?}", parsed.kind);
        };
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].ident.name, "x");
        assert_eq!(fields[1].ident.name, "y");
        assert!(rest.is_none());
    }

    #[test]
    fn parses_a_struct_pattern_with_a_trailing_comma() {
        let tokens = tokens("Point { x, y, }");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let PatKind::Struct(_, _, fields, _) = parsed.kind else {
            panic!("expected PatKind::Struct, got {:?}", parsed.kind);
        };
        assert_eq!(fields.len(), 2);
    }

    #[test]
    fn parses_a_struct_pattern_with_a_rest_marker() {
        let tokens = tokens("Point { x, .. }");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let PatKind::Struct(_, _, fields, rest) = parsed.kind else {
            panic!("expected PatKind::Struct, got {:?}", parsed.kind);
        };
        assert_eq!(fields.len(), 1);
        assert!(rest.is_some());
    }

    #[test]
    fn parses_a_struct_pattern_that_is_only_a_rest_marker() {
        let tokens = tokens("Point { .. }");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let PatKind::Struct(_, _, fields, rest) = parsed.kind else {
            panic!("expected PatKind::Struct, got {:?}", parsed.kind);
        };
        assert!(fields.is_empty());
        assert!(rest.is_some());
    }

    #[test]
    fn parses_a_nested_struct_pattern() {
        // Proves the self-recursion actually works: a struct pattern's
        // own field can itself be a full struct pattern.
        let tokens = tokens("Outer { inner: Inner { x } }");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let PatKind::Struct(_, outer_path, fields, _) = parsed.kind else {
            panic!("expected PatKind::Struct, got {:?}", parsed.kind);
        };
        assert_eq!(outer_path.segments[0].ident.name, "Outer");
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].ident.name, "inner");

        let PatKind::Struct(_, inner_path, inner_fields, _) = &fields[0].pat.kind else {
            panic!(
                "expected a nested PatKind::Struct, got {:?}",
                fields[0].pat.kind
            );
        };
        assert_eq!(inner_path.segments[0].ident.name, "Inner");
        assert_eq!(inner_fields.len(), 1);
        assert_eq!(inner_fields[0].ident.name, "x");
    }

    #[test]
    fn parses_an_annotated_field_inside_a_struct_pattern() {
        let tokens = tokens("Point { #[range(min = 0, max = 100)] x }");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let PatKind::Struct(_, _, fields, _) = parsed.kind else {
            panic!("expected PatKind::Struct, got {:?}", parsed.kind);
        };
        assert_eq!(fields[0].annotations.len(), 1);
    }

    #[test]
    fn parses_a_wild_pattern() {
        let tokens = tokens("_");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert!(matches!(parsed.kind, PatKind::Wild));
    }

    #[test]
    fn parses_a_rest_pattern() {
        let tokens = tokens("..");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert!(matches!(parsed.kind, PatKind::Rest));
    }

    #[test]
    fn parses_a_never_pattern() {
        let tokens = tokens("!");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert!(matches!(parsed.kind, PatKind::Never));
    }

    #[test]
    fn parses_a_parenthesized_pattern() {
        let tokens = tokens("(Point {})");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let PatKind::Paren(inner) = parsed.kind else {
            panic!("expected PatKind::Paren, got {:?}", parsed.kind);
        };
        assert!(matches!(inner.kind, PatKind::Struct(..)));
    }

    #[test]
    fn parses_a_doubly_parenthesized_pattern() {
        let tokens = tokens("((_))");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let PatKind::Paren(inner) = parsed.kind else {
            panic!("expected PatKind::Paren, got {:?}", parsed.kind);
        };
        assert!(matches!(inner.kind, PatKind::Paren(_)));
    }

    #[test]
    fn parses_a_plain_binding_pattern() {
        let tokens = tokens("x");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let PatKind::Ident(ident, sub) = parsed.kind else {
            panic!("expected PatKind::Ident, got {:?}", parsed.kind);
        };
        assert_eq!(ident.name, "x");
        assert!(sub.is_none());
    }

    #[test]
    fn parses_a_captured_pattern() {
        let tokens = tokens("n @ _");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let PatKind::Ident(ident, sub) = parsed.kind else {
            panic!("expected PatKind::Ident, got {:?}", parsed.kind);
        };
        assert_eq!(ident.name, "n");
        let sub = sub.expect("should have a sub-pattern");
        assert!(matches!(sub.kind, PatKind::Wild));
    }

    #[test]
    fn prefers_struct_pattern_over_ident_when_a_brace_follows() {
        // Same "more specific alternative first" hazard as generic_arg
        // and Fn types: pat_ident must not win just because it can match
        // the bare identifier and stop before `{ x }` is ever seen.
        let tokens = tokens("Point { x }");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert!(matches!(parsed.kind, PatKind::Struct(..)));
    }

    #[test]
    fn parses_a_tuple_struct_pattern() {
        let tokens = tokens("Some(x)");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let PatKind::TupleStruct(qself, path, elems) = parsed.kind else {
            panic!("expected PatKind::TupleStruct, got {:?}", parsed.kind);
        };
        assert!(qself.is_none());
        assert_eq!(path.segments[0].ident.name, "Some");
        assert_eq!(elems.len(), 1);
    }

    #[test]
    fn parses_an_empty_tuple_struct_pattern() {
        let tokens = tokens("Unit()");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let PatKind::TupleStruct(_, _, elems) = parsed.kind else {
            panic!("expected PatKind::TupleStruct, got {:?}", parsed.kind);
        };
        assert!(elems.is_empty());
    }

    #[test]
    fn parses_a_tuple_struct_pattern_with_a_multi_segment_path() {
        let tokens = tokens("foo::Bar(x)");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let PatKind::TupleStruct(_, path, elems) = parsed.kind else {
            panic!("expected PatKind::TupleStruct, got {:?}", parsed.kind);
        };
        assert_eq!(path.segments.len(), 2);
        assert_eq!(elems.len(), 1);
    }

    #[test]
    fn parses_a_multi_segment_path_pattern() {
        let tokens = tokens("Color::Red");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let PatKind::Path(qself, path) = parsed.kind else {
            panic!("expected PatKind::Path, got {:?}", parsed.kind);
        };
        assert!(qself.is_none());
        assert_eq!(path.segments.len(), 2);
    }

    #[test]
    fn parses_a_single_segment_as_ident_not_path() {
        // pat_path requires >1 segment (or a qself) — a bare name always
        // goes through pat_ident instead, since disambiguating "binding
        // vs. reference to a unit const/variant" needs name resolution.
        let tokens = tokens("Foo");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert!(matches!(parsed.kind, PatKind::Ident(..)));
    }

    #[test]
    fn parses_the_unit_pattern() {
        let tokens = tokens("()");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let PatKind::Tuple(elems) = parsed.kind else {
            panic!("expected PatKind::Tuple, got {:?}", parsed.kind);
        };
        assert!(elems.is_empty());
    }

    #[test]
    fn parses_a_parenthesized_pattern_as_paren_not_a_one_tuple() {
        let tokens = tokens("(x)");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert!(
            matches!(parsed.kind, PatKind::Paren(_)),
            "got {:?}",
            parsed.kind
        );
    }

    #[test]
    fn parses_a_trailing_comma_as_a_one_tuple_pattern() {
        let tokens = tokens("(x,)");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let PatKind::Tuple(elems) = parsed.kind else {
            panic!("expected PatKind::Tuple, got {:?}", parsed.kind);
        };
        assert_eq!(elems.len(), 1);
    }

    #[test]
    fn parses_a_multi_element_tuple_pattern() {
        let tokens = tokens("(x, y)");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let PatKind::Tuple(elems) = parsed.kind else {
            panic!("expected PatKind::Tuple, got {:?}", parsed.kind);
        };
        assert_eq!(elems.len(), 2);
    }

    #[test]
    fn parses_an_array_pattern() {
        let tokens = tokens("[x, y]");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let PatKind::Array(elems) = parsed.kind else {
            panic!("expected PatKind::Array, got {:?}", parsed.kind);
        };
        assert_eq!(elems.len(), 2);
    }

    #[test]
    fn parses_an_array_pattern_with_a_rest_marker() {
        let tokens = tokens("[first, .., last]");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let PatKind::Array(elems) = parsed.kind else {
            panic!("expected PatKind::Array, got {:?}", parsed.kind);
        };
        assert_eq!(elems.len(), 3);
        assert!(matches!(elems[1].kind, PatKind::Rest));
    }

    #[test]
    fn parses_a_bare_literal_as_an_expr_pattern() {
        let tokens = tokens("5");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert!(matches!(parsed.kind, PatKind::Expr(_)));
    }

    #[test]
    fn parses_a_range_pattern_with_start_and_end() {
        let tokens = tokens("1..5");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let PatKind::Range(start, end, range_end, _) = parsed.kind else {
            panic!("expected PatKind::Range, got {:?}", parsed.kind);
        };
        assert!(start.is_some());
        assert!(end.is_some());
        assert_eq!(range_end, RangeEnd::Excluded);
    }

    #[test]
    fn parses_an_inclusive_range_pattern() {
        let tokens = tokens("1..=5");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let PatKind::Range(_, _, range_end, _) = parsed.kind else {
            panic!("expected PatKind::Range, got {:?}", parsed.kind);
        };
        assert_eq!(range_end, RangeEnd::Included);
    }

    #[test]
    fn parses_a_range_pattern_with_only_a_start() {
        let tokens = tokens("1..");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let PatKind::Range(start, end, ..) = parsed.kind else {
            panic!("expected PatKind::Range, got {:?}", parsed.kind);
        };
        assert!(start.is_some());
        assert!(end.is_none());
    }

    #[test]
    fn parses_a_range_pattern_with_only_an_end() {
        let tokens = tokens("..5");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let PatKind::Range(start, end, ..) = parsed.kind else {
            panic!("expected PatKind::Range, got {:?}", parsed.kind);
        };
        assert!(start.is_none());
        assert!(end.is_some());
    }

    #[test]
    fn parses_a_boundless_dot_dot_as_rest_not_an_empty_range() {
        let tokens = tokens("..");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert!(matches!(parsed.kind, PatKind::Rest));
    }

    #[test]
    fn parses_an_or_pattern() {
        let tokens = tokens("1 | 2 | 3");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let PatKind::Or(alts) = parsed.kind else {
            panic!("expected PatKind::Or, got {:?}", parsed.kind);
        };
        assert_eq!(alts.len(), 3);
    }

    #[test]
    fn does_not_wrap_a_single_pattern_in_or() {
        let tokens = tokens("x");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert!(matches!(parsed.kind, PatKind::Ident(..)));
    }

    #[test]
    fn parses_an_or_pattern_nested_inside_parens() {
        // Proves Or-wrapping happens through the shared recursive `pat`
        // handle, not just at the top level.
        let tokens = tokens("(1 | 2)");
        let parsed = pat(dummy_expr())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let PatKind::Paren(inner) = parsed.kind else {
            panic!("expected PatKind::Paren, got {:?}", parsed.kind);
        };
        assert!(matches!(inner.kind, PatKind::Or(_)));
    }
}
