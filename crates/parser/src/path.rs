//! Paths, path segments, generics, where-clauses, and qualified-self
//! (`QSelf`) parsing — the mutually recursive `Ty`/`Path`/`GenericArgs`
//! cluster lives here.

use crate::*;
use ast::*;
use lexer::Token;

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

pub fn turbofish_generic_args<'src>(
    ty: impl FigParser<'src, Ty> + 'src,
) -> impl FigParser<'src, GenericArgs> {
    just(Token::PathSep).ignore_then(generic_args(ty))
}

pub fn assoc_item_constraint<'src>(
    ty: impl FigParser<'src, Ty> + 'src,
) -> impl FigParser<'src, AssocItemConstraint> {
    assoc_item_constraint_with(ty.clone(), generic_arg(ty))
}

fn path_segment_with<'src>(
    generic_args: impl FigParser<'src, GenericArgs> + 'src,
) -> impl FigParser<'src, PathSegment> {
    ident()
        .then(generic_args.or_not())
        .map(|(ident, args)| PathSegment { ident, args })
}

pub fn path_segment<'src>(
    ty: impl FigParser<'src, Ty> + 'src,
) -> impl FigParser<'src, PathSegment> {
    path_segment_with(generic_args(ty))
}

pub fn path_segment_turbofish<'src>(
    ty: impl FigParser<'src, Ty> + 'src,
) -> impl FigParser<'src, PathSegment> {
    path_segment_with(turbofish_generic_args(ty))
}

fn path_with<'src>(
    path_segment: impl FigParser<'src, PathSegment> + 'src,
) -> impl FigParser<'src, Path> {
    path_segment
        .separated_by(just(Token::PathSep))
        .at_least(1)
        .collect::<Vec<_>>()
        .map_with(|segments, e| Path {
            segments,
            span: span(e),
        })
}

pub fn path<'src>(ty: impl FigParser<'src, Ty> + 'src) -> impl FigParser<'src, Path> {
    path_with(path_segment(ty))
}

pub fn path_turbofish<'src>(ty: impl FigParser<'src, Ty> + 'src) -> impl FigParser<'src, Path> {
    path_with(path_segment_turbofish(ty))
}

pub fn generic_bounds<'src>() -> impl FigParser<'src, GenericBounds> {
    ty().separated_by(just(Token::Plus)).collect::<Vec<_>>()
}

pub fn generic_param<'src>() -> impl FigParser<'src, GenericParam> {
    annotations()
        .then(ident())
        .then(
            just(Token::Colon)
                .ignore_then(generic_bounds())
                .or_not()
                .map(Option::unwrap_or_default),
        )
        .then(just(Token::Eq).ignore_then(ty()).or_not())
        .map(|(((annotations, ident), bounds), default)| GenericParam {
            ident,
            bounds,
            default,
            annotations,
        })
}

pub fn where_predicate<'src>() -> impl FigParser<'src, WherePredicate> {
    ty().map(Box::new)
        .then_ignore(just(Token::Colon))
        .then(generic_bounds())
        .map(|(bounded_ty, bounds)| WherePredicate { bounded_ty, bounds })
}

pub fn where_clause<'src>() -> impl FigParser<'src, WhereClause> {
    where_predicate()
        .separated_by(just(Token::Comma))
        .collect::<Vec<_>>()
        .map_with(|predicates, e| WhereClause {
            predicates,
            span: span(e),
        })
}

pub fn generics<'src>() -> impl FigParser<'src, Generics> {
    generic_param()
        .separated_by(just(Token::Comma))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::Lt), just(Token::Gt))
        .or_not()
        .map(|params| params.unwrap_or_default())
        .then(just(Token::Where).ignore_then(where_clause()).or_not())
        .map_with(|(params, where_clause), e| Generics {
            params,
            where_clause: where_clause.unwrap_or_else(|| WhereClause {
                predicates: Vec::new(),
                span: span(e),
            }),
            span: span(e),
        })
}

pub fn q_self<'src>() -> impl FigParser<'src, QSelf> {
    ty().map(Box::new)
        .then_ignore(just(Token::As))
        .then(path(ty()).or_not())
        .delimited_by(just(Token::Lt), just(Token::Gt))
        .map(|(ty, trait_path)| QSelf { ty, trait_path })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::tokens;

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
    fn parses_a_plain_generic_param_with_no_annotations() {
        let tokens = tokens("T");
        let parsed = generic_param()
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.ident.name, "T");
        assert!(parsed.annotations.is_empty());
    }

    #[test]
    fn parses_an_annotated_generic_param() {
        let tokens = tokens("#[opaque] T");
        let parsed = generic_param()
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.ident.name, "T");
        assert_eq!(parsed.annotations.len(), 1);
        assert_eq!(
            parsed.annotations[0].item.path.segments[0].ident.name,
            "opaque"
        );
    }

    #[test]
    fn parses_a_generic_param_with_stacked_annotations() {
        let tokens = tokens("#[a] #[b] T");
        let parsed = generic_param()
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.annotations.len(), 2);
    }

    #[test]
    fn parses_a_generic_param_with_a_bound() {
        let tokens = tokens("T: Display");
        let parsed = generic_param()
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.bounds.len(), 1);
        assert!(parsed.default.is_none());
    }

    #[test]
    fn parses_a_generic_param_with_multiple_bounds() {
        let tokens = tokens("T: Display + Clone");
        let parsed = generic_param()
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.bounds.len(), 2);
    }

    #[test]
    fn parses_a_generic_param_with_a_default() {
        let tokens = tokens("T = int");
        let parsed = generic_param()
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert!(parsed.bounds.is_empty());
        assert!(parsed.default.is_some());
    }

    #[test]
    fn parses_a_generic_param_with_a_bound_and_a_default() {
        let tokens = tokens("T: Display = int");
        let parsed = generic_param()
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.bounds.len(), 1);
        assert!(parsed.default.is_some());
    }

    #[test]
    fn parses_a_where_predicate() {
        let tokens = tokens("T: Display + Clone");
        let parsed = where_predicate()
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.bounds.len(), 2);
    }

    #[test]
    fn rejects_a_where_predicate_missing_its_colon() {
        let tokens = tokens("T Display");
        assert!(where_predicate().parse(&tokens).into_result().is_err());
    }

    #[test]
    fn parses_empty_generics_when_absent() {
        let tokens = tokens("");
        let parsed = generics()
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert!(parsed.params.is_empty());
        assert!(parsed.where_clause.predicates.is_empty());
    }

    #[test]
    fn parses_generics_with_params_only() {
        let tokens = tokens("<T, U: Display>");
        let parsed = generics()
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.params.len(), 2);
        assert!(parsed.where_clause.predicates.is_empty());
    }

    #[test]
    fn parses_generics_with_a_trailing_where_clause() {
        let tokens = tokens("<T> where T: Display");
        let parsed = generics()
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.params.len(), 1);
        assert_eq!(parsed.where_clause.predicates.len(), 1);
    }

    #[test]
    fn parses_a_bare_where_clause_with_no_generic_params() {
        let tokens = tokens("where T: Display");
        let parsed = generics()
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert!(parsed.params.is_empty());
        assert_eq!(parsed.where_clause.predicates.len(), 1);
    }

    #[test]
    fn parses_a_turbofish_path_segment() {
        let tokens = tokens("Vec::<int>");
        let parsed = path_segment_turbofish(ty())
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.ident.name, "Vec");
        assert_eq!(parsed.args.expect("should have generic args").args.len(), 1);
    }

    #[test]
    fn rejects_bare_generics_on_a_turbofish_path_segment() {
        // The whole point: `Vec<int>` (no `::`) must NOT be accepted in
        // expression position, since `<` is the less-than operator there.
        let tokens = tokens("Vec<int>");
        assert!(
            path_segment_turbofish(ty())
                .parse(&tokens)
                .into_result()
                .is_err()
        );
    }

    #[test]
    fn parses_a_turbofish_path_segment_with_no_generics() {
        // No `::<...>` at all is still just a plain segment.
        let tokens = tokens("foo");
        let parsed = path_segment_turbofish(ty())
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.ident.name, "foo");
        assert!(parsed.args.is_none());
    }

    #[test]
    fn parses_a_multi_segment_turbofish_path_without_generics() {
        // Ordinary "::" separators between segments must keep working
        // exactly as before — only a "::" immediately followed by "<" is
        // ever turbofish; `path_segment_turbofish`'s attempt to treat the
        // separator as turbofish has to cleanly fail and backtrack here.
        let tokens = tokens("foo::bar::baz");
        let parsed = path_turbofish(ty())
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
    fn parses_a_turbofish_path_with_generics_mid_path() {
        let tokens = tokens("Vec::<int>::new");
        let parsed = path_turbofish(ty())
            .parse(&tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.segments.len(), 2);
        assert_eq!(parsed.segments[0].ident.name, "Vec");
        assert_eq!(
            parsed.segments[0]
                .args
                .as_ref()
                .expect("should have generic args")
                .args
                .len(),
            1
        );
        assert_eq!(parsed.segments[1].ident.name, "new");
        assert!(parsed.segments[1].args.is_none());
    }
}
