//! Runs fig-lang's type checker (resolve -> lower_signatures -> check)
//! over `example.fig` and reports the result.

use chumsky::Parser;
use typecheck::TypeCheckContext;

fn main() {
    let source = include_str!("example.fig");

    let tokens = lexer::tokenize_all(source).expect("failed to lex example.fig");
    let items = parser::module()
        .parse(parser::input(tokens))
        .into_result()
        .expect("failed to parse example.fig");

    let mut cx = TypeCheckContext::new();
    cx.resolve(&items);
    cx.lower_signatures(&items);
    cx.check(&items);

    println!(
        "type checked {} top-level item(s) from example.fig",
        items.len()
    );

    let diagnostics = cx.diagnostics();
    println!("{} diagnostic(s):", diagnostics.len());
    for diagnostic in diagnostics {
        println!("{diagnostic:#?}");
    }
}
