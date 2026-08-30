//! Defines the def data model — [`Def`], [`DefKind`], and the
//! per-kind payload of each kind of def — and [`Defs`], the def
//! table containing every [`Def`] in the program.

use std::marker::PhantomData;

use ast::Span;
use intern::Symbol;
use slotmap::SlotMap;

use crate::inference::TyId;
use crate::{DefId, GenericId, Namespace, ScopeId};

pub(crate) struct DefIdOf<K>(DefId, PhantomData<K>);

impl<K> DefIdOf<K> {
    /// Wraps a `DefId` as a `DefIdOf<K>`. Only call this at the moment
    /// `K` is actually established, either because the `DefKind`
    /// being constructed or matched is statically known to be `K`'s
    /// def kind.
    pub(crate) fn new_unchecked(id: DefId) -> Self {
        Self(id, PhantomData)
    }

    /// Discards the kind information, recovering the underlying
    /// untyped [`DefId`].
    pub(crate) fn id(self) -> DefId {
        self.0
    }
}

// Manual impls: a derive would add a spurious `K: Trait` bound, but
// `K` is a zero-sized marker that never actually appears in `self`.
impl<K> Clone for DefIdOf<K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K> Copy for DefIdOf<K> {}

impl<K> std::fmt::Debug for DefIdOf<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("DefIdOf").field(&self.0).finish()
    }
}

impl<K> PartialEq for DefIdOf<K> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<K> Eq for DefIdOf<K> {}

/// Ties a def payload type to the [`DefKind`] variant it belongs in,
/// so [`TypeCheckContext::declare_typed`](crate::TypeCheckContext::declare_typed)
/// can mint a `DefIdOf<Self>` without a runtime check: the payload type
/// being constructed *is* the proof of which kind it is, so it can
/// double as `DefIdOf`'s own phantom tag — no separate marker type
/// needed.
pub(crate) trait IntoDefKind: Sized {
    fn into_def_kind(self) -> DefKind;
}

impl IntoDefKind for FnDef {
    fn into_def_kind(self) -> DefKind {
        DefKind::Fn(self)
    }
}

impl IntoDefKind for StructDef {
    fn into_def_kind(self) -> DefKind {
        DefKind::Struct(self)
    }
}

impl IntoDefKind for EnumDef {
    fn into_def_kind(self) -> DefKind {
        DefKind::Enum(self)
    }
}

impl IntoDefKind for TyAliasDef {
    fn into_def_kind(self) -> DefKind {
        DefKind::TyAlias(self)
    }
}

impl IntoDefKind for VariantDef {
    fn into_def_kind(self) -> DefKind {
        DefKind::Variant(self)
    }
}

impl IntoDefKind for TraitDef {
    fn into_def_kind(self) -> DefKind {
        DefKind::Trait(self)
    }
}

impl IntoDefKind for ModDef {
    fn into_def_kind(self) -> DefKind {
        DefKind::Mod(self)
    }
}

/// A def within a def table.
#[derive(Debug)]
pub(crate) struct Def {
    /// An interned string which is the symbol of the def.
    pub(crate) symbol: Symbol,

    /// The specific kind of def that it is.
    pub(crate) kind: DefKind,

    /// The span within the source code that resulted in
    /// the introduction of this def.
    pub(crate) declared_at: Span,
}

impl Def {
    /// The ty representing the type associated with this
    /// def.
    ///
    /// Panics if this def's kind can never have a ty (e.g.
    /// [`DefKind::Mod`]). Only call this where the kind is
    /// already known by construction.
    pub(crate) fn ty(&self) -> TyId {
        self.kind.ty().expect("def kind does not have a ty")
    }

    /// The generic parameters associated with this def.
    /// Empty for kinds that can never have generics.
    pub(crate) fn generics(&self) -> &[GenericId] {
        self.kind.generics().unwrap_or(&[])
    }

    /// The variant data of this def, whether it is a struct def or
    /// a bare enum variant def.
    pub(crate) fn variant(&self) -> Option<&VariantDef> {
        self.kind.as_struct_or_variant()
    }

    /// The function data of this def, if it is a function def.
    pub(crate) fn as_fn(&self) -> Option<&FnDef> {
        self.kind.as_fn()
    }
}

/// Extra information about a function def.
#[derive(Debug)]
pub(crate) struct FnDef {
    /// A handle to the scope of the function body.
    pub(crate) scope: ScopeId,

    /// The parameters of the function, in declaration order.
    pub(crate) params: Vec<Param>,

    /// The ty representing the type of this function.
    pub(crate) ty: TyId,

    /// The generic parameters associated with this function.
    pub(crate) generics: Vec<GenericId>,
}

/// One parameter of a function, as it appeared in its signature.
///
/// Bundles the parameter's symbol and the span of its type
/// annotation, which previously lived in two separate
/// `Vec<String>`/`Vec<Option<Span>>` fields on [`FnDef`] populated by
/// two separate passes over `f.sig.inputs`. Nothing tied their
/// lengths or ordering together, so a future edit to either pass
/// could silently desync a parameter's name from its span. Bundling
/// them into one `Vec<Param>`, filled in a single pass, makes that
/// desync impossible to represent: there is only one length and one
/// order to get right.
#[derive(Debug, Clone)]
pub(crate) struct Param {
    /// The parameter's symbol, as it appears in the source code.
    pub(crate) symbol: String,

    /// The span of the parameter's type annotation, if it has one.
    pub(crate) span: Option<Span>,
}

/// Extra information about a type alias def.
#[derive(Debug)]
pub(crate) struct TyAliasDef {
    /// A handle to the scope in which the alias's generic
    /// parameters live.
    pub(crate) scope: ScopeId,

    /// The ty representing the aliased type.
    pub(crate) ty: TyId,

    /// The generic parameters associated with this alias.
    pub(crate) generics: Vec<GenericId>,
}

#[derive(Debug)]
pub(crate) struct EnumDef {
    pub(crate) variants: Vec<DefIdOf<VariantDef>>,
    /// The generic parameters associated with this enum.
    pub(crate) generics: Vec<GenericId>,

    pub(crate) scope: ScopeId,
}

#[derive(Debug)]
pub(crate) struct StructDef {
    pub(crate) variant: VariantDef,

    pub(crate) scope: ScopeId,
}

#[derive(Debug)]
pub(crate) struct TraitDef {
    pub(crate) scope: ScopeId,
}

#[derive(Debug)]
pub(crate) struct ModDef {
    pub(crate) scope: ScopeId,
}

#[derive(Debug)]
pub(crate) struct VariantDef {
    pub(crate) name: Symbol,
    pub(crate) span: Span,
    pub(crate) fields: Vec<FieldDef>,
    pub(crate) ctor_ty: Option<TyId>,
    /// The generic parameters associated with this variant.
    pub(crate) generics: Vec<GenericId>,
    pub(crate) parent: Option<DefIdOf<EnumDef>>,
}

impl VariantDef {
    pub(crate) fn field(&self, symbol: Symbol) -> Option<&FieldDef> {
        self.fields.iter().find(|f| f.name == symbol)
    }
}

#[derive(Debug)]
pub(crate) struct FieldDef {
    pub(crate) name: Symbol,
    pub(crate) ty: TyId,
}

/// The specific kind of [`Def`].
#[derive(Debug)]
pub(crate) enum DefKind {
    Struct(StructDef),
    Enum(EnumDef),
    Variant(VariantDef),
    Trait(TraitDef),
    /// A type alias. Type aliases need their own scope
    /// because they can have generic type parameters which
    /// should only exist during the evaluation of the
    /// type on the right hand side of the alias.
    TyAlias(TyAliasDef),
    Mod(ModDef),
    Fn(FnDef),
    Local(TyId),
    Param(TyId),
    GenericParam(TyId),
}

impl DefKind {
    /// A human-readable description of this kind of def,
    /// e.g. for use in diagnostics like "expected a module,
    /// found a function".
    pub(crate) fn describe(&self) -> &'static str {
        match self {
            DefKind::Struct(_) => "a struct",
            DefKind::Enum(_) => "an enum",
            DefKind::Variant(_) => "an enum variant",
            DefKind::Trait(_) => "a trait",
            DefKind::TyAlias(_) => "a type alias",
            DefKind::Mod(_) => "a module",
            DefKind::Fn(_) => "a function",
            DefKind::Local(_) => "a local variable",
            DefKind::Param(_) => "a parameter",
            DefKind::GenericParam(_) => "a generic parameter",
        }
    }

    /// The ty representing the type of this def, if this
    /// kind of def can have one at all.
    pub(crate) fn ty(&self) -> Option<TyId> {
        match self {
            DefKind::Fn(fn_data) => Some(fn_data.ty),
            DefKind::TyAlias(alias_data) => Some(alias_data.ty),
            DefKind::Local(ty) | DefKind::Param(ty) | DefKind::GenericParam(ty) => Some(*ty),
            DefKind::Struct(StructDef { variant, .. }) | DefKind::Variant(variant) => {
                variant.ctor_ty
            }
            DefKind::Enum(_) | DefKind::Trait(_) | DefKind::Mod(_) => None,
        }
    }

    /// The generic parameters of this def, if this kind of
    /// def can have any at all.
    pub(crate) fn generics(&self) -> Option<&[GenericId]> {
        match self {
            DefKind::Fn(fn_data) => Some(&fn_data.generics),
            DefKind::TyAlias(alias_data) => Some(&alias_data.generics),
            DefKind::Enum(enum_data) => Some(&enum_data.generics),
            DefKind::Struct(StructDef { variant, .. }) | DefKind::Variant(variant) => {
                Some(&variant.generics)
            }
            _ => None,
        }
    }

    /// The variant data of this def, if it is a struct def.
    pub(crate) fn as_struct(&self) -> Option<&VariantDef> {
        match self {
            DefKind::Struct(StructDef { variant, .. }) => Some(variant),
            _ => None,
        }
    }

    /// The variant data of this def, if it is a bare enum variant def.
    pub(crate) fn as_variant(&self) -> Option<&VariantDef> {
        match self {
            DefKind::Variant(variant) => Some(variant),
            _ => None,
        }
    }

    /// The variant data of this def, whether it is a struct def or
    /// a bare enum variant def. Both share a [`VariantDef`] since
    /// they support the same field access.
    pub(crate) fn as_struct_or_variant(&self) -> Option<&VariantDef> {
        self.as_struct().or_else(|| self.as_variant())
    }

    /// The function data of this def, if it is a function def.
    pub(crate) fn as_fn(&self) -> Option<&FnDef> {
        match self {
            DefKind::Fn(fn_data) => Some(fn_data),
            _ => None,
        }
    }

    /// Whether this def is a generic parameter.
    pub(crate) fn is_generic_param(&self) -> bool {
        matches!(self, DefKind::GenericParam(_))
    }

    /// Which [`Namespace`] a def of this kind belongs to
    /// within a scope.
    pub(crate) fn namespace(&self) -> Namespace {
        match self {
            DefKind::Struct(_)
            | DefKind::Enum(_)
            | DefKind::Trait(_)
            | DefKind::TyAlias(_)
            | DefKind::GenericParam(_)
            | DefKind::Mod(_) => Namespace::Type,
            DefKind::Variant(_) | DefKind::Fn(_) | DefKind::Local(_) | DefKind::Param(_) => {
                Namespace::Value
            }
        }
    }
}

/// The def table. Contains all defs within the program. It is a
/// generational arena where a [`DefId`] is a unique handle to a
/// [`Def`].
#[derive(Debug)]
pub(crate) struct Defs {
    defs: SlotMap<DefId, Def>,
}

impl Defs {
    pub(crate) fn new() -> Self {
        Self {
            defs: SlotMap::with_key(),
        }
    }

    /// Inserts a new def into the table, and returns a handle to
    /// it.
    pub(crate) fn insert(&mut self, def: Def) -> DefId {
        self.defs.insert(def)
    }

    pub(crate) fn get(&self, def: DefId) -> &Def {
        &self.defs[def]
    }

    pub(crate) fn len(&self) -> usize {
        self.defs.len()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (DefId, &Def)> {
        self.defs.iter()
    }

    pub(crate) fn struct_mut(&mut self, def: DefIdOf<StructDef>) -> &mut StructDef {
        match &mut self.defs[def.id()].kind {
            DefKind::Struct(s) => s,
            _ => unreachable!("DefIdOf<StructDef> guarantees a struct def"),
        }
    }

    pub(crate) fn enum_mut(&mut self, def: DefIdOf<EnumDef>) -> &mut EnumDef {
        match &mut self.defs[def.id()].kind {
            DefKind::Enum(e) => e,
            _ => unreachable!("DefIdOf<EnumDef> guarantees an enum def"),
        }
    }

    pub(crate) fn fn_mut(&mut self, def: DefIdOf<FnDef>) -> &mut FnDef {
        match &mut self.defs[def.id()].kind {
            DefKind::Fn(f) => f,
            _ => unreachable!("DefIdOf<FnDef> guarantees a function def"),
        }
    }

    pub(crate) fn ty_alias_mut(&mut self, def: DefIdOf<TyAliasDef>) -> &mut TyAliasDef {
        match &mut self.defs[def.id()].kind {
            DefKind::TyAlias(a) => a,
            _ => unreachable!("DefIdOf<TyAliasDef> guarantees a type alias def"),
        }
    }

    pub(crate) fn variant_mut(&mut self, def: DefIdOf<VariantDef>) -> &mut VariantDef {
        match &mut self.defs[def.id()].kind {
            DefKind::Variant(v) => v,
            _ => unreachable!("DefIdOf<VariantDef> guarantees a variant def"),
        }
    }

    pub(crate) fn trait_ref(&self, def: DefIdOf<TraitDef>) -> &TraitDef {
        match &self.defs[def.id()].kind {
            DefKind::Trait(t) => t,
            _ => unreachable!("DefIdOf<TraitDef> guarantees a trait def"),
        }
    }
}

#[cfg(test)]
impl Defs {
    pub(crate) fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }
}
