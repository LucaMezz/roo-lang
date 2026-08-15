use crate::*;
use ast::*;
use lexer::Token;

pub fn use_tree<'src>() -> impl FigParser<'src, UseTree> {
    recursive(|use_tree| {
        path(ty())
            .then(choice((
                just(Token::PathSep)
                    .ignore_then(
                        use_tree
                            .separated_by(just(Token::Comma))
                            .allow_trailing()
                            .collect::<Vec<_>>()
                            .delimited_by(just(Token::LBrace), just(Token::RBrace)),
                    )
                    .map_with(|items, e| UseTreeKind::Nested {
                        items,
                        span: span(e),
                    }),
                just(Token::PathSep)
                    .ignore_then(just(Token::Star))
                    .map_with(|_, e| UseTreeKind::Glob(span(e))),
                just(Token::As)
                    .ignore_then(ident())
                    .or_not()
                    .map(UseTreeKind::Simple),
            )))
            .map(|(prefix, kind)| UseTree { prefix, kind })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::tokens;

    #[test]
    fn parses_a_simple_use() {
        let tokens = tokens("foo");
        let parsed = use_tree()
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.prefix.segments.len(), 1);
        assert!(matches!(parsed.kind, UseTreeKind::Simple(None)));
    }

    #[test]
    fn parses_a_renamed_use() {
        let tokens = tokens("foo as bar");
        let parsed = use_tree()
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let UseTreeKind::Simple(Some(rename)) = parsed.kind else {
            panic!("expected UseTreeKind::Simple(Some(_))");
        };
        assert_eq!(rename.name, "bar");
    }

    #[test]
    fn parses_a_multi_segment_prefix() {
        let tokens = tokens("foo::bar");
        let parsed = use_tree()
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.prefix.segments.len(), 2);
        assert!(matches!(parsed.kind, UseTreeKind::Simple(None)));
    }

    #[test]
    fn parses_a_glob_use() {
        let tokens = tokens("foo::*");
        let parsed = use_tree()
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.prefix.segments.len(), 1);
        assert!(matches!(parsed.kind, UseTreeKind::Glob(_)));
    }

    #[test]
    fn parses_a_nested_use_group() {
        let tokens = tokens("foo::{a, b}");
        let parsed = use_tree()
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let UseTreeKind::Nested { items, .. } = parsed.kind else {
            panic!("expected UseTreeKind::Nested");
        };
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].prefix.segments[0].ident.name, "a");
        assert_eq!(items[1].prefix.segments[0].ident.name, "b");
    }

    #[test]
    fn parses_a_nested_group_with_varied_items() {
        let tokens = tokens("foo::{a::b, c::*, d as e}");
        let parsed = use_tree()
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let UseTreeKind::Nested { items, .. } = parsed.kind else {
            panic!("expected UseTreeKind::Nested");
        };
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].prefix.segments.len(), 2);
        assert!(matches!(items[0].kind, UseTreeKind::Simple(None)));
        assert!(matches!(items[1].kind, UseTreeKind::Glob(_)));
        let UseTreeKind::Simple(Some(ref rename)) = items[2].kind else {
            panic!("expected UseTreeKind::Simple(Some(_))");
        };
        assert_eq!(rename.name, "e");
    }

    #[test]
    fn parses_a_nested_group_containing_another_nested_group() {
        let tokens = tokens("foo::{a, b::{c, d}}");
        let parsed = use_tree()
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let UseTreeKind::Nested { items, .. } = parsed.kind else {
            panic!("expected UseTreeKind::Nested");
        };
        assert_eq!(items.len(), 2);
        let UseTreeKind::Nested { items: inner, .. } = &items[1].kind else {
            panic!("expected a nested UseTreeKind::Nested");
        };
        assert_eq!(inner.len(), 2);
    }

    #[test]
    fn parses_a_trailing_comma_in_a_nested_group() {
        let tokens = tokens("foo::{a, b,}");
        let parsed = use_tree()
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let UseTreeKind::Nested { items, .. } = parsed.kind else {
            panic!("expected UseTreeKind::Nested");
        };
        assert_eq!(items.len(), 2);
    }
}
