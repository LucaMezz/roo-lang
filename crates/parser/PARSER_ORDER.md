# Parser implementation order

Bottom-up build order for `crates/parser`, derived from the node
dependency graph in `crates/ast/src/lib.rs`. Within a tier, order mostly
doesn't matter. Two real cycles exist in the grammar (noted below) —
they're resolved by writing mutually-recursive parser functions, not by
picking an order.

Check items off as their parser function lands.

## Tier 0 — Leaf tokens

- [x] `Ident`
- [x] `Lit` / `LitKind` (`literal()`) — Char, Str, RawStr, Int, Float
- [x] `UnOp`
- [x] `BinOpKind` / `BinOp`
- [x] `AssignOpKind` / `AssignOp`
- [x] `Label`

## Tier 1 — Path & Type (mutually recursive — build together)

`TyKind::Path` needs `Path`; `GenericArg::Arg` needs `Ty`. Write
`parse_path`/`parse_ty` as two functions calling each other.

- [x] `PathSegment`, `GenericArgs`, `GenericArg`, `AssocItemConstraint`
- [x] `Path`
- [x] `TyKind`, `Ty`, `FnTy`, `FnRetTy`

## Tier 2 — Generics (needs `Ty`)

- [x] `GenericParam`, `GenericBounds`
- [x] `WherePredicate`, `WhereClause`
- [ ] `Generics`

## Tier 3 — small standalone consumers of Tier 1/2

- [x] `QSelf` (needs `Ty`)
- [x] `MetaItem`, `MetaItemKind`, `MetaItemInner`, `Annotation`, `AnnotationVec` (need `Path`, `Lit`)
- [x] `Visibility`, `VisibilityKind` (needs `Path`)

## Tier 4 — Pattern

`PatKind::Expr`/`PatKind::Range` embed `Box<Expr>`, but only for
literal/path-like sub-expressions — parse those with a **restricted**
expr parser (literals + unary minus + paths), not the full one below.

- [x] `PatField`
- [x] `PatKind`, `Pat` — leaf-first: `Wild`/`Rest`/`Never` → `Ident`/`Path` →
      `Tuple`/`Array`/`Or`/`Struct`/`TupleStruct` → `Range`/`Expr`

## Tier 5 — Full Expression

`Closure`/`FnDecl`/`Param` are expression-level, not item-level, and
live here.

- [x] `Guard`, `Arm`
- [x] `ExprField`, `StructExpr`
- [x] `MethodCall`
- [x] `Param`, `FnDecl`
- [x] `Closure`
- [x] `ExprKind`, `Expr` — use `ExprPrecedence`/`Fixity`/`BinOpKind::precedence`
      for precedence climbing

## Tier 6 — Block/Stmt/Local

Second cycle: `Expr::If/While/ForLoop/Loop/Block` need `Block`; `Block`
needs `Stmt`; `Stmt::Item` needs `Item`; `Item::Fn` needs `Block` for its
body. `parse_block`/`parse_stmt` call into both `parse_expr` and
`parse_item` as forward references.

- [x] `LocalKind`, `Local`
- [x] `StmtKind`, `Stmt`
- [x] `Block`
- [x] go back and wire up the `Expr` variants that need `Block`: `If`,
      `While`, `ForLoop`, `Loop`, `Match`, `Block`

## Tier 7 — Item and its substructures

- [ ] `FieldDef`
- [ ] `VariantData`, `Variant`, `EnumDef`
- [ ] `TyAlias`
- [ ] `UseTreeKind`, `UseTree`
- [ ] `ModKind`
- [ ] `Fn`
- [ ] `AssocItemKind`, `AssocItem`
- [ ] `Trait`, `Impl`
- [ ] `ItemKind`
- [ ] `Item<K>`

## Tier 8 — Top level

- [ ] crate root / module parse entry point (`Vec<Box<Item>>` or similar —
      lives in `parser`, not yet defined)
