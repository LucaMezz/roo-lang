use crate::*;
use ast::*;
use lexer::Token;

pub fn module_body<'src>(
    item: impl FigParser<'src, Item> + 'src,
) -> impl FigParser<'src, Vec<Box<Item>>> {
    item.map(Box::new).repeated().collect::<Vec<_>>()
}

pub fn mod_kind<'src>(item: impl FigParser<'src, Item> + 'src) -> impl FigParser<'src, ModKind> {
    choice((
        just(Token::Semi).to(ModKind::Unloaded),
        module_body(item)
            .delimited_by(just(Token::LBrace), just(Token::RBrace))
            .map(ModKind::Loaded),
    ))
}

pub fn module<'src>() -> impl FigParser<'src, Vec<Box<Item>>> {
    module_body(item())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::tokens;

    #[test]
    fn parses_an_empty_module() {
        let tokens = tokens("");
        let parsed = module().parse(&tokens).into_result().expect("should parse");
        assert!(parsed.is_empty());
    }

    #[test]
    fn parses_a_module_with_multiple_items() {
        let tokens = tokens("struct A; fn b() {} use c::d;");
        let parsed = module().parse(&tokens).into_result().expect("should parse");
        assert_eq!(parsed.len(), 3);
    }

    #[test]
    fn rejects_trailing_garbage_after_the_last_item() {
        let tokens = tokens("struct A; )");
        assert!(module().parse(&tokens).into_result().is_err());
    }
}
