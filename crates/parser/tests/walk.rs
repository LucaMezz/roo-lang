use ast::visit::Visitor;
use chumsky::Parser;

#[derive(Default)]
struct IdentCollector {
    names: Vec<String>,
}

impl Visitor for IdentCollector {
    fn visit_ident(&mut self, ident: &ast::Ident) {
        self.names.push(ident.name.clone());
    }
}

#[test]
fn walks_a_whole_fn_item_and_collects_every_ident() {
    let source = "fn add(a: int, b: int) -> int { a + b }";
    let tokens = lexer::tokenize_all(source).expect("should lex");
    let item = parser::item()
        .parse(parser::input(tokens))
        .into_result()
        .expect("should parse");

    let mut collector = IdentCollector::default();
    collector.visit_item(&item);

    assert_eq!(
        collector.names,
        vec!["add", "int", "a", "int", "b", "int", "a", "b"]
    );
}
