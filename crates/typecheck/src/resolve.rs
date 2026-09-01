//! Performs the resolution stage of the type checking. See
//! [`Resolver`].

use ast::visit::{Visitable, Visitor, Walkable};
use ast::{
    AssocItemKind, EnumDef as AstEnumDef, Fn, Generics, Ident, Item, ItemKind, ModKind, SELF_TYPE,
    Span, Trait, TyAlias, VariantData,
};
use intern::Symbol;

use crate::defs::{
    EnumDef, FieldDef, FnDef, GenericParamDef, ModDef, StructDef, TraitDef, TyAliasDef, VariantDef,
};
use crate::{CxExt, DefIdOf, Namespace, ScopeId, TypeCheckContext};

/// The AST Visitor that performs the Resolution stage of the
/// type checking. Walks the AST, creating new defs in the
/// def table for each item it finds.
pub(crate) struct Resolver<'a, 'ast> {
    /// Mutable reference to the underlying TypeCheckContext.
    pub(crate) cx: &'a mut TypeCheckContext<'ast>,
}

impl<'ast> Resolver<'_, 'ast> {
    /// Creates a new scope as a child of the current scope in the
    /// underlying [`TypeCheckContext`], and returns a handle to it.
    pub(crate) fn new_scope(&mut self) -> ScopeId {
        self.cx.scopes.new_child(self.cx.current_scope)
    }
}

impl<'ast> CxExt<'ast> for Resolver<'_, 'ast> {
    fn cx(&mut self) -> &mut TypeCheckContext<'ast> {
        self.cx
    }
}

impl Visitor for Resolver<'_, '_> {
    fn visit_item(&mut self, item: &Item) {
        match &item.kind {
            ItemKind::Fn(f) => self.resolve_fn_item(f),
            ItemKind::TyAlias(alias) => self.resolve_ty_alias_item(alias),
            ItemKind::Enum(ident, generics, def) => self.resolve_enum_item(ident, generics, def),
            ItemKind::Struct(ident, generics, data) => {
                self.resolve_struct_item(ident, generics, data)
            }
            ItemKind::Trait(t) => self.resolve_trait_item(t),
            ItemKind::Mod(ident, ModKind::Unloaded) => self.resolve_mod_unloaded_item(ident),
            ItemKind::Mod(ident, ModKind::Loaded(_)) => self.resolve_mod_loaded_item(ident, item),
            ItemKind::Use(_) => self.resolve_use_item(),
            ItemKind::Impl(_) => self.resolve_impl_item(),
        }
    }

    fn visit_assoc_item(&mut self, item: &Item<AssocItemKind>) {
        match &item.kind {
            AssocItemKind::Fn(f) => self.resolve_fn_item(f),
            AssocItemKind::Type(alias) => self.resolve_ty_alias_item(alias),
        }
    }
}

impl Resolver<'_, '_> {
    pub(crate) fn resolve_fn_item(&mut self, f: &Fn) {
        let scope = self.new_scope();
        let ty = self.cx.fresh_var_at(Some(f.ident.span));
        let fn_def = self.cx.declare_typed(
            f.ident.symbol,
            f.ident.span,
            FnDef {
                scope,
                params: Vec::new(),
                ty,
                generics: Vec::new(),
            },
        );

        let mut generics = Vec::new();
        self.with_scope(scope, |this| {
            generics = this.cx.declare_generic_params(&f.generics);
            f.walk(this);
        });
        self.cx.defs.fn_mut(fn_def).generics = generics;
    }

    pub(crate) fn resolve_ty_alias_item(&mut self, alias: &TyAlias) {
        let scope = self.new_scope();
        let ty = self.cx.fresh_var_at(Some(alias.ident.span));
        let alias_def = self.cx.declare_typed(
            alias.ident.symbol,
            alias.ident.span,
            TyAliasDef {
                scope,
                ty,
                generics: Vec::new(),
            },
        );

        let mut generics = Vec::new();
        self.with_scope(scope, |this| {
            generics = this.cx.declare_generic_params(&alias.generics);
        });
        self.cx.defs.ty_alias_mut(alias_def).generics = generics;
    }

    fn resolve_enum_item(&mut self, ident: &Ident, generics: &Generics, def: &AstEnumDef) {
        let scope = self.new_scope();
        let generics = self.with_scope(scope, |this| this.cx.declare_generic_params(generics));
        let enum_def = self.cx.declare_typed(
            ident.symbol,
            ident.span,
            EnumDef {
                variants: Vec::new(),
                generics: generics.clone(),
                scope,
            },
        );
        let variants = def
            .variants
            .iter()
            .map(|v| {
                self.resolve_variant_data(
                    v.ident.symbol,
                    v.ident.span,
                    &v.data,
                    generics.clone(),
                    Some(enum_def),
                )
            })
            .collect::<Vec<_>>();
        let variants = self.with_scope(scope, |this| {
            variants
                .into_iter()
                .map(|v| this.cx.declare_typed(v.name, v.span, v))
                .collect::<Vec<_>>()
        });
        self.cx.defs.enum_mut(enum_def).variants = variants;
    }

    fn resolve_variant_data(
        &mut self,
        name: Symbol,
        span: Span,
        data: &VariantData,
        generics: Vec<DefIdOf<GenericParamDef>>,
        parent: Option<DefIdOf<EnumDef>>,
    ) -> VariantDef {
        let ctor_ty = match data {
            VariantData::Struct(_) => None,
            VariantData::Tuple(_) | VariantData::Unit => Some(self.cx.fresh_var_at(Some(span))),
        };
        match data {
            VariantData::Unit => VariantDef {
                name,
                span,
                fields: vec![],
                ctor_ty,
                generics,
                parent,
            },
            VariantData::Tuple(fields) => VariantDef {
                name,
                span,
                fields: fields
                    .iter()
                    .enumerate()
                    .map(|(i, field)| FieldDef {
                        name: self.cx.symbols.intern_owned(&i.to_string()),
                        ty: self.cx.fresh_var_at(Some(field.span)),
                    })
                    .collect(),
                ctor_ty,
                generics,
                parent,
            },
            VariantData::Struct(fields) => VariantDef {
                name,
                span,
                fields: fields
                    .iter()
                    .map(|field| FieldDef {
                        name: field.ident.unwrap().symbol,
                        ty: self.cx.fresh_var_at(Some(field.span)),
                    })
                    .collect(),
                ctor_ty,
                generics,
                parent,
            },
        }
    }

    fn resolve_struct_item(&mut self, ident: &Ident, generics: &Generics, data: &VariantData) {
        let scope = self.new_scope();
        let generics = self.with_scope(scope, |this| this.cx.declare_generic_params(generics));
        let variant = self.resolve_variant_data(ident.symbol, ident.span, data, generics, None);
        let def = self
            .cx
            .declare_typed(ident.symbol, ident.span, StructDef { variant, scope });
        if !matches!(data, VariantData::Struct(_)) {
            let symbol = ident.symbol;
            self.cx
                .check_redeclaration(Namespace::Value, symbol, ident.span);
            self.cx.insert_value_in_scope(symbol, def.id());
        }
    }

    fn resolve_trait_item(&mut self, t: &Trait) {
        let scope = self.new_scope();
        let (self_generic, generics) = self.with_scope(scope, |this| {
            let symbol = this.cx.symbols.intern(SELF_TYPE);
            (
                this.cx.declare_synthetic_generic_param(symbol),
                this.cx.declare_generic_params(&t.generics),
            )
        });

        self.cx.declare_typed(
            t.ident.symbol,
            t.ident.span,
            TraitDef {
                scope,
                generics,
                self_generic,
            },
        );
        self.with_scope(scope, |this| {
            t.items.iter().for_each(|item| item.visit(this))
        })
    }

    fn resolve_mod_unloaded_item(&mut self, ident: &Ident) {
        let scope = self.new_scope();
        self.cx
            .declare_typed(ident.symbol, ident.span, ModDef { scope });
    }

    fn resolve_mod_loaded_item(&mut self, ident: &Ident, item: &Item) {
        let scope = self.new_scope();
        self.cx
            .declare_typed(ident.symbol, ident.span, ModDef { scope });
        self.with_scope(scope, |this| item.walk(this));
    }

    fn resolve_use_item(&mut self) {}

    fn resolve_impl_item(&mut self) {}
}

#[cfg(test)]
mod tests {
    use crate::tests::*;

    #[test]
    fn declares_a_free_fn_in_the_value_namespace() {
        let cx = resolve("fn bar() {}");
        assert!(lookup(&cx, cx.current_scope, Namespace::Value, "bar"));
    }

    #[test]
    fn declares_a_symbold_struct_only_in_the_type_namespace() {
        let cx = resolve("struct Foo { x: int }");
        assert!(lookup(&cx, cx.current_scope, Namespace::Type, "Foo"));
        assert!(!lookup(&cx, cx.current_scope, Namespace::Value, "Foo"));
    }

    #[test]
    fn declares_a_tuple_struct_in_both_namespaces() {
        let cx = resolve("struct Foo(int);");
        assert!(lookup(&cx, cx.current_scope, Namespace::Type, "Foo"));
        assert!(lookup(&cx, cx.current_scope, Namespace::Value, "Foo"));
    }

    #[test]
    fn a_struct_and_a_fn_can_share_a_symbol() {
        let cx = resolve("struct Foo { x: int } fn Foo() {}");
        assert!(lookup(&cx, cx.current_scope, Namespace::Type, "Foo"));
        assert!(lookup(&cx, cx.current_scope, Namespace::Value, "Foo"));
    }

    #[test]
    fn a_mod_gets_its_own_child_scope() {
        let cx = resolve("mod m { fn baz() {} }");
        assert!(lookup(&cx, cx.current_scope, Namespace::Type, "m"));
        assert!(!lookup(&cx, cx.current_scope, Namespace::Value, "baz"));

        let child_scope = cx
            .scopes
            .child_of(cx.current_scope)
            .expect("mod should have created a child scope");
        assert!(lookup(&cx, child_scope, Namespace::Value, "baz"));
    }

    #[test]
    fn an_item_nested_inside_a_fn_body_is_hoisted_into_its_own_scope() {
        let cx = resolve("fn outer() { fn inner() {} }");
        assert!(lookup(&cx, cx.current_scope, Namespace::Value, "outer"));
        assert!(!lookup(&cx, cx.current_scope, Namespace::Value, "inner"));

        let body_scope = cx
            .scopes
            .child_of(cx.current_scope)
            .expect("the fn body should have created a child scope");
        assert!(lookup(&cx, body_scope, Namespace::Value, "inner"));
    }

    #[test]
    fn resolve_path_finds_a_single_segment_symbol() {
        let mut cx = resolve("struct Foo { x: int }");
        let target = path(&mut cx.symbols, &["Foo"]);
        assert!(cx.resolve_path_to_type(&target).is_some());
    }

    #[test]
    fn resolve_path_fails_on_an_undeclared_symbol() {
        let mut cx = resolve("struct Foo { x: int }");
        let target = path(&mut cx.symbols, &["Bar"]);
        assert!(cx.resolve_path_to_type(&target).is_none());
    }

    #[test]
    fn resolve_path_checks_the_requested_namespace() {
        let mut cx = resolve("struct Foo { x: int }");
        let target = path(&mut cx.symbols, &["Foo"]);
        assert!(cx.resolve_path_to_value(&target).is_none());
    }

    #[test]
    fn resolve_path_walks_through_a_module() {
        let mut cx = resolve("mod m { fn baz() {} }");
        let target = path(&mut cx.symbols, &["m", "baz"]);
        let resolved = cx.resolve_path_to_value(&target);
        assert!(resolved.is_some());
    }

    #[test]
    fn resolve_path_rejects_walking_through_a_non_module_segment() {
        let mut cx = resolve("struct Foo { x: int } fn bar() {}");
        let target = path(&mut cx.symbols, &["Foo", "bar"]);
        assert!(cx.resolve_path_to_value(&target).is_none());
    }

    #[test]
    fn resolve_path_module_segment_is_looked_up_by_namespace_not_by_symbol_alone() {
        let mut cx = resolve("mod m { fn baz() {} } fn m() {}");
        let target = path(&mut cx.symbols, &["m", "baz"]);
        assert!(cx.resolve_path_to_value(&target).is_some());
    }

    #[test]
    fn resolve_path_resolves_a_fn_through_a_primitives_inherent_impl() {
        let mut cx = resolve_and_lower("impl int { fn zero() -> int { 0 } }");
        let target = path(&mut cx.symbols, &["int", "zero"]);
        assert!(cx.resolve_path_to_value(&target).is_some());
    }

    #[test]
    fn resolve_path_resolves_an_associated_type_through_a_primitives_trait_impl() {
        let source = indoc! { r#"
            trait Add<Rhs = Self> {
                type Output;
                fn add(self, other: Rhs) -> Output;
            }
            impl Add for int {
                type Output = Self;
                fn add(self, other: Self) -> Output { self }
            }
        "#};
        let mut cx = resolve_and_lower(source);
        let target = path(&mut cx.symbols, &["int", "Output"]);
        assert!(cx.resolve_path_to_type(&target).is_some());
    }

    #[test]
    fn resolve_path_through_a_primitive_impl_checks_the_requested_namespace() {
        let source = indoc! {r#"
            trait Add<Rhs = Self> {
                type Output;
                fn add(self, other: Rhs) -> Output;
            }
            impl Add for int {
                type Output = Self;
                fn add(self, other: Self) -> Output { self }
            }
        "#};
        let mut cx = resolve_and_lower(source);
        let target = path(&mut cx.symbols, &["int", "Output"]);
        assert!(cx.resolve_path_to_value(&target).is_none());
    }

    #[test]
    fn resolve_path_resolves_through_a_blanket_impl_for_a_primitive() {
        let source = indoc! {r#"
            trait Into<K> {
                fn into() -> K;
            }
            impl<T> Into<T> for T {
                fn into() -> T { 0 }
            }
        "#};
        let mut cx = resolve_and_lower(source);
        let target = path(&mut cx.symbols, &["int", "into"]);
        assert!(cx.resolve_path_to_value(&target).is_some());
    }

    #[test]
    fn resolve_path_fails_when_no_impl_provides_the_requested_item() {
        let mut cx = resolve_and_lower("impl int { fn zero() -> int { 0 } }");
        let target = path(&mut cx.symbols, &["int", "nonexistent"]);
        assert!(cx.resolve_path_to_value(&target).is_none());
    }

    #[test]
    fn resolve_path_is_ambiguous_when_two_impls_define_the_same_name_for_a_primitive() {
        let source = indoc! {r#"
            impl int {
                fn zero() -> int { 0 }
            }
            impl int {
                fn zero() -> int { 1 }
            }
        "#};
        let mut cx = resolve_and_lower(source);
        let target = path(&mut cx.symbols, &["int", "zero"]);
        assert!(cx.resolve_path_to_value(&target).is_none());
    }

    #[test]
    fn resolve_path_resolves_a_fn_through_a_struct_declared_inherent_impl() {
        let mut cx = resolve_and_lower("struct Foo; impl Foo { fn make() -> Foo { Foo } }");
        let target = path(&mut cx.symbols, &["Foo", "make"]);
        assert!(cx.resolve_path_to_value(&target).is_some());
    }

    #[test]
    fn resolve_path_continues_past_an_associated_type_alias_into_its_underlying_types_impls() {
        let source = indoc! {r#"
            trait Add<Rhs = Self> {
                type Output;
                fn add(self, other: Rhs) -> Output;
            }
            impl Add for int {
                type Output = Self;
                fn add(self, other: Self) -> Output { self }
            }
        "#};
        let mut cx = resolve_and_lower(source);
        let target = path(&mut cx.symbols, &["int", "Output", "add"]);
        assert!(cx.resolve_path_to_value(&target).is_some());
    }

    #[test]
    fn resolve_trait_declares_its_generic_params_in_its_own_scope() {
        let cx = resolve("trait Container<T> { fn get() -> T; }");

        let trait_def = declared_def(&cx, cx.current_scope, Namespace::Type, "Container")
            .expect("Container should resolve");
        assert_eq!(cx.def(trait_def).generics().len(), 1);

        let DefKind::Trait(trait_data) = &cx.def(trait_def).kind else {
            panic!("expected a trait def, found {:?}", cx.def(trait_def).kind);
        };
        let t = declared_def(&cx, trait_data.scope, Namespace::Type, "T")
            .expect("T should be declared in the trait's own scope");
        assert!(matches!(cx.def(t).kind, DefKind::GenericParam(_)));
    }
}
