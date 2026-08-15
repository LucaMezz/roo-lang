//! Roo's `#[...]`/`#![...]` annotation system (`Annotation`/`MetaItem`).

use crate::*;
use ast::*;
use lexer::Token;

fn meta_item_list<'src>(
    meta_item: impl RooParser<'src, MetaItem> + 'src,
) -> impl RooParser<'src, MetaItemKind> {
    choice((
        meta_item.map(MetaItemInner::MetaItem),
        literal().map(MetaItemInner::Lit),
    ))
    .separated_by(just(Token::Comma))
    .allow_trailing()
    .collect::<Vec<_>>()
    .delimited_by(just(Token::LParen), just(Token::RParen))
    .map(MetaItemKind::List)
}

fn meta_item_name_value<'src>() -> impl RooParser<'src, MetaItemKind> {
    just(Token::Eq)
        .ignore_then(literal())
        .map(MetaItemKind::NameValue)
}

pub fn meta_item<'src>() -> impl RooParser<'src, MetaItem> {
    recursive(|meta_item| {
        path(ty())
            .then(choice((
                meta_item_list(meta_item),
                meta_item_name_value(),
                empty().to(MetaItemKind::Word),
            )))
            .map_with(|(path, kind), e| MetaItem {
                path,
                kind,
                span: span(e),
            })
    })
}

pub fn annotation<'src>() -> impl RooParser<'src, Annotation> {
    just(Token::Pound)
        .ignore_then(choice((
            just(Token::Bang).map(|_| AnnotationStyle::Inner),
            empty().map(|_| AnnotationStyle::Outer),
        )))
        .then(meta_item().delimited_by(just(Token::LBracket), just(Token::RBracket)))
        .map(|(style, item)| Annotation { style, item })
}

pub fn annotations<'src>() -> impl RooParser<'src, AnnotationVec> {
    annotation().repeated().collect::<Vec<_>>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::tokens;

    #[test]
    fn parses_a_word_meta_item() {
        let tokens = tokens("component");
        let parsed = meta_item()
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.path.segments[0].ident.name, "component");
        assert!(matches!(parsed.kind, MetaItemKind::Word));
    }

    #[test]
    fn parses_a_name_value_meta_item() {
        let tokens = tokens(r#"audio_cue = "jump""#);
        let parsed = meta_item()
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.path.segments[0].ident.name, "audio_cue");
        let MetaItemKind::NameValue(lit) = parsed.kind else {
            panic!("expected MetaItemKind::NameValue, got {:?}", parsed.kind);
        };
        assert_eq!(lit.kind, LitKind::Str("jump".to_owned()));
    }

    #[test]
    fn parses_a_recursively_nested_list_meta_item() {
        // The exact shape used in examples/ecs.roo's `#[replicated(...)]`:
        // a list containing a name-value pair and a nested list, whose own
        // argument is a bare word.
        let tokens = tokens(r#"replicated(rename = "hp", skip_if(default))"#);
        let parsed = meta_item()
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.path.segments[0].ident.name, "replicated");

        let MetaItemKind::List(items) = parsed.kind else {
            panic!("expected MetaItemKind::List, got {:?}", parsed.kind);
        };
        assert_eq!(items.len(), 2);

        let MetaItemInner::MetaItem(rename) = &items[0] else {
            panic!("expected a nested MetaItem, got {:?}", items[0]);
        };
        assert_eq!(rename.path.segments[0].ident.name, "rename");
        assert!(matches!(rename.kind, MetaItemKind::NameValue(_)));

        let MetaItemInner::MetaItem(skip_if) = &items[1] else {
            panic!("expected a nested MetaItem, got {:?}", items[1]);
        };
        assert_eq!(skip_if.path.segments[0].ident.name, "skip_if");
        let MetaItemKind::List(inner_items) = &skip_if.kind else {
            panic!("expected a nested List, got {:?}", skip_if.kind);
        };
        assert_eq!(inner_items.len(), 1);
        let MetaItemInner::MetaItem(default_arg) = &inner_items[0] else {
            panic!("expected a nested MetaItem, got {:?}", inner_items[0]);
        };
        assert_eq!(default_arg.path.segments[0].ident.name, "default");
        assert!(matches!(default_arg.kind, MetaItemKind::Word));
    }

    #[test]
    fn parses_an_outer_annotation() {
        let tokens = tokens("#[component]");
        let parsed = annotation()
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert!(matches!(parsed.style, AnnotationStyle::Outer));
        assert_eq!(parsed.item.path.segments[0].ident.name, "component");
    }

    #[test]
    fn parses_an_inner_annotation() {
        let tokens = tokens("#![replicated]");
        let parsed = annotation()
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert!(matches!(parsed.style, AnnotationStyle::Inner));
        assert_eq!(parsed.item.path.segments[0].ident.name, "replicated");
    }

    #[test]
    fn parses_the_exact_annotations_used_in_the_ecs_example() {
        let cases = [
            "#[component]",
            r#"#[replicated(rename = "hp", skip_if(default))]"#,
            "#[range(min = 0, max = 100)]",
            "#[must_use]",
            "#[system]",
            r#"#[audio_cue = "jump"]"#,
        ];
        for src in cases {
            let tokens = tokens(src);
            annotation()
                .parse(tokens)
                .into_result()
                .unwrap_or_else(|e| panic!("failed to parse {src:?}: {e:?}"));
        }
    }

    #[test]
    fn rejects_an_annotation_missing_its_brackets() {
        let tokens = tokens("#component");
        assert!(annotation().parse(tokens).into_result().is_err());
    }
}
