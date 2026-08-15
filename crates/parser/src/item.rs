use crate::*;
use ast::*;
use lexer::Token;

fn named_field<'src>(ty: impl FigParser<'src, Ty> + 'src) -> impl FigParser<'src, FieldDef> {
    annotations()
        .then(visibility())
        .then(ident())
        .then(just(Token::Colon).ignore_then(ty).map(Box::new).or_not())
        .map_with(|(((annotations, vis), ident), ty), e| FieldDef {
            annotations,
            span: span(e),
            vis,
            ident: Some(ident),
            ty,
        })
}

fn tuple_field<'src>(ty: impl FigParser<'src, Ty> + 'src) -> impl FigParser<'src, FieldDef> {
    annotations()
        .then(visibility())
        .then(ty)
        .map_with(|((annotations, vis), ty), e| FieldDef {
            annotations,
            span: span(e),
            vis,
            ident: None,
            ty: Some(Box::new(ty)),
        })
}

fn enum_tuple_field<'src>(ty: impl FigParser<'src, Ty> + 'src) -> impl FigParser<'src, FieldDef> {
    annotations()
        .then(ty)
        .map_with(|(annotations, ty), e| FieldDef {
            annotations,
            span: span(e),
            vis: Visibility {
                kind: VisibilityKind::Inherited,
                span: span(e),
            },
            ident: None,
            ty: Some(Box::new(ty)),
        })
}

fn struct_body<'src>(ty: impl FigParser<'src, Ty> + 'src) -> impl FigParser<'src, VariantData> {
    choice((
        named_field(ty.clone())
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::LBrace), just(Token::RBrace))
            .map(VariantData::Struct),
        tuple_field(ty)
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::LParen), just(Token::RParen))
            .then_ignore(just(Token::Semi))
            .map(VariantData::Tuple),
        just(Token::Semi).to(VariantData::Unit),
    ))
}

fn enum_variant_data<'src>(
    ty: impl FigParser<'src, Ty> + 'src,
) -> impl FigParser<'src, VariantData> {
    choice((
        named_field(ty.clone())
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::LBrace), just(Token::RBrace))
            .map(VariantData::Struct),
        enum_tuple_field(ty)
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::LParen), just(Token::RParen))
            .map(VariantData::Tuple),
    ))
    .or_not()
    .map(|data| data.unwrap_or(VariantData::Unit))
}

fn variant<'src>(ty: impl FigParser<'src, Ty> + 'src) -> impl FigParser<'src, Variant> {
    annotations()
        .then(ident())
        .then(enum_variant_data(ty))
        .map_with(|((annotations, ident), data), e| Variant {
            annotations,
            span: span(e),
            vis: Visibility {
                kind: VisibilityKind::Inherited,
                span: span(e),
            },
            ident,
            data,
        })
}

fn enum_def<'src>(ty: impl FigParser<'src, Ty> + 'src) -> impl FigParser<'src, EnumDef> {
    variant(ty)
        .separated_by(just(Token::Comma))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::LBrace), just(Token::RBrace))
        .map(|variants| EnumDef { variants })
}

fn ty_alias<'src>(ty: impl FigParser<'src, Ty> + 'src) -> impl FigParser<'src, TyAlias> {
    just(Token::Type)
        .ignore_then(ident())
        .then(
            generic_param()
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::Lt), just(Token::Gt))
                .or_not()
                .map(Option::unwrap_or_default),
        )
        .then(
            just(Token::Colon)
                .ignore_then(generic_bounds())
                .or_not()
                .map(Option::unwrap_or_default),
        )
        .then(just(Token::Where).ignore_then(where_clause()).or_not())
        .then(just(Token::Eq).ignore_then(ty).or_not())
        .then(just(Token::Where).ignore_then(where_clause()).or_not())
        .then_ignore(just(Token::Semi))
        .map_with(
            |(((((ident, params), bounds), before_where), ty), after_where), e| TyAlias {
                ident,
                generics: Generics {
                    params,
                    where_clause: before_where.unwrap_or_else(|| WhereClause {
                        predicates: Vec::new(),
                        span: span(e),
                    }),
                    span: span(e),
                },
                after_where_clause: after_where.unwrap_or_else(|| WhereClause {
                    predicates: Vec::new(),
                    span: span(e),
                }),
                bounds,
                ty: ty.map(Box::new),
            },
        )
}

fn self_param<'src>() -> impl FigParser<'src, Param> {
    just(Token::SelfLower).map_with(|_, e| Param {
        annotations: Vec::new(),
        ty: Some(Box::new(Ty {
            kind: TyKind::ImplicitSelf,
            span: span(e),
        })),
        pat: Box::new(Pat {
            kind: PatKind::Ident(
                Ident {
                    name: "self".to_owned(),
                    span: span(e),
                },
                None,
            ),
            span: span(e),
        }),
        span: span(e),
    })
}

fn fn_param<'src>(expr: impl FigParser<'src, Expr> + 'src) -> impl FigParser<'src, Param> {
    choice((self_param(), param(expr)))
}

fn fn_item<'src>(
    expr: impl FigParser<'src, Expr> + 'src,
    block: impl FigParser<'src, Block> + 'src,
) -> impl FigParser<'src, Fn> {
    just(Token::Fn)
        .ignore_then(ident())
        .then(generics())
        .then(
            fn_param(expr)
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LParen), just(Token::RParen)),
        )
        .then(fn_ret_ty(ty()))
        .then(choice((block.map(Some), just(Token::Semi).to(None))))
        .map(|((((ident, generics), inputs), output), body)| Fn {
            ident,
            generics,
            sig: FnDecl { inputs, output },
            body: body.map(Box::new),
        })
}

fn assoc_item_kind<'src>(
    expr: impl FigParser<'src, Expr> + 'src,
    block: impl FigParser<'src, Block> + 'src,
) -> impl FigParser<'src, AssocItemKind> {
    choice((
        fn_item(expr, block).map(Box::new).map(AssocItemKind::Fn),
        ty_alias(ty()).map(Box::new).map(AssocItemKind::Type),
    ))
}

fn assoc_item<'src>(
    expr: impl FigParser<'src, Expr> + 'src,
    block: impl FigParser<'src, Block> + 'src,
) -> impl FigParser<'src, AssocItem> {
    annotations()
        .then(visibility())
        .then(assoc_item_kind(expr, block))
        .map_with(|((annotations, vis), kind), e| Item {
            span: span(e),
            vis,
            annotations,
            kind,
        })
}

fn trait_def<'src>(
    expr: impl FigParser<'src, Expr> + 'src,
    block: impl FigParser<'src, Block> + 'src,
) -> impl FigParser<'src, Trait> {
    just(Token::Trait)
        .ignore_then(ident())
        .then(generics())
        .then(
            just(Token::Colon)
                .ignore_then(generic_bounds())
                .or_not()
                .map(Option::unwrap_or_default),
        )
        .then(
            assoc_item(expr, block)
                .map(Box::new)
                .repeated()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LBrace), just(Token::RBrace)),
        )
        .map(|(((ident, generics), bounds), items)| Trait {
            ident,
            generics,
            bounds,
            items,
        })
}

fn impl_def<'src>(
    expr: impl FigParser<'src, Expr> + 'src,
    block: impl FigParser<'src, Block> + 'src,
) -> impl FigParser<'src, Impl> {
    just(Token::Impl)
        .ignore_then(generics())
        .then(choice((
            path(ty())
                .then_ignore(just(Token::For))
                .then(ty())
                .map(|(trait_path, self_ty)| (Some(Box::new(trait_path)), self_ty)),
            ty().map(|self_ty| (None, self_ty)),
        )))
        .then(
            assoc_item(expr, block)
                .map(Box::new)
                .repeated()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LBrace), just(Token::RBrace)),
        )
        .map(|((generics, (of_trait, self_ty)), items)| Impl {
            generics,
            of_trait,
            self_ty: Box::new(self_ty),
            items,
        })
}

fn item_kind<'src>(
    item: impl FigParser<'src, Item> + 'src,
    expr: impl FigParser<'src, Expr> + 'src,
    block: impl FigParser<'src, Block> + 'src,
) -> impl FigParser<'src, ItemKind> {
    choice((
        just(Token::Use)
            .ignore_then(use_tree())
            .then_ignore(just(Token::Semi))
            .map(ItemKind::Use),
        fn_item(expr.clone(), block.clone())
            .map(Box::new)
            .map(ItemKind::Fn),
        just(Token::Mod)
            .ignore_then(ident())
            .then(mod_kind(item))
            .map(|(ident, kind)| ItemKind::Mod(ident, kind)),
        ty_alias(ty()).map(Box::new).map(ItemKind::TyAlias),
        just(Token::Enum)
            .ignore_then(ident())
            .then(generics())
            .then(enum_def(ty()))
            .map(|((ident, generics), def)| ItemKind::Enum(ident, generics, def)),
        just(Token::Struct)
            .ignore_then(ident())
            .then(generics())
            .then(struct_body(ty()))
            .map(|((ident, generics), body)| ItemKind::Struct(ident, generics, body)),
        trait_def(expr.clone(), block.clone())
            .map(Box::new)
            .map(ItemKind::Trait),
        impl_def(expr, block).map(ItemKind::Impl),
    ))
}

/// Takes `expr`/`block` as parameters rather than building fresh ones —
/// so that `stmt()` (which already has an already-tied `expr`/`block` of
/// its own, from `expr()`'s/`block()`'s own recursive ties) can call
/// this for item-statements without recreating them, which would
/// recurse forever: `expr -> block -> stmt -> item -> expr -> ...`.
pub(crate) fn item_with<'src>(
    expr: impl FigParser<'src, Expr> + 'src,
    block: impl FigParser<'src, Block> + 'src,
) -> impl FigParser<'src, Item> {
    recursive(|item| {
        annotations()
            .then(visibility())
            .then(item_kind(item, expr.clone(), block.clone()))
            .map_with(|((annotations, vis), kind), e| Item {
                span: span(e),
                vis,
                annotations,
                kind,
            })
    })
}

pub fn item<'src>() -> impl FigParser<'src, Item> {
    let expr = expr();
    let block = block(expr.clone());
    item_with(expr, block)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::tokens;

    #[test]
    fn parses_a_named_field_with_a_type() {
        let tokens = tokens("x: int");
        let parsed = named_field(ty())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.ident.expect("should have an ident").name, "x");
        assert!(parsed.ty.is_some());
    }

    #[test]
    fn parses_a_named_field_with_no_type_as_dynamic() {
        let tokens = tokens("x");
        let parsed = named_field(ty())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert!(parsed.ty.is_none());
    }

    #[test]
    fn parses_a_pub_named_field() {
        let tokens = tokens("pub x: int");
        let parsed = named_field(ty())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert!(matches!(parsed.vis.kind, VisibilityKind::Public));
    }

    #[test]
    fn parses_a_pub_tuple_field() {
        let tokens = tokens("pub int");
        let parsed = tuple_field(ty())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert!(parsed.ident.is_none());
        assert!(matches!(parsed.vis.kind, VisibilityKind::Public));
    }

    #[test]
    fn parses_a_struct_with_named_fields() {
        let tokens = tokens("{ x: int, y: int }");
        let parsed = struct_body(ty())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let VariantData::Struct(fields) = parsed else {
            panic!("expected VariantData::Struct");
        };
        assert_eq!(fields.len(), 2);
    }

    #[test]
    fn parses_a_tuple_struct_body() {
        let tokens = tokens("(int, float);");
        let parsed = struct_body(ty())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let VariantData::Tuple(fields) = parsed else {
            panic!("expected VariantData::Tuple");
        };
        assert_eq!(fields.len(), 2);
    }

    #[test]
    fn parses_a_unit_struct_body() {
        let tokens = tokens(";");
        let parsed = struct_body(ty())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert!(matches!(parsed, VariantData::Unit));
    }

    #[test]
    fn parses_an_enum_variant_with_no_data() {
        let tokens = tokens("None");
        let parsed = variant(ty())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.ident.name, "None");
        assert!(matches!(parsed.data, VariantData::Unit));
    }

    #[test]
    fn parses_an_enum_variant_with_tuple_data() {
        let tokens = tokens("Some(int)");
        let parsed = variant(ty())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let VariantData::Tuple(fields) = parsed.data else {
            panic!("expected VariantData::Tuple");
        };
        assert_eq!(fields.len(), 1);
        assert!(matches!(fields[0].vis.kind, VisibilityKind::Inherited));
    }

    #[test]
    fn parses_an_enum_variant_with_struct_data() {
        let tokens = tokens("Point { x: int, y: int }");
        let parsed = variant(ty())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        let VariantData::Struct(fields) = parsed.data else {
            panic!("expected VariantData::Struct");
        };
        assert_eq!(fields.len(), 2);
    }

    #[test]
    fn parses_an_enum_def_with_multiple_variants() {
        let tokens = tokens("{ A, B(int), C { x: int } }");
        let parsed = enum_def(ty())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.variants.len(), 3);
    }

    #[test]
    fn parses_a_minimal_trait_associated_type() {
        let tokens = tokens("type Item;");
        let parsed = ty_alias(ty())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.ident.name, "Item");
        assert!(parsed.ty.is_none());
        assert!(parsed.bounds.is_empty());
    }

    #[test]
    fn parses_an_impl_associated_type() {
        let tokens = tokens("type Item = int;");
        let parsed = ty_alias(ty())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert!(parsed.ty.is_some());
    }

    #[test]
    fn parses_a_full_ty_alias_with_bounds_and_where_clauses() {
        let tokens = tokens("type Foo<T>: Display where T: Clone = int where T: Eq;");
        let parsed = ty_alias(ty())
            .parse(tokens)
            .into_result()
            .expect("should parse");
        assert_eq!(parsed.generics.params.len(), 1);
        assert_eq!(parsed.bounds.len(), 1);
        assert_eq!(parsed.generics.where_clause.predicates.len(), 1);
        assert_eq!(parsed.after_where_clause.predicates.len(), 1);
        assert!(parsed.ty.is_some());
    }

    #[test]
    fn parses_a_fn_item() {
        let tokens = tokens("fn add(a: int, b: int) -> int { a }");
        let parsed = item().parse(tokens).into_result().expect("should parse");
        let ItemKind::Fn(f) = parsed.kind else {
            panic!("expected ItemKind::Fn");
        };
        assert_eq!(f.ident.name, "add");
        assert_eq!(f.sig.inputs.len(), 2);
        assert!(matches!(f.sig.output, FnRetTy::Ty(_)));
        assert!(f.body.is_some());
    }

    #[test]
    fn parses_an_ambient_fn_item_with_no_body() {
        let tokens = tokens("fn add(a: int, b: int) -> int;");
        let parsed = item().parse(tokens).into_result().expect("should parse");
        let ItemKind::Fn(f) = parsed.kind else {
            panic!("expected ItemKind::Fn");
        };
        assert!(f.body.is_none());
    }

    #[test]
    fn parses_a_fn_item_with_a_self_param() {
        let tokens = tokens("fn describe(self) -> String { self.name }");
        let parsed = item().parse(tokens).into_result().expect("should parse");
        let ItemKind::Fn(f) = parsed.kind else {
            panic!("expected ItemKind::Fn");
        };
        assert_eq!(f.sig.inputs.len(), 1);
        let PatKind::Ident(ident, _) = &f.sig.inputs[0].pat.kind else {
            panic!(
                "expected PatKind::Ident, got {:?}",
                f.sig.inputs[0].pat.kind
            );
        };
        assert_eq!(ident.name, "self");
        assert!(matches!(
            f.sig.inputs[0].ty.as_deref().map(|ty| &ty.kind),
            Some(TyKind::ImplicitSelf)
        ));
    }

    #[test]
    fn parses_a_fn_item_with_a_self_param_and_more_params() {
        let tokens = tokens("fn heal(self, amount: int) { self.health += amount; }");
        let parsed = item().parse(tokens).into_result().expect("should parse");
        let ItemKind::Fn(f) = parsed.kind else {
            panic!("expected ItemKind::Fn");
        };
        assert_eq!(f.sig.inputs.len(), 2);
        let PatKind::Ident(ident, _) = &f.sig.inputs[0].pat.kind else {
            panic!(
                "expected PatKind::Ident, got {:?}",
                f.sig.inputs[0].pat.kind
            );
        };
        assert_eq!(ident.name, "self");
    }

    #[test]
    fn parses_a_generic_fn_item() {
        let tokens = tokens("fn id<T>(x: T) -> T { x }");
        let parsed = item().parse(tokens).into_result().expect("should parse");
        let ItemKind::Fn(f) = parsed.kind else {
            panic!("expected ItemKind::Fn");
        };
        assert_eq!(f.generics.params.len(), 1);
    }

    #[test]
    fn parses_a_struct_item() {
        let tokens = tokens("struct Point { x: int, y: int }");
        let parsed = item().parse(tokens).into_result().expect("should parse");
        let ItemKind::Struct(ident, generics, data) = parsed.kind else {
            panic!("expected ItemKind::Struct");
        };
        assert_eq!(ident.name, "Point");
        assert!(generics.params.is_empty());
        assert!(matches!(data, VariantData::Struct(_)));
    }

    #[test]
    fn parses_a_pub_struct_item() {
        let tokens = tokens("pub struct Point { x: int }");
        let parsed = item().parse(tokens).into_result().expect("should parse");
        assert!(matches!(parsed.vis.kind, VisibilityKind::Public));
    }

    #[test]
    fn parses_an_enum_item() {
        let tokens = tokens("enum Color { Red, Green, Blue }");
        let parsed = item().parse(tokens).into_result().expect("should parse");
        let ItemKind::Enum(ident, _, def) = parsed.kind else {
            panic!("expected ItemKind::Enum");
        };
        assert_eq!(ident.name, "Color");
        assert_eq!(def.variants.len(), 3);
    }

    #[test]
    fn parses_a_use_item() {
        let tokens = tokens("use foo::bar;");
        let parsed = item().parse(tokens).into_result().expect("should parse");
        let ItemKind::Use(tree) = parsed.kind else {
            panic!("expected ItemKind::Use");
        };
        assert_eq!(tree.prefix.segments.len(), 2);
    }

    #[test]
    fn parses_a_ty_alias_item() {
        let tokens = tokens("type Meters = float;");
        let parsed = item().parse(tokens).into_result().expect("should parse");
        assert!(matches!(parsed.kind, ItemKind::TyAlias(_)));
    }

    #[test]
    fn parses_a_trait_item_with_a_supertrait_bound_and_members() {
        let tokens = tokens("trait Shape: Clone { fn area(self) -> float; type Unit; }");
        let parsed = item().parse(tokens).into_result().expect("should parse");
        let ItemKind::Trait(t) = parsed.kind else {
            panic!("expected ItemKind::Trait");
        };
        assert_eq!(t.ident.name, "Shape");
        assert_eq!(t.bounds.len(), 1);
        assert_eq!(t.items.len(), 2);
        assert!(matches!(t.items[0].kind, AssocItemKind::Fn(_)));
        assert!(matches!(t.items[1].kind, AssocItemKind::Type(_)));
    }

    #[test]
    fn parses_an_inherent_impl() {
        let tokens = tokens("impl Point { fn zero() -> Point; }");
        let parsed = item().parse(tokens).into_result().expect("should parse");
        let ItemKind::Impl(i) = parsed.kind else {
            panic!("expected ItemKind::Impl");
        };
        assert!(i.of_trait.is_none());
        assert_eq!(i.items.len(), 1);
    }

    #[test]
    fn parses_a_trait_impl() {
        let tokens = tokens("impl Clone for Point { fn clone(self) -> Point; }");
        let parsed = item().parse(tokens).into_result().expect("should parse");
        let ItemKind::Impl(i) = parsed.kind else {
            panic!("expected ItemKind::Impl");
        };
        assert!(i.of_trait.is_some());
        assert_eq!(i.items.len(), 1);
    }

    #[test]
    fn parses_an_unloaded_mod_item() {
        let tokens = tokens("mod foo;");
        let parsed = item().parse(tokens).into_result().expect("should parse");
        let ItemKind::Mod(ident, kind) = parsed.kind else {
            panic!("expected ItemKind::Mod");
        };
        assert_eq!(ident.name, "foo");
        assert!(matches!(kind, ModKind::Unloaded));
    }

    #[test]
    fn parses_a_loaded_mod_item_with_nested_items() {
        let tokens = tokens("mod foo { struct A; fn b() {} }");
        let parsed = item().parse(tokens).into_result().expect("should parse");
        let ItemKind::Mod(_, ModKind::Loaded(items)) = parsed.kind else {
            panic!("expected ItemKind::Mod with ModKind::Loaded");
        };
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn parses_annotations_on_an_item() {
        let tokens = tokens("#[component] struct Position { x: int }");
        let parsed = item().parse(tokens).into_result().expect("should parse");
        assert_eq!(parsed.annotations.len(), 1);
    }
}
