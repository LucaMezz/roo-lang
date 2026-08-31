//! Performs the resolution stage of the type checking. See
//! [`Resolver`].

use ast::visit::{Visitable, Visitor, Walkable};
use ast::{
    AssocItem, AssocItemKind, EnumDef as AstEnumDef, Fn, Generics, Ident, Item, ItemKind, ModKind,
    SELF_TYPE, Span, Trait, TyAlias, VariantData,
};
use intern::Symbol;

use crate::defs::{EnumDef, FieldDef, FnDef, ModDef, StructDef, TraitDef, TyAliasDef, VariantDef};
use crate::types::TyKind;
use crate::{CxExt, DefIdOf, GenericId, Namespace, ScopeId, TypeCheckContext};

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
        generics: Vec<GenericId>,
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
        let generics = self.with_scope(scope, |this| this.cx.declare_generic_params(&t.generics));
        let self_generic = self.cx.generics.declare_new(SELF_TYPE.to_owned());
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
            let self_ty = this.cx.ty(TyKind::Generic(self_generic));
            this.cx.declare_self_ty_alias(self_ty, t.ident.span);
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
