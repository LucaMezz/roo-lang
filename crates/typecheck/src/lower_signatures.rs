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
use crate::defs::{DefKind, Param, TraitDef};
use crate::errors::{
    InvalidGlobTarget, MissingSelfParam, MissingTraitItem, UnexpectedSelfParam, UnresolvedImport,
    expected_due_to,
};
use crate::generics::SyntheticNames;
use crate::inference::TyId;
use crate::resolve::Resolver;
use crate::types::{self, TyKind};
use crate::{
    CxExt, DefId, DefIdOf, GenericId, Namespace, ScopeId, TypeCheckContext, display_path,
    impl_target_of,
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
        let generics = resolver.with_scope(scope, |this| {
            this.cx.declare_generic_params(&imp.generics.params)
        });
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
        subst: &mut HashMap<GenericId, TyId>,
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
        let trait_span = self.cx.def(trait_def).declared_at;
        let impl_span = self.cx.def(impl_def).declared_at;

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
        subst: &mut HashMap<GenericId, TyId>,
    ) {
        let raw_expected = self.cx.def(trait_def).ty();
        let expected = self.cx.instantiate_ty(raw_expected, subst);
        let found = self.cx.def(impl_def).ty();
        let trait_span = self.cx.def(trait_def).declared_at;
        let impl_span = self.cx.def(impl_def).declared_at;

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
    ) -> (Vec<TyId>, Vec<GenericId>) {
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
                        let id = self.cx.generics.declare_synthetic(names);
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
                TyKind::Struct(def.id(), placeholder_args),
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
            let results: Vec<(Vec<TyId>, Vec<GenericId>)> = def
                .variants
                .iter()
                .map(|v| this.lower_variant_data_field_tys(&v.data, &mut names))
                .collect();
            let (lowered, synthesised): (Vec<Vec<TyId>>, Vec<Vec<GenericId>>) =
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
