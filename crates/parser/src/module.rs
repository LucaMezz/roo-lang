use crate::*;
use ast::*;
use lexer::Token;

fn inner_doc_comments<'src>() -> impl RooParser<'src, ()> {
    select! { Token::InnerDocComment(_) => () }
        .repeated()
        .ignored()
}

pub fn module_body<'src>(
    item: impl RooParser<'src, Item> + 'src,
) -> impl RooParser<'src, Vec<Box<Item>>> {
    inner_doc_comments().ignore_then(item.map(Box::new).repeated().collect::<Vec<_>>())
}

pub fn mod_kind<'src>(item: impl RooParser<'src, Item> + 'src) -> impl RooParser<'src, ModKind> {
    choice((
        just(Token::Semi).to(ModKind::Unloaded),
        module_body(item)
            .delimited_by(just(Token::LBrace), just(Token::RBrace))
            .map(ModKind::Loaded),
    ))
}

pub fn module<'src>() -> impl RooParser<'src, Vec<Box<Item>>> {
    module_body(item())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::tokens;

    #[test]
    fn parses_an_empty_module() {
        let tokens = tokens("");
        let parsed = module().parse(tokens).into_result().expect("should parse");
        assert!(parsed.is_empty());
    }

    #[test]
    fn parses_a_module_with_multiple_items() {
        let tokens = tokens("struct A; fn b() {} use c::d;");
        let parsed = module().parse(tokens).into_result().expect("should parse");
        assert_eq!(parsed.len(), 3);
    }

    #[test]
    fn rejects_trailing_garbage_after_the_last_item() {
        let tokens = tokens("struct A; )");
        assert!(module().parse(tokens).into_result().is_err());
    }

    #[test]
    fn parses_leading_inner_doc_comments() {
        let tokens = tokens("//! a module\n//! across two lines\nstruct A;");
        let parsed = module().parse(tokens).into_result().expect("should parse");
        assert_eq!(parsed.len(), 1);
    }
}
