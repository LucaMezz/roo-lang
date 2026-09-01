//! Performs the signature lowering stage of the type checking. See
//! [`SignatureLowerer`].

use std::collections::HashMap;

use ast::visit::{Visitable, Visitor, Walkable};
use ast::{
    AssocItem, AssocItemKind, EnumDef as AstEnumDef, Fn, Ident, Impl, Item, ItemKind, ModKind,
    Path, SELF_PARAM, Span, Trait, TyAlias, UseTree, UseTreeKind, VariantData,
};
use intern::Symbol;

use crate::check::TypeMismatchExtras;
use crate::defs::{DefKind, GenericParamDef, Param, TraitDef};
use crate::errors::{
    InvalidGlobTarget, MissingSelfParam, MissingTraitItem, UnexpectedSelfParam, UnresolvedImport,
    expected_due_to,
};
use crate::generics::SyntheticNames;
use crate::inference::TyId;
use crate::resolve::Resolver;
use crate::types::{self, TyKind};
use crate::{
    CxExt, DefId, DefIdOf, Namespace, ScopeId, TypeCheckContext, display_path, impl_target_of,
};

/// Performs the signature lowering stage of the type checking.
/// Fills in the types of the defs created by the [`Resolver`]
/// where possible, for example for functions and type aliases.
pub(crate) struct SignatureLowerer<'a, 'ast> {
    /// A mutable reference to the underlying TypeCheckContext.
    pub(crate) cx: &'a mut TypeCheckContext<'ast>,
}

impl<'ast> CxExt<'ast> for SignatureLowerer<'_, 'ast> {
    fn cx(&mut self) -> &mut TypeCheckContext<'ast> {
        self.cx
    }
}

impl SignatureLowerer<'_, '_> {
    /// Creates a ty representing the type of a function
    /// based on the explicit type annotations within its
    /// signature.
    fn lower_fn_sig(&mut self, f: &Fn) -> TyId {
        let inputs = f
            .sig
            .inputs
            .iter()
            .map(|param| match &param.ty {
                Some(ty) => self.cx.lower_ty(ty),
                None => self.cx.fresh_var(),
            })
            .collect();
        let output_ty = self.cx.lower_ret_ty(&f.sig.output, None);
        self.cx.ty(TyKind::Fn(inputs, output_ty))
    }
}

impl SignatureLowerer<'_, '_> {
    fn lower_trait_item(&mut self, trt: &Trait) {
        self.with_trait_scope(trt.ident.symbol, |this, def| {
            let self_generic = this.cx.defs.trait_ref(def).self_generic;
            let self_ty = this.cx.ty(TyKind::Generic(self_generic));
            this.with_self_ty(self_ty, |this| {
                trt.items.iter().for_each(|item| item.visit(this));
            });
        });
    }

    fn lower_impl_item(&mut self, imp: &Impl) {
        let mut resolver = Resolver { cx: self.cx };
        let scope = resolver.new_scope();
        let generics =
            resolver.with_scope(scope, |this| this.cx.declare_generic_params(&imp.generics));
        resolver.with_scope(scope, |this| {
            imp.items.iter().for_each(|item| match &item.kind {
                AssocItemKind::Fn(f) => this.resolve_fn_item(f),
                AssocItemKind::Type(alias) => this.resolve_ty_alias_item(alias),
            });
        });

        let self_ty = self.with_scope(scope, |this| this.cx.lower_ty(&imp.self_ty));

        self.with_scope(scope, |this| {
            this.cx.declare_self_ty_alias(self_ty, imp.self_ty.span);
            this.with_self_ty(self_ty, |this| {
                imp.items.iter().for_each(|item| item.visit(this));
            });
        });

        let resolved = self.cx.inf.resolve(self_ty);
        let target = self.cx.inf.ty(resolved).and_then(impl_target_of);

        let mut trait_args = Vec::new();
        let of_trait = imp.of_trait.as_ref().and_then(|path| {
            let of_trait = self.with_scope(scope, |this| this.cx.resolve_path_to_trait(path));
            if let Some(of_trait) = of_trait {
                let trait_generics = self.cx.defs.trait_ref(of_trait).generics.clone();
                let mut subst =
                    self.with_scope(scope, |this| this.cx.subst_for(&trait_generics, path));
                let self_generic = self.cx.defs.trait_ref(of_trait).self_generic;
                subst.insert(self_generic, self_ty);
                trait_args = self.cx.args_from_subst(&trait_generics, &mut subst);
                self.check_trait_impl_complete(of_trait, scope, path.span, &mut subst);
            }
            of_trait
        });

        match target {
            Some(target) => self
                .cx
                .register_impl_for(target, scope, of_trait, generics, trait_args, self_ty),
            None => self
                .cx
                .register_blanket_impl(scope, of_trait, generics, trait_args, self_ty),
        }
    }

    fn check_trait_impl_complete(
        &mut self,
        of_trait: DefIdOf<TraitDef>,
        impl_scope: ScopeId,
        span: Span,
        subst: &mut HashMap<DefIdOf<GenericParamDef>, TyId>,
    ) {
        let trait_scope = self.cx.defs.trait_ref(of_trait).scope;
        let trait_symbol = self.cx.def(of_trait.id()).symbol;
        let trait_name = self.cx.symbols.resolve(trait_symbol).to_owned();

        let mut missing: Vec<(&'static str, Symbol)> = Vec::new();
        let mut matched: Vec<(DefId, DefId)> = Vec::new();

        for (symbol, trait_def) in self.cx.scopes.entries(trait_scope, Namespace::Value) {
            if !matches!(self.cx.def(trait_def).kind, DefKind::Fn(_)) {
                continue;
            }
            match self.cx.scopes.lookup(impl_scope, symbol, Namespace::Value) {
                Some(impl_def) => matched.push((trait_def, impl_def)),
                None => missing.push(("function", symbol)),
            }
        }

        for (symbol, trait_def) in self.cx.scopes.entries(trait_scope, Namespace::Type) {
            if !matches!(self.cx.def(trait_def).kind, DefKind::TyAlias(_)) {
                continue;
            }
            match self.cx.scopes.lookup(impl_scope, symbol, Namespace::Type) {
                Some(impl_def) => matched.push((trait_def, impl_def)),
                None => missing.push(("associated type", symbol)),
            }
        }

        for (kind, symbol) in missing {
            let name = self.cx.symbols.resolve(symbol).to_owned();
            self.cx.diagnostics.push(MissingTraitItem::new(
                span,
                kind.to_owned(),
                name,
                trait_name.clone(),
            ));
        }

        for (trait_def, impl_def) in matched {
            self.check_trait_item_self_matches(trait_def, impl_def, &trait_name);
            self.check_trait_item_ty_matches(trait_def, impl_def, subst);
        }
    }

    fn check_trait_item_self_matches(
        &mut self,
        trait_def: DefId,
        impl_def: DefId,
        trait_name: &str,
    ) {
        let (Some(trait_fn), Some(impl_fn)) = (
            self.cx.def(trait_def).as_fn(),
            self.cx.def(impl_def).as_fn(),
        ) else {
            return;
        };
        let trait_has_self = trait_fn
            .params
            .first()
            .is_some_and(|p| p.symbol == SELF_PARAM);
        let impl_has_self = impl_fn
            .params
            .first()
            .is_some_and(|p| p.symbol == SELF_PARAM);
        if trait_has_self == impl_has_self {
            return;
        }

        let impl_symbol = self.cx.def(impl_def).symbol;
        let name = self.cx.symbols.resolve(impl_symbol).to_owned();
        let trait_span = self.cx.def(trait_def).span();
        let impl_span = self.cx.def(impl_def).span();

        if trait_has_self {
            self.cx.diagnostics.push(MissingSelfParam::new(
                impl_span,
                name,
                trait_name.to_owned(),
                trait_span,
            ));
        } else {
            self.cx.diagnostics.push(UnexpectedSelfParam::new(
                impl_span,
                name,
                trait_name.to_owned(),
                trait_span,
            ));
        }
    }

    fn check_trait_item_ty_matches(
        &mut self,
        trait_def: DefId,
        impl_def: DefId,
        subst: &mut HashMap<DefIdOf<GenericParamDef>, TyId>,
    ) {
        let raw_expected = self.cx.def(trait_def).ty();
        let expected = self.cx.instantiate_ty(raw_expected, subst);
        let found = self.cx.def(impl_def).ty();
        let trait_span = self.cx.def(trait_def).span();
        let impl_span = self.cx.def(impl_def).span();

        let extras =
            TypeMismatchExtras::default().expected_due_to(Some(expected_due_to(trait_span)));
        let _ = self
            .cx
            .unify_reporting_mismatch(expected, found, impl_span, trait_span, extras);
    }

    fn lower_fn_item(&mut self, f: &Fn) {
        let symbol = f.ident.symbol;
        self.with_fn_scope(symbol, |this, def| {
            let fn_ty = this.lower_fn_sig(f);
            let def_ty = this.cx.def(def.id()).ty();
            // Unifies the fresh placeholder inference variable which
            // was created during the previous Resolution stage with the
            // ty created by lowering the function signature.
            let _ = this.cx.inf.unify(def_ty, fn_ty);

            // Collect each parameter's symbol and type-annotation span
            // together, in one pass, so they can never end up out of
            // step with each other (see `Param`).
            let params: Vec<Param> = f
                .sig
                .inputs
                .iter()
                .map(|p| Param {
                    symbol: types::pat_display_name(&p.pat, &this.cx.symbols),
                    span: p.ty.as_ref().map(|ty| ty.span),
                })
                .collect();
            this.cx.defs.fn_mut(def).params = params;
            f.walk(this);
        });
    }

    /// Resolves a `use` path, optionally rooted at an already
    /// resolved parent module (for paths nested inside a
    /// `use foo::{ ... }` group).
    fn resolve_use_path(
        &mut self,
        prefix: Option<DefId>,
        path: &Path,
        namespace: Namespace,
    ) -> Option<DefId> {
        match prefix {
            Some(pid) => self.cx.resolve_path_from(pid, path, namespace),
            None => self.cx.resolve_path(path, namespace),
        }
    }

    fn resolve_use_path_to_type(&mut self, prefix: Option<DefId>, path: &Path) -> Option<DefId> {
        self.resolve_use_path(prefix, path, Namespace::Type)
    }

    fn resolve_use_path_to_value(&mut self, prefix: Option<DefId>, path: &Path) -> Option<DefId> {
        self.resolve_use_path(prefix, path, Namespace::Value)
    }

    fn lower_use_tree(&mut self, tree: &UseTree, prefix: Option<DefId>) {
        let mut sid = self.resolve_use_path_to_type(prefix, &tree.prefix);
        if sid.is_none() && matches!(tree.kind, UseTreeKind::Simple(_)) {
            sid = self.resolve_use_path_to_value(prefix, &tree.prefix);
        }
        let sid = sid.filter(|&sid| !self.cx.def(sid).kind.is_generic_param());
        let Some(sid) = sid else {
            self.cx.diagnostics.push(UnresolvedImport::new(
                tree.prefix.span,
                display_path(&tree.prefix, &self.cx.symbols),
            ));
            return;
        };

        self.cx.record_path_reference(&tree.prefix, sid);

        match &tree.kind {
            UseTreeKind::Simple(ident) => self.lower_use_tree_simple(tree, sid, ident),
            UseTreeKind::Glob(span) => self.lower_use_tree_glob(tree, sid, *span),
            UseTreeKind::Nested { items, .. } => self.lower_use_tree_nested(items, sid),
        }
    }

    fn lower_use_tree_simple(&mut self, tree: &UseTree, sid: DefId, ident: &Option<Ident>) {
        let Some(ident) = ident
            .as_ref()
            .or(tree.prefix.segments.last().map(|seg| &seg.ident))
        else {
            unreachable!("A path should always have a valid symbol");
        };
        let symbol = ident.symbol;
        let namespace = self.cx().def(sid).kind.namespace();
        self.cx.insert_in_scope(symbol, sid, namespace);
    }

    fn lower_use_tree_glob(&mut self, tree: &UseTree, sid: DefId, span: Span) {
        let Some(scope) = self
            .cx
            .mod_def_scope(sid)
            .map(|(_, scope)| scope)
            .or_else(|| self.cx.enum_def_scope(sid).map(|(_, scope)| scope))
        else {
            self.cx.diagnostics.push(InvalidGlobTarget::new(
                span,
                display_path(&tree.prefix, &self.cx.symbols),
                self.cx.def(sid).kind.describe().to_string(),
            ));
            return;
        };
        let types: Vec<(Symbol, DefId)> = self
            .cx
            .scopes
            .entries(scope, Namespace::Type)
            .into_iter()
            .filter(|(_, sid)| !self.cx.def(*sid).kind.is_generic_param())
            .collect();
        types
            .into_iter()
            .for_each(|(symbol, sid)| self.cx.insert_type_in_scope(symbol, sid));
        self.cx
            .scopes
            .entries(scope, Namespace::Value)
            .into_iter()
            .for_each(|(symbol, sid)| self.cx.insert_value_in_scope(symbol, sid));
    }

    fn lower_use_tree_nested(&mut self, items: &[UseTree], sid: DefId) {
        items
            .iter()
            .for_each(|item| self.lower_use_tree(item, Some(sid)));
    }

    fn lower_ty_alias_item(&mut self, alias: &TyAlias) {
        let Some(ty) = alias.ty.as_ref() else {
            return;
        };
        let symbol = alias.ident.symbol;
        self.with_ty_alias_scope(symbol, |this, def| {
            let aliased = this.cx.lower_ty(ty);
            let def_ty = this.cx.def(def.id()).ty();
            // Unifies the fresh placeholder inference variable which
            // was created during the previous Resolution stage with the
            // ty created by lowering the type of the expression being
            // aliased. A type alias can never refer to itself, directly
            // or indirectly (e.g. `type Foo = (Foo, int);`), since that
            // would make it an infinitely-sized type.
            this.cx.unify_or_report_cycle(def_ty, aliased, ty.span);
        });
    }

    /// Lowers `data`'s field types, synthesising a fresh generic for
    /// each field with no type annotation, drawn from `names`. `names`
    /// is threaded in by the caller (rather than started fresh here)
    /// so that sibling variants of the same enum share one synthesis
    /// scope and can't assign the same name to two different generics.
    fn lower_variant_data_field_tys(
        &mut self,
        data: &VariantData,
        names: &mut SyntheticNames,
    ) -> (Vec<TyId>, Vec<DefIdOf<GenericParamDef>>) {
        let fields = match data {
            VariantData::Unit => return (vec![], vec![]),
            VariantData::Tuple(fields) | VariantData::Struct(fields) => fields,
        };
        let mut synthesized = Vec::new();
        let tys = fields
            .iter()
            .map(|field| {
                field
                    .ty
                    .as_ref()
                    .map(|ty| self.cx.lower_ty(ty))
                    .unwrap_or_else(|| {
                        let symbol = self.cx.symbols.intern(names.fresh().as_str());
                        let id = self.cx.declare_synthetic_generic_param(symbol);
                        synthesized.push(id);
                        self.cx.ty(TyKind::Generic(id))
                    })
            })
            .collect();
        (tys, synthesized)
    }

    fn unify_variant_field_tys(&mut self, def: DefId, lowered: &[TyId]) {
        let Some(variant) = self.cx.def(def).variant() else {
            return;
        };
        let field_tys: Vec<TyId> = variant.fields.iter().map(|field| field.ty).collect();
        for (field_ty, lowered_ty) in field_tys.into_iter().zip(lowered) {
            let _ = self.cx.inf.unify(field_ty, *lowered_ty);
        }
    }

    fn unify_ctor_ty(&mut self, def: DefId, self_ty: TyKind, data: &VariantData, lowered: &[TyId]) {
        let Some(placeholder) = self
            .cx
            .def(def)
            .variant()
            .and_then(|variant| variant.ctor_ty)
        else {
            return;
        };

        let self_ty = self.cx.ty(self_ty);
        let ctor_ty = match data {
            VariantData::Tuple(_) => {
                let params = lowered.into();
                self.cx.ty(TyKind::Fn(params, self_ty))
            }
            VariantData::Unit | VariantData::Struct(_) => self_ty,
        };
        let _ = self.cx.inf.unify(placeholder, ctor_ty);
    }

    fn lower_struct_item(&mut self, ident: &Ident, data: &VariantData) {
        self.with_struct_scope(ident.symbol, |this, def| {
            let mut names = SyntheticNames::new();
            this.cx.reserve_declared_generics(def.id(), &mut names);
            let (lowered, synthesized) = this.lower_variant_data_field_tys(data, &mut names);
            this.cx
                .defs
                .struct_mut(def)
                .variant
                .generics
                .extend(synthesized);
            this.unify_variant_field_tys(def.id(), &lowered);
            let generics = this.cx.def(def.id()).generics().to_vec();
            let placeholder_args = generics
                .iter()
                .map(|&id| this.cx.ty(TyKind::Generic(id)))
                .collect();
            this.unify_ctor_ty(
                def.id(),
                TyKind::Struct(def, placeholder_args),
                data,
                &lowered,
            );
        });
    }

    fn lower_enum_item(&mut self, ident: &Ident, def: &AstEnumDef) {
        self.with_enum_scope(ident.symbol, |this, id| {
            // One `SyntheticNames`, shared across every variant, so a
            // field synthesised for one variant reserves its name
            // against the others too (see `lower_variant_data_field_tys`).
            let mut names = SyntheticNames::new();
            this.cx.reserve_declared_generics(id.id(), &mut names);
            let results: Vec<(Vec<TyId>, Vec<DefIdOf<GenericParamDef>>)> = def
                .variants
                .iter()
                .map(|v| this.lower_variant_data_field_tys(&v.data, &mut names))
                .collect();
            let (lowered, synthesised): (Vec<Vec<TyId>>, Vec<Vec<DefIdOf<GenericParamDef>>>) =
                results.into_iter().unzip();
            this.cx
                .defs
                .enum_mut(id)
                .generics
                .extend(synthesised.iter().flatten());

            let generics = this.cx.defs.enum_mut(id).generics.clone();
            let variants = this.cx.defs.enum_mut(id).variants.clone();
            variants
                .into_iter()
                .zip(lowered)
                .zip(&def.variants)
                .for_each(|((variant, lowered_fields), ast_variant)| {
                    this.cx.defs.variant_mut(variant).generics = generics.clone();
                    this.unify_variant_field_tys(variant.id(), &lowered_fields);
                    let placeholder_args = generics
                        .iter()
                        .map(|&id| this.cx.ty(TyKind::Generic(id)))
                        .collect();
                    this.unify_ctor_ty(
                        variant.id(),
                        TyKind::Enum(id, placeholder_args),
                        &ast_variant.data,
                        &lowered_fields,
                    );
                });
        });
    }

    fn lower_mod_item(&mut self, symbol: &Ident, _kind: &ModKind, item: &Item) {
        self.with_mod_scope(symbol.symbol, |this, _def| item.walk(this));
    }
}

impl Visitor for SignatureLowerer<'_, '_> {
    fn visit_item(&mut self, item: &Item) {
        match &item.kind {
            ItemKind::Fn(f) => self.lower_fn_item(f),
            ItemKind::TyAlias(alias) => self.lower_ty_alias_item(alias),
            ItemKind::Struct(ident, _generics, data) => self.lower_struct_item(ident, data),
            ItemKind::Enum(ident, _generics, def) => self.lower_enum_item(ident, def),
            ItemKind::Mod(ident, kind) => self.lower_mod_item(ident, kind, item),
            ItemKind::Use(tree) => self.lower_use_tree(tree, None),
            ItemKind::Trait(trt) => self.lower_trait_item(trt),
            ItemKind::Impl(imp) => self.lower_impl_item(imp),
        }
    }

    fn visit_assoc_item(&mut self, item: &AssocItem) {
        match &item.kind {
            AssocItemKind::Fn(f) => self.lower_fn_item(f),
            AssocItemKind::Type(alias) => self.lower_ty_alias_item(alias),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::*;

    #[test]
    fn lower_signatures_fn_with_typed_params_and_return() {
        let mut cx = resolve_and_lower("fn add(a: int, b: int) -> float { a }");
        let target = path(&mut cx.symbols, &["add"]);
        let def = cx
            .resolve_path_to_value(&target)
            .expect("add should resolve");
        let def_ty = cx.def(def).ty();

        let Some(TyKind::Fn(input_args, ret)) = resolved_kind(&mut cx, def_ty) else {
            panic!("should be a Fn ty");
        };
        assert_eq!(resolved_kind(&mut cx, input_args[0]), Some(TyKind::Int));
        assert_eq!(resolved_kind(&mut cx, input_args[1]), Some(TyKind::Int));
        assert_eq!(resolved_kind(&mut cx, ret), Some(TyKind::Float));
    }

    #[test]
    fn lower_signatures_fn_with_no_return_type_is_a_fresh_unbound_var() {
        let mut cx = resolve_and_lower("fn foo() {}");
        let target = path(&mut cx.symbols, &["foo"]);
        let def = cx
            .resolve_path_to_value(&target)
            .expect("foo should resolve");
        let def_ty = cx.def(def).ty();

        let Some(TyKind::Fn(_, ret)) = resolved_kind(&mut cx, def_ty) else {
            panic!("should be a Fn ty");
        };
        let resolved = cx.inf.resolve(ret);
        assert!(matches!(cx.inf.ty(resolved), Some(TyKind::Var(_))));
    }

    #[test]
    fn lower_signatures_fn_with_an_untyped_param_gets_a_fresh_var() {
        let mut cx = resolve_and_lower("fn foo(x) {}");
        let target = path(&mut cx.symbols, &["foo"]);
        let def = cx
            .resolve_path_to_value(&target)
            .expect("foo should resolve");
        let def_ty = cx.def(def).ty();

        let Some(TyKind::Fn(input_args, _)) = resolved_kind(&mut cx, def_ty) else {
            panic!("should be a Fn ty");
        };
        let resolved = cx.inf.resolve(input_args[0]);
        assert!(matches!(cx.inf.ty(resolved), Some(TyKind::Var(_))));
    }

    #[test]
    fn lower_signatures_ty_alias() {
        let mut cx = resolve_and_lower("type MyInt = int;");
        let target = path(&mut cx.symbols, &["MyInt"]);
        let def = cx
            .resolve_path_to_type(&target)
            .expect("MyInt should resolve");
        let def_ty = cx.def(def).ty();
        assert_eq!(resolved_kind(&mut cx, def_ty), Some(TyKind::Int));
    }

    #[test]
    fn lower_signatures_each_struct_synthesises_field_generics_starting_from_t_independently() {
        // Regression test: struct/enum field synthesis used to share one
        // ever-incrementing counter across the whole program, so `First`
        // would correctly synthesise `T` for its untyped field but
        // `Second` would get `U` instead of independently restarting at
        // `T`.
        let mut cx = resolve_and_lower(indoc! {r#"
            struct First { value }
            struct Second { value }
        "#});

        let first_path = path(&mut cx.symbols, &["First"]);
        let first = cx
            .resolve_path_to_type(&first_path)
            .expect("First should resolve");
        let second_path = path(&mut cx.symbols, &["Second"]);
        let second = cx
            .resolve_path_to_type(&second_path)
            .expect("Second should resolve");

        let first_field_ty = cx.def(first).variant().expect("First is a struct").fields[0].ty;
        let second_field_ty = cx.def(second).variant().expect("Second is a struct").fields[0].ty;

        assert_eq!(cx.renderer().render_ty(first_field_ty), "T");
        assert_eq!(cx.renderer().render_ty(second_field_ty), "T");
    }

    #[test]
    fn lower_signatures_sibling_enum_variants_synthesise_field_generics_without_colliding() {
        // Unrelated structs each independently get `T` (see the test
        // above), but sibling variants of the *same* enum are a different
        // case: they must NOT collide, since two distinct generics named
        // `T` on the same enum would be ambiguous. One `SyntheticNames` is
        // shared across all of an enum's variants for exactly this reason.
        let mut cx = resolve_and_lower(indoc! {r#"
            enum Either {
                A { value },
                B { value },
            }
        "#});

        let target = path(&mut cx.symbols, &["Either"]);
        let either = cx
            .resolve_path_to_type(&target)
            .expect("Either should resolve");

        let DefKind::Enum(EnumDef { variants, .. }) = &cx.defs.get(either).kind else {
            panic!("Either should be an enum");
        };
        let variants: Vec<DefId> = variants.iter().map(|v| v.id()).collect();
        assert_eq!(variants.len(), 2);

        let a_field_ty = cx
            .def(variants[0])
            .variant()
            .expect("A is a variant")
            .fields[0]
            .ty;
        let b_field_ty = cx
            .def(variants[1])
            .variant()
            .expect("B is a variant")
            .fields[0]
            .ty;

        assert_eq!(cx.renderer().render_ty(a_field_ty), "T");
        assert_eq!(cx.renderer().render_ty(b_field_ty), "U");
    }

    #[test]
    fn lower_signatures_recurses_into_a_fns_own_body() {
        let mut cx = resolve_and_lower("fn outer() { fn inner(x: int) -> bool { true } }");
        let body_scope = cx
            .scopes
            .child_of(cx.current_scope)
            .expect("outer's body should have a child scope");
        let def = declared_def(&cx, body_scope, Namespace::Value, "inner")
            .expect("inner should be declared");
        let def_ty = cx.def(def).ty();
        assert!(matches!(
            resolved_kind(&mut cx, def_ty),
            Some(TyKind::Fn(..))
        ));
    }

    #[test]
    fn lower_signatures_recurses_into_a_mod() {
        let mut cx = resolve_and_lower("mod m { fn baz(x: bool) {} }");
        let target = path(&mut cx.symbols, &["m"]);
        let m_def = cx
            .resolve_path_to_type(&target)
            .expect("m should resolve");
        let DefKind::Mod(mod_data) = &cx.def(m_def).kind else {
            panic!("m should be a Mod def");
        };
        let m_scope = mod_data.scope;

        let def = declared_def(&cx, m_scope, Namespace::Value, "baz").expect("baz should resolve");
        let def_ty = cx.def(def).ty();
        assert!(matches!(
            resolved_kind(&mut cx, def_ty),
            Some(TyKind::Fn(..))
        ));
    }

    #[test]
    fn lower_impl_item_self_param_resolves_to_the_impls_self_ty() {
        let source = indoc! {r#"
            struct Foo;
            impl Foo {
                fn hello(self) -> bool {
                    true
                }
            }
        "#};
        let mut cx = resolve_and_lower(source);
        assert!(cx.diagnostics.is_empty());

        let foo_def = declared_def(&cx, cx.current_scope, Namespace::Type, "Foo")
            .expect("Foo should resolve");

        let hello_offset = source.find("hello").expect("source contains hello");
        let hello_def = cx
            .def_at(hello_offset)
            .expect("hello should have a recorded def");
        let hello_ty = cx.def(hello_def).ty();

        let Some(TyKind::Fn(params, _)) = resolved_kind(&mut cx, hello_ty) else {
            panic!("expected a Fn ty");
        };
        assert_eq!(params.len(), 1);
        let Some(TyKind::Struct(self_struct_def, _)) = resolved_kind(&mut cx, params[0]) else {
            panic!("expected self param to resolve to a Struct ty");
        };
        assert_eq!(self_struct_def, DefIdOf::new_unchecked(foo_def));
    }

    #[test]
    fn lower_impl_item_self_param_is_specific_to_each_impl_block() {
        let source = indoc! {r#"
            struct Foo;
            struct Bar;
            impl Foo {
                fn hello(self) -> bool { true }
            }
            impl Bar {
                fn greet(self) -> bool { true }
            }
        "#};
        let mut cx = resolve_and_lower(source);
        assert!(cx.diagnostics.is_empty());

        let foo_def = declared_def(&cx, cx.current_scope, Namespace::Type, "Foo")
            .expect("Foo should resolve");
        let bar_def = declared_def(&cx, cx.current_scope, Namespace::Type, "Bar")
            .expect("Bar should resolve");

        let hello_def = cx
            .def_at(source.find("hello").unwrap())
            .expect("hello should have a recorded def");
        let greet_def = cx
            .def_at(source.find("greet").unwrap())
            .expect("greet should have a recorded def");

        let hello_ty = cx.def(hello_def).ty();
        let greet_ty = cx.def(greet_def).ty();
        let Some(TyKind::Fn(hello_params, _)) = resolved_kind(&mut cx, hello_ty) else {
            panic!("expected a Fn ty");
        };
        let Some(TyKind::Fn(greet_params, _)) = resolved_kind(&mut cx, greet_ty) else {
            panic!("expected a Fn ty");
        };

        let Some(TyKind::Struct(hello_self_def, _)) = resolved_kind(&mut cx, hello_params[0])
        else {
            panic!("expected self param to resolve to a Struct ty");
        };
        let Some(TyKind::Struct(greet_self_def, _)) = resolved_kind(&mut cx, greet_params[0])
        else {
            panic!("expected self param to resolve to a Struct ty");
        };
        assert_eq!(hello_self_def, DefIdOf::new_unchecked(foo_def));
        assert_eq!(greet_self_def, DefIdOf::new_unchecked(bar_def));
    }

    #[test]
    fn lower_trait_item_self_param_resolves_to_a_generic_ty() {
        let source = indoc! {r#"
            trait Greet {
                fn hello(self) -> int;
            }
        "#};
        let mut cx = resolve_and_lower(source);
        assert!(cx.diagnostics.is_empty());

        let hello_offset = source.find("hello").expect("source contains hello");
        let hello_def = cx
            .def_at(hello_offset)
            .expect("hello should have a recorded def");
        let hello_ty = cx.def(hello_def).ty();

        let Some(TyKind::Fn(params, _)) = resolved_kind(&mut cx, hello_ty) else {
            panic!("expected a Fn ty");
        };
        assert_eq!(params.len(), 1);
        assert!(matches!(
            resolved_kind(&mut cx, params[0]),
            Some(TyKind::Generic(_))
        ));
    }

    #[test]
    fn lower_trait_item_self_param_is_consistent_across_the_traits_own_items() {
        let source = indoc! {r#"
            trait Greet {
                fn hello(self) -> int;
                fn bye(self) -> int;
            }
        "#};
        let mut cx = resolve_and_lower(source);
        assert!(cx.diagnostics.is_empty());

        let hello_def = cx
            .def_at(source.find("hello").unwrap())
            .expect("hello should have a recorded def");
        let bye_def = cx
            .def_at(source.find("bye").unwrap())
            .expect("bye should have a recorded def");

        let hello_ty = cx.def(hello_def).ty();
        let bye_ty = cx.def(bye_def).ty();
        let Some(TyKind::Fn(hello_params, _)) = resolved_kind(&mut cx, hello_ty) else {
            panic!("expected a Fn ty");
        };
        let Some(TyKind::Fn(bye_params, _)) = resolved_kind(&mut cx, bye_ty) else {
            panic!("expected a Fn ty");
        };

        let Some(TyKind::Generic(hello_self)) = resolved_kind(&mut cx, hello_params[0]) else {
            panic!("expected self param to resolve to a Generic ty");
        };
        let Some(TyKind::Generic(bye_self)) = resolved_kind(&mut cx, bye_params[0]) else {
            panic!("expected self param to resolve to a Generic ty");
        };
        assert_eq!(hello_self, bye_self);
    }

    #[test]
    fn lower_trait_item_self_type_resolves_to_the_traits_self_generic() {
        let source = indoc! {r#"
            trait Make {
                fn make() -> Self;
            }
        "#};
        let mut cx = resolve_and_lower(source);
        assert!(cx.diagnostics.is_empty());

        let make_def = cx
            .def_at(source.find("make").unwrap())
            .expect("make should have a recorded def");
        let make_ty = cx.def(make_def).ty();

        let Some(TyKind::Fn(_, output)) = resolved_kind(&mut cx, make_ty) else {
            panic!("expected a Fn ty");
        };
        assert!(matches!(
            resolved_kind(&mut cx, output),
            Some(TyKind::Generic(_))
        ));
    }

    #[test]
    fn lower_trait_item_self_type_and_self_param_share_the_same_generic() {
        let source = indoc! {r#"
            trait Make {
                fn make(self) -> Self;
            }
        "#};
        let mut cx = resolve_and_lower(source);
        assert!(cx.diagnostics.is_empty());

        let make_def = cx
            .def_at(source.find("make").unwrap())
            .expect("make should have a recorded def");
        let make_ty = cx.def(make_def).ty();

        let Some(TyKind::Fn(params, output)) = resolved_kind(&mut cx, make_ty) else {
            panic!("expected a Fn ty");
        };
        let Some(TyKind::Generic(param_generic)) = resolved_kind(&mut cx, params[0]) else {
            panic!("expected self param to resolve to a Generic ty");
        };
        let Some(TyKind::Generic(output_generic)) = resolved_kind(&mut cx, output) else {
            panic!("expected Self return type to resolve to a Generic ty");
        };
        assert_eq!(param_generic, output_generic);
    }

    #[test]
    fn lower_impl_item_self_type_resolves_to_the_impls_concrete_self_ty() {
        let source = indoc! {r#"
            struct Foo;
            impl Foo {
                fn make() -> Self {
                    Foo
                }
            }
        "#};
        let mut cx = resolve_and_lower(source);
        assert!(cx.diagnostics.is_empty());

        let foo_def = declared_def(&cx, cx.current_scope, Namespace::Type, "Foo")
            .expect("Foo should resolve");

        let make_def = cx
            .def_at(source.find("make").unwrap())
            .expect("make should have a recorded def");
        let make_ty = cx.def(make_def).ty();

        let Some(TyKind::Fn(_, output)) = resolved_kind(&mut cx, make_ty) else {
            panic!("expected a Fn ty");
        };
        let Some(TyKind::Struct(self_struct_def, _)) = resolved_kind(&mut cx, output) else {
            panic!("expected Self to resolve to a Struct ty");
        };
        assert_eq!(self_struct_def, DefIdOf::new_unchecked(foo_def));
    }

    #[test]
    fn lower_impl_item_self_type_is_specific_to_each_impl_block() {
        let source = indoc! {r#"
            struct Foo;
            struct Bar;
            impl Foo {
                fn hello() -> Self { Foo }
            }
            impl Bar {
                fn greet() -> Self { Bar }
            }
        "#};
        let mut cx = resolve_and_lower(source);
        assert!(cx.diagnostics.is_empty());

        let foo_def = declared_def(&cx, cx.current_scope, Namespace::Type, "Foo")
            .expect("Foo should resolve");
        let bar_def = declared_def(&cx, cx.current_scope, Namespace::Type, "Bar")
            .expect("Bar should resolve");

        let hello_def = cx
            .def_at(source.find("hello").unwrap())
            .expect("hello should have a recorded def");
        let greet_def = cx
            .def_at(source.find("greet").unwrap())
            .expect("greet should have a recorded def");

        let hello_ty = cx.def(hello_def).ty();
        let greet_ty = cx.def(greet_def).ty();
        let Some(TyKind::Fn(_, hello_output)) = resolved_kind(&mut cx, hello_ty) else {
            panic!("expected a Fn ty");
        };
        let Some(TyKind::Fn(_, greet_output)) = resolved_kind(&mut cx, greet_ty) else {
            panic!("expected a Fn ty");
        };

        let Some(TyKind::Struct(hello_self, _)) = resolved_kind(&mut cx, hello_output) else {
            panic!("expected Self to resolve to a Struct ty");
        };
        let Some(TyKind::Struct(greet_self, _)) = resolved_kind(&mut cx, greet_output) else {
            panic!("expected Self to resolve to a Struct ty");
        };
        assert_eq!(hello_self, DefIdOf::new_unchecked(foo_def));
        assert_eq!(greet_self, DefIdOf::new_unchecked(bar_def));
    }

    #[test]
    fn lower_use_tree_simple_imports_a_value_into_the_current_scope() {
        let mut cx = resolve_and_lower("mod m { fn baz() {} } use m::baz;");

        let target = path(&mut cx.symbols, &["m", "baz"]);
        let original = cx
            .resolve_path_to_value(&target)
            .expect("m::baz should resolve");
        let imported = declared_def(&cx, cx.current_scope, Namespace::Value, "baz")
            .expect("baz should have been imported into the current scope");

        assert_eq!(imported, original);
    }

    #[test]
    fn lower_use_tree_glob_imports_every_item_from_a_module() {
        let mut cx = resolve_and_lower("mod m { struct Foo; fn baz() {} } use m::*;");

        let foo_target = path(&mut cx.symbols, &["m", "Foo"]);
        let foo_original = cx
            .resolve_path_to_type(&foo_target)
            .expect("m::Foo should resolve");
        let baz_target = path(&mut cx.symbols, &["m", "baz"]);
        let baz_original = cx
            .resolve_path_to_value(&baz_target)
            .expect("m::baz should resolve");

        let foo_imported = declared_def(&cx, cx.current_scope, Namespace::Type, "Foo")
            .expect("Foo should have been imported into the current scope");
        let baz_imported = declared_def(&cx, cx.current_scope, Namespace::Value, "baz")
            .expect("baz should have been imported into the current scope");

        assert_eq!(foo_imported, foo_original);
        assert_eq!(baz_imported, baz_original);
    }

    #[test]
    fn lower_use_tree_nested_imports_each_item_in_the_group_and_honours_resymbols() {
        let mut cx =
            resolve_and_lower("mod m { struct Foo; fn baz() {} } use m::{Foo, baz as make_baz};");

        let foo_target = path(&mut cx.symbols, &["m", "Foo"]);
        let foo_original = cx
            .resolve_path_to_type(&foo_target)
            .expect("m::Foo should resolve");
        let baz_target = path(&mut cx.symbols, &["m", "baz"]);
        let baz_original = cx
            .resolve_path_to_value(&baz_target)
            .expect("m::baz should resolve");

        let foo_imported = declared_def(&cx, cx.current_scope, Namespace::Type, "Foo")
            .expect("Foo should have been imported into the current scope");
        let make_baz_imported = declared_def(&cx, cx.current_scope, Namespace::Value, "make_baz")
            .expect("baz should have been imported as make_baz");

        assert_eq!(foo_imported, foo_original);
        assert_eq!(make_baz_imported, baz_original);
        assert!(
            declared_def(&cx, cx.current_scope, Namespace::Value, "baz").is_none(),
            "baz should not also be imported under its original symbol"
        );
    }

    #[test]
    fn lower_signatures_makes_the_declared_signature_authoritative() {
        let mut cx = resolve_and_lower("fn foo(x: int) {}");
        let target = path(&mut cx.symbols, &["foo"]);
        let def = cx
            .resolve_path_to_value(&target)
            .expect("foo should resolve");
        let def_ty = cx.def(def).ty();

        let expr_val = expr(&mut cx.symbols, "foo(\"wrong\")");
        cx.check_expr(&expr_val, None);

        let Some(TyKind::Fn(input_args, _)) = resolved_kind(&mut cx, def_ty) else {
            panic!("should still be a Fn ty");
        };
        assert_eq!(resolved_kind(&mut cx, input_args[0]), Some(TyKind::Int));
    }
}
