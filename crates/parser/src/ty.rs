//! Types (`Ty`/`TyKind`), including `Fn` types and function return types.

use crate::*;
use ast::*;
use lexer::Token;

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

pub fn fn_ret_ty<'src>(ty: impl FigParser<'src, Ty> + 'src) -> impl FigParser<'src, FnRetTy> {
    choice((
        just(Token::Arrow)
            .ignore_then(ty)
            .map(Box::new)
            .map(FnRetTy::Ty),
        empty().map_with(|_, e| FnRetTy::Default(span(e))),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::tokens;

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
