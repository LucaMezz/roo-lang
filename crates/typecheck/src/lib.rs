//! Contains the type checking stage of the interpreter.
//!
//! The type checking stage depends on the AST that was constructed
//! by the `parser` during the previous stage. Given some AST, the
//! type checker does several passes of the AST.
//!
//! ```text
//!     1.
//! ```

use std::collections::{HashMap, HashSet};

use ast::visit::{Visitor, Walkable};
use ast::{
    EnumDef as AstEnumDef, Fn, FnRetTy, FnTy, GenericParam, Generics, Ident, Item, ItemKind,
    ModKind, Path, Span, Trait, Ty, TyAlias, TyKind as AstTyKind, UseTree, UseTreeKind, Variant,
    VariantData,
};
use intern::{Interner, Symbol};
use slotmap::SlotMap;

use crate::inference::{InferenceTable, TyId, VarId};
use crate::types::TyKind;

mod adt;
mod call_graph;
mod check;
mod checked_program;
mod errors;
mod generic_names;
mod inference;
mod polymorphism;
mod position_index;
mod types;

use check::collect_fn_mod_items;
use errors::Diagnostics;
use generic_names::GenericNames;
use position_index::PositionIndex;

pub use checked_program::CheckedProgram;
pub use diagnostics::{Diagnostic, Level};
pub use errors::Locale;

use crate::call_graph::{CallGraph, SCCCollector};
use crate::errors::{
    AlreadyDefined, AnnotationsNeeded, InvalidGlobTarget, UnresolvedImport, UnresolvedType,
};

slotmap::new_key_type! {
    /// A handle to a scope stored in the scope arena within
    /// the [`TypeCheckContext`]
    pub struct ScopeId;

    /// A handle to a def stored in the def arena within
    /// the [`TypeCheckContext`]
    pub struct DefId;

    /// A handle to a generic parameter stored in the generic
    /// arena within the [`TypeCheckContext`].
    pub struct GenericId;
}

/// A kind of Namespace within each scope.
///
/// A scope has two separate Namespaces for defs. One only
/// contains defs which represent types within the scope,
/// while the other only contains defs which represent
/// values within the scope.
#[derive(Clone, Copy)]
enum Namespace {
    /// The Namespace of Types within a scope.
    Type,

    /// The Namespace of Values within a scope.
    Value,
}

/// A scope. Represents a context where defs can be defined.
///
/// Scopes are created for things such as function bodies,
/// blocks, etc.
#[derive(Debug)]
struct Scope {
    /// A handle to the enclosing scope.
    parent: Option<ScopeId>,

    /// The [Namespace::Type] Namespace. Maps the symbol of each
    /// type defined in this scope to its def's handle.
    types: HashMap<Symbol, DefId>,

    /// The [Namespace::Value] Namespace. Maps the symbol of
    /// each value defined in this scope to its def's
    /// handle.
    values: HashMap<Symbol, DefId>,
}

/// A def within a def table.
#[derive(Debug)]
struct Def {
    /// An interned string which is the symbol of the def.
    symbol: Symbol,

    /// The specific kind of def that it is.
    kind: DefKind,

    /// The span within the source code that resulted in
    /// the introduction of this def.
    declared_at: Span,
}

impl Def {
    /// The ty representing the type associated with this
    /// def.
    ///
    /// Panics if this def's kind can never have a ty (e.g.
    /// [`DefKind::Mod`]). Only call this where the kind is
    /// already known by construction.
    fn ty(&self) -> TyId {
        self.kind.ty().expect("def kind does not have a ty")
    }

    /// The generic parameters associated with this def.
    /// Empty for kinds that can never have generics.
    fn generics(&self) -> &[GenericId] {
        self.kind.generics().unwrap_or(&[])
    }
}

/// Extra information about a function def.
#[derive(Debug)]
struct FnDef {
    /// A handle to the scope of the function body.
    scope: ScopeId,

    /// The span of each of the parameters of the function
    /// within the source code.
    param_spans: Vec<Option<Span>>,

    /// The symbol of each of the parameters as they appear
    /// in the source code.
    param_symbols: Vec<String>,

    /// The ty representing the type of this function.
    ty: TyId,

    /// The generic parameters associated with this function.
    generics: Vec<GenericId>,
}

/// Extra information about a type alias def.
#[derive(Debug)]
struct TyAliasDef {
    /// A handle to the scope in which the alias's generic
    /// parameters live.
    scope: ScopeId,

    /// The ty representing the aliased type.
    ty: TyId,

    /// The generic parameters associated with this alias.
    generics: Vec<GenericId>,
}

#[derive(Debug)]
struct EnumDef {
    variants: Vec<DefId>,
    /// The generic parameters associated with this enum.
    generics: Vec<GenericId>,

    scope: ScopeId,
}

#[derive(Debug)]
struct StructDef {
    variant: VariantDef,

    scope: ScopeId,
}

#[derive(Debug)]
struct VariantDef {
    name: Symbol,
    span: Span,
    fields: Vec<FieldDef>,
    ctor_ty: Option<TyId>,
    /// The generic parameters associated with this variant.
    generics: Vec<GenericId>,
}

impl VariantDef {
    fn field(&self, symbol: Symbol) -> Option<&FieldDef> {
        self.fields.iter().find(|f| f.name == symbol)
    }
}

#[derive(Debug)]
struct FieldDef {
    name: Symbol,
    ty: TyId,
}

/// The specific kind of [`Def`].
#[derive(Debug)]
enum DefKind {
    Struct(StructDef),
    Enum(EnumDef),
    Variant(VariantDef),
    Trait,
    /// A type alias. Type aliases need their own scope
    /// because they can have generic type parameters which
    /// should only exist during the evaluation of the
    /// type on the right hand side of the alias.
    TyAlias(TyAliasDef),
    /// A module. Here, the [`ScopeId`] is a handle to the
    /// scope of the module body.
    Mod(ScopeId),
    Fn(FnDef),
    Local(TyId),
    Param(TyId),
    GenericParam(TyId),
}

impl DefKind {
    /// A human-readable description of this kind of def,
    /// e.g. for use in diagnostics like "expected a module,
    /// found a function".
    fn describe(&self) -> &'static str {
        match self {
            DefKind::Struct(_) => "a struct",
            DefKind::Enum(_) => "an enum",
            DefKind::Variant(_) => "an enum variant",
            DefKind::Trait => "a trait",
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
    fn ty(&self) -> Option<TyId> {
        match self {
            DefKind::Fn(fn_data) => Some(fn_data.ty),
            DefKind::TyAlias(alias_data) => Some(alias_data.ty),
            DefKind::Local(ty) | DefKind::Param(ty) | DefKind::GenericParam(ty) => Some(*ty),
            DefKind::Struct(StructDef { variant, .. }) | DefKind::Variant(variant) => {
                variant.ctor_ty
            }
            DefKind::Enum(_) | DefKind::Trait | DefKind::Mod(_) => None,
        }
    }

    /// The generic parameters of this def, if this kind of
    /// def can have any at all.
    fn generics(&self) -> Option<&[GenericId]> {
        match self {
            DefKind::Fn(fn_data) => Some(&fn_data.generics),
            DefKind::TyAlias(alias_data) => Some(&alias_data.generics),
            DefKind::Struct(StructDef { variant, .. }) => Some(&variant.generics),
            _ => None,
        }
    }
}

/// Differentiates between a variable introduced to the scope
/// of a function via being a parameter, and one introduced
/// by a let def.
#[derive(Clone, Copy)]
enum PatDeclKind {
    Param,
    Let,
}

impl PatDeclKind {
    fn def_kind(self, ty: TyId) -> DefKind {
        match self {
            PatDeclKind::Param => DefKind::Param(ty),
            PatDeclKind::Let => DefKind::Local(ty),
        }
    }
}

impl DefKind {
    /// Which [`Namespace`] a def of this kind belongs to
    /// within a [`Scope`].
    fn namespace(&self) -> Namespace {
        match self {
            DefKind::Struct(_)
            | DefKind::Enum(_)
            | DefKind::Trait
            | DefKind::TyAlias(_)
            | DefKind::GenericParam(_)
            | DefKind::Mod(_) => Namespace::Type,
            DefKind::Variant(_) | DefKind::Fn(_) | DefKind::Local(_) | DefKind::Param(_) => {
                Namespace::Value
            }
        }
    }
}

/// Holds all state required to perform type checking on an
/// entire program. Also provides methods used to complete
/// the type checking and type inference.
struct TypeCheckContext<'ast> {
    /// Keeps track of what every inference variable
    /// throughout the entire program is bound to. Facilitates
    /// O(1) unification of two tys, performing the required
    /// substitutions to ensure two tys are equal. Also
    /// stores a [`Span`] with the reason two tys were
    /// unified.
    inf: InferenceTable,

    /// Maps all symbols used throughout the type checking
    /// process to unique symbol IDs to improve performance.
    /// They are cheap to copy and hash, etc.
    symbols: Interner,

    /// The def table. Contains all defs within the
    /// program. It is a generational arena where a
    /// [`DefId`] is a unique handle to a [`Def`].
    defs: SlotMap<DefId, Def>,

    /// Contains all scopes within the program. It is a
    /// generational arena where a [`ScopeId`] is a unique
    /// handle to a certain [`Scope`].
    scopes: SlotMap<ScopeId, Scope>,

    /// Identifies a unique generic type parameter which appears
    /// somewhere in the program.
    ///
    /// TODO No need for a SlotMap here. GenericIds are never
    /// deleted from the arena, so need for a generational arena.
    generic_ids: SlotMap<GenericId, ()>,

    /// Store and synthesise symbols for generics. See
    /// [`GenericNames`] for more info.
    generic_names: GenericNames,

    /// A handle to the scope that is currently being checked.
    current_scope: ScopeId,

    /// All the diagnostics that have been accumulated so far.
    diagnostics: Diagnostics,

    /// An index which allows you to query for defs and types
    /// and other things, based on spans within the source code.
    positions: PositionIndex,

    graph: CallGraph,

    sccc: SCCCollector,

    items_by_def: HashMap<DefId, &'ast Item>,

    current_fn: Option<DefId>,

    checking_stack: Vec<DefId>,

    inference_vars: Vec<(VarId, Span)>,
}

impl<'ast> TypeCheckContext<'ast> {
    /// Create a new blank [`TypeCheckContext`].
    fn new(symbols: Interner) -> Self {
        let mut scopes = SlotMap::with_key();
        let root = scopes.insert(Scope {
            parent: None,
            types: HashMap::new(),
            values: HashMap::new(),
        });

        Self {
            inf: InferenceTable::new(),

            symbols,
            scopes,
            generic_ids: SlotMap::with_key(),
            generic_names: GenericNames::new(),
            inference_vars: Vec::new(),
            defs: SlotMap::with_key(),
            current_scope: root,
            diagnostics: Diagnostics::default(),
            positions: PositionIndex::default(),
            graph: CallGraph::new(),
            sccc: SCCCollector::new(),
            items_by_def: HashMap::new(),
            current_fn: None,
            checking_stack: Vec::new(),
        }
    }

    fn with_scope<T>(&mut self, scope: ScopeId, f: impl FnOnce(&mut Self) -> T) -> T {
        let parent = self.current_scope;
        self.current_scope = scope;
        let result = f(self);
        self.current_scope = parent;
        result
    }

    fn def(&self, def: DefId) -> &Def {
        &self.defs[def]
    }

    #[cfg(test)]
    pub(crate) fn diagnostics(&self) -> Vec<Diagnostic> {
        let catalog = errors::catalog(errors::Locale::EnUs);
        self.diagnostics
            .as_slice()
            .iter()
            .map(|d| d.render(catalog))
            .collect()
    }

    /// Updates the [`PositionIndex`] to include a newly found path.
    fn record_path_reference(&mut self, path: &Path, def: DefId) {
        if let Some(segment) = path.segments.last() {
            self.positions.record_def(segment.ident.span, def);
        }
    }

    #[cfg(test)]
    pub(crate) fn def_at(&self, offset: usize) -> Option<DefId> {
        self.positions.def_at(offset)
    }

    #[cfg(test)]
    pub(crate) fn type_symbol_at(&self, offset: usize) -> Option<&'static str> {
        self.positions.type_name_at(offset)
    }

    fn fresh_var_at(&mut self, span: Option<Span>) -> TyId {
        let var = self.inf.fresh_var();
        let ty = self.ty_var(var);
        if let Some(span) = span {
            self.inference_vars.push((var, span));
        }
        ty
    }

    fn fresh_var(&mut self) -> TyId {
        self.fresh_var_at(None)
    }

    fn or_fresh_var(&mut self, ty: Option<TyId>) -> TyId {
        ty.unwrap_or_else(|| self.fresh_var())
    }

    fn ty(&mut self, kind: TyKind) -> TyId {
        self.inf.insert_ty(kind)
    }

    fn ty_var(&mut self, id: VarId) -> TyId {
        self.ty(TyKind::Var(id))
    }

    /// Performs the resolution stage of the type checking.
    ///
    /// This is the first stage of the type checking process.
    /// It walks the AST, and creates a new def in the def
    /// table for each item.
    ///
    /// For example, say one of the items is a `function`
    /// ```ignore
    /// fn add_int<T>(a: T, b: int) {
    ///     a + b
    /// }
    /// ```
    /// Then a new [`Def`] is created within the def table
    /// for it, with [`DefKind::Fn`] kind. Note that in this
    /// stage *does* create a [`Scope`] for the function body,
    /// and declares defs for the generic type parameters,
    /// in this case just `T`. However, this stage does *not*
    /// try to determine the type of the new def for the
    /// function yet. Instead, it just assigns a fresh inference
    /// variable to represent its type.
    fn resolve(&mut self, items: &[Box<Item>]) {
        let mut resolver = Resolver { cx: self };
        items.iter().for_each(|item| resolver.visit_item(item));
    }

    /// Performs lowering of signatures. It does this for
    /// functions and also type aliases.
    ///
    /// This is the second stage of the type checking process.
    /// In the previous `resolve` stage, it created new defs
    /// for all of the items within the AST. However, it only
    /// assigns a fresh inference variable as the type of each
    /// def.
    ///
    /// This stage uses information in the AST about types of
    /// the defs and lowers those types into tys. It does
    /// this for all applicable kinds of items.
    ///
    /// For example, say one of the items is a function
    /// ```ignore
    /// fn add_int<T>(a: T, b: int) {
    ///     a + b
    /// }
    /// ```
    /// In the previous resolution stage, a new [`Def`] of
    /// kind [`DefKind::Fn`] would have been created inside
    /// the def table. Now, a new `ty` is created for the
    /// type of this `add_int` function, based on the type
    /// annotations in the signature only. So the [`Def`]
    /// will now have type `Fn<T>(T, int) -> ?a`. The return
    /// type is an inference variable because there is no
    /// return type annotation, and no type checking or
    /// inference of function bodies has been performed yet.
    fn lower_signatures(&mut self, items: &[Box<Item>]) {
        let mut lowerer = SignatureLowerer { cx: self };
        items.iter().for_each(|item| lowerer.visit_item(item));
    }

    /// Performs type checking of function bodies.
    ///
    /// This is the third and final stage of the type checking
    /// process. So far after the `resolve` and
    /// `lower_signatures` stages have compelted, we have a
    /// def table where functions all have types matching
    /// the types annotated in their signatures (including
    /// inference variables `?a` where type annotations were
    /// left out).
    ///
    /// This stage performs type checking and inference on
    /// the body of all functions. It also unifies the ty
    /// representing the type of the function that was created
    /// in the previous `lower_signatures` stage.
    ///
    /// For example, say there is a function
    /// ```ignore
    /// fn apply(f, x: int) {
    ///     f(x)
    /// }
    /// ```
    /// The def for `add_int` would have had its type
    /// recorded as `Fn(?a, x: int) -> ?b` in the previous
    /// step.
    ///
    /// Now we look at the body of the function. Since the
    /// parameter `f` is called with argument `x`, it would
    /// introduce the constraint that `f` must have type
    /// `Fn(int) -> ?c`, since `f` must be a function that
    /// can be called for that call to be valid, and it must
    /// support a parameter of type `int` since `x : int`.
    ///
    /// Additionally, since the call is also the last
    /// expression in the function and has no semicolon,
    /// we also require that the type of the expression
    /// matches the return type of the function `apply`.
    ///
    /// So we `unify(?c, ?b)` which means that both these
    /// inference variables must bind to the same ty.
    /// Suppose the representative is ?b. Then the inferred
    /// type of the function at this point is
    /// `Fn( Fn(int) -> ?b ) -> ?b`.
    ///
    /// At this point [`Self::generalize_group`] introduces
    /// a new type parameter `T` giving final inferred type
    /// `Fn<T>( Fn(int) -> T ) -> T`.
    fn check(&mut self, items: &'ast [Box<Item>]) {
        collect_fn_mod_items(self, items.iter().map(Box::as_ref));

        self.check_items(items.iter().map(Box::as_ref));

        // self.free_variables();
    }

    fn check_items(&mut self, items: impl IntoIterator<Item = &'ast Item>) {
        items.into_iter().for_each(|item| match &item.kind {
            ItemKind::Fn(f) => self.check_function(f),
            ItemKind::Mod(ident, kind) => self.check_module(ident, kind),
            _ => {}
        });
    }

    /// Iterates the recorded list of inference variables and the
    /// span within the source code of what the variables were
    /// created to represent the type of. Finds any inference
    /// variables that are still unbound and raises an error
    /// that type annotations are needed.
    ///
    /// NOTE Not sure if we actually need this function. If the
    /// type of something cannot be inferred, we don't really care
    /// at the moment. Types just ensure there are no type
    /// mismatches at compile time, so if something cant have a
    /// type inferred, it isn't really breaking anything.
    /// Its probably likely that it is dead / unused code anway.
    #[allow(unused)]
    fn free_variables(&mut self) {
        self.inference_vars
            .iter()
            .filter(|(v, _)| self.inf.binding(*v).is_none())
            .for_each(|(_, span)| {
                self.diagnostics.push(AnnotationsNeeded::new(*span));
            });
    }

    /// Returns a handle to the [`Def`] which the given path
    /// references, if it exists in the given Namespace.
    ///
    /// Searches for the def represented by the first
    /// segment in the path recursively up the chain of
    /// enclosed scopes. If it finds it, it then continues
    /// recursively resolving the shortened path until
    /// it potentially arrives at a def.
    fn resolve_path(&mut self, path: &Path, namespace: Namespace) -> Option<DefId> {
        let last = path.segments.len() - 1;

        let mut segments = path.segments.iter().enumerate();
        let (i, first) = segments.next()?;
        let symbol = first.ident.symbol;
        let first_def = self.lookup_up_scope_chain(
            self.current_scope,
            symbol,
            segment_namespace(i, last, namespace),
        )?;

        segments.try_fold(first_def, |def, (i, segment)| {
            let scope = self.mod_def_scope(def)?;
            let symbol = segment.ident.symbol;
            self.lookup_in_scope(scope, symbol, segment_namespace(i, last, namespace))
        })
    }

    fn resolve_path_to_type(&mut self, path: &Path) -> Option<DefId> {
        self.resolve_path(path, Namespace::Type)
    }

    fn resolve_path_to_value(&mut self, path: &Path) -> Option<DefId> {
        self.resolve_path(path, Namespace::Value)
    }

    fn resolve_path_to_struct(&mut self, path: &Path) -> Option<(DefId, &VariantDef)> {
        let id = self.resolve_path_to_type(path)?;
        match &self.def(id).kind {
            DefKind::Struct(StructDef { variant, .. }) => Some((id, variant)),
            _ => None,
        }
    }

    fn resolve_path_to_enum(&mut self, path: &Path) -> Option<DefId> {
        self.resolve_path_to_type(path)
            .filter(|def| matches!(self.def(*def).kind, DefKind::Enum(_)))
    }

    fn resolve_path_to_variant(&mut self, path: &Path) -> Option<DefId> {
        self.resolve_path_to_type(path)
            .filter(|def| matches!(self.def(*def).kind, DefKind::Variant(_)))
    }

    fn resolve_path_from(
        &mut self,
        root: DefId,
        path: &Path,
        namespace: Namespace,
    ) -> Option<DefId> {
        let last = path.segments.len() - 1;
        path.segments
            .iter()
            .enumerate()
            .try_fold(root, |def, (i, segment)| {
                let scope = self.mod_def_scope(def)?;
                let symbol = segment.ident.symbol;
                self.lookup_in_scope(scope, symbol, segment_namespace(i, last, namespace))
            })
    }

    /// Directly checks if the given scope contains a def
    /// with a given symbol, which belongs to a certain Namespace.
    fn lookup_in_scope(
        &self,
        scope: ScopeId,
        symbol: Symbol,
        namespace: Namespace,
    ) -> Option<DefId> {
        let map = match namespace {
            Namespace::Type => &self.scopes[scope].types,
            Namespace::Value => &self.scopes[scope].values,
        };
        map.get(&symbol).copied()
    }

    fn with_def_in_scope<T>(
        &mut self,
        symbol: Symbol,
        namespace: Namespace,
        f: impl FnOnce(&mut Self, DefId) -> T,
    ) -> Option<T> {
        self.lookup_in_scope(self.current_scope, symbol, namespace)
            .map(|def| f(self, def))
    }

    fn with_type_def<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefId) -> T,
    ) -> Option<T> {
        self.with_def_in_scope(symbol, Namespace::Type, f)
    }

    fn with_value_def<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefId) -> T,
    ) -> Option<T> {
        self.with_def_in_scope(symbol, Namespace::Value, f)
    }

    fn fn_def_scope(&self, def: DefId) -> Option<ScopeId> {
        let DefKind::Fn(fn_data) = &self.def(def).kind else {
            return None;
        };
        Some(fn_data.scope)
    }

    fn struct_def_scope(&self, def: DefId) -> Option<ScopeId> {
        let DefKind::Struct(def) = &self.def(def).kind else {
            return None;
        };
        Some(def.scope)
    }

    fn mod_def_scope(&self, def: DefId) -> Option<ScopeId> {
        let DefKind::Mod(scope) = self.def(def).kind else {
            return None;
        };
        Some(scope)
    }

    fn ty_alias_def_scope(&self, def: DefId) -> Option<ScopeId> {
        let DefKind::TyAlias(alias_data) = &self.def(def).kind else {
            return None;
        };
        Some(alias_data.scope)
    }

    fn with_fn_def<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefId, ScopeId) -> T,
    ) -> Option<T> {
        self.with_value_def(symbol, |this, def| {
            this.fn_def_scope(def).map(|scope| f(this, def, scope))
        })
        .flatten()
    }

    fn with_fn_scope<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefId) -> T,
    ) -> Option<T> {
        self.with_fn_def(symbol, |this, def, scope| {
            this.with_scope(scope, |this| f(this, def))
        })
    }

    fn with_struct_def<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefId, ScopeId) -> T,
    ) -> Option<T> {
        self.with_type_def(symbol, |this, def| {
            this.struct_def_scope(def).map(|scope| f(this, def, scope))
        })
        .flatten()
    }

    fn with_mod_def<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefId, ScopeId) -> T,
    ) -> Option<T> {
        self.with_type_def(symbol, |this, def| {
            this.mod_def_scope(def).map(|scope| f(this, def, scope))
        })
        .flatten()
    }

    fn with_mod_scope<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefId) -> T,
    ) -> Option<T> {
        self.with_mod_def(symbol, |this, def, scope| {
            this.with_scope(scope, |this| f(this, def))
        })
    }

    fn with_ty_alias_def<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefId, ScopeId) -> T,
    ) -> Option<T> {
        self.with_type_def(symbol, |this, def| {
            this.ty_alias_def_scope(def)
                .map(|scope| f(this, def, scope))
        })
        .flatten()
    }

    #[allow(unused)]
    fn with_ty_alias_scope<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefId) -> T,
    ) -> Option<T> {
        self.with_ty_alias_def(symbol, |this, def, scope| {
            this.with_scope(scope, |this| f(this, def))
        })
    }

    /// Recursively searches the current scope and enclosing
    /// scopes for a [`Def`] with a given symbol and which
    /// belongs to the specified Namespace.
    fn lookup_up_scope_chain(
        &self,
        scope: ScopeId,
        symbol: Symbol,
        namespace: Namespace,
    ) -> Option<DefId> {
        std::iter::successors(Some(scope), |&scope| self.scopes[scope].parent)
            .find_map(|scope| self.lookup_in_scope(scope, symbol, namespace))
    }

    /// Declares a new [`Def`] in a scope of a certain kind.
    ///
    /// Any ty this kind of def needs must already be embedded
    /// in `kind` by the caller (see e.g. [`PatDeclKind::def_kind`],
    /// which requires a fresh inference variable `?a` be created
    /// up front to represent the type of the def until something
    /// else later constrains it).
    fn declare(&mut self, symbol: Symbol, span: Span, kind: DefKind) -> DefId {
        let namespace = kind.namespace();

        // `let` defs (and, transitively through them, their
        // shadowing sub-patterns) are always allowed to shadow
        // whatever previously held the same symbol in this scope.
        // Everything else -- functions, modules, structs, enums,
        // traits, type aliases, and parameters -- must be uniquely
        // symbold within a scope.
        if !matches!(kind, DefKind::Local(_)) {
            self.check_redeclaration(namespace, symbol, span);
        }

        let def = self.defs.insert(Def {
            symbol,
            kind,
            declared_at: span,
        });
        self.insert_in_scope(symbol, def, namespace);
        self.positions.record_def(span, def);
        def
    }

    /// If a def already exists with the given symbol in the given
    /// Namespace of the current scope, emits an [`AlreadyDefined`]
    /// diagnostic pointing back at its original declaration.
    fn check_redeclaration(&mut self, namespace: Namespace, symbol: Symbol, span: Span) {
        self.lookup_in_scope(self.current_scope, symbol, namespace)
            .into_iter()
            .for_each(|existing| {
                let original = self.def(existing).declared_at;
                let symbol = self.symbols.resolve(symbol).to_owned();
                self.diagnostics
                    .push(AlreadyDefined::new(span, symbol, original));
            });
    }

    /// Declares a new generic parameter within the current
    /// scope. For example, say there is a function
    /// ```ignore
    /// fn identity<T>(x: T) -> T {
    ///     x
    /// }
    /// ```
    /// then when resolving this function, the scope of the
    /// body is entered, and a new [`DefKind::GenericParam`]
    /// is created for `T` inside that scope so `T` becomes
    /// a valid type that can be used within the function body.
    fn declare_generic_param(&mut self, symbol: Symbol, span: Span) -> (DefId, GenericId) {
        let id = self.generic_ids.insert(());
        self.generic_names
            .declare(id, self.symbols.resolve(symbol).to_owned());
        let ty = self.ty(TyKind::Generic(id));
        let def = self.defs.insert(Def {
            symbol,
            kind: DefKind::GenericParam(ty),
            declared_at: span,
        });
        self.insert_type_in_scope(symbol, def);
        self.positions.record_def(span, def);
        (def, id)
    }

    fn declare_generic_params(&mut self, params: &[GenericParam]) -> Vec<GenericId> {
        params
            .iter()
            .map(|param| {
                self.declare_generic_param(param.ident.symbol, param.ident.span)
                    .1
            })
            .collect()
    }

    fn declare_synthetic_generic_param(&mut self, taken: &mut HashSet<String>) -> GenericId {
        let id = self.generic_ids.insert(());
        let name = self.generic_names.fresh_synthetic(taken);
        self.generic_names.declare(id, name);
        id
    }

    /// Inserts a def into the current [`Scope`] via its
    /// handle.
    fn insert_in_scope(&mut self, symbol: Symbol, def: DefId, namespace: Namespace) {
        let scope = &mut self.scopes[self.current_scope];
        match namespace {
            Namespace::Type => scope.types.insert(symbol, def),
            Namespace::Value => scope.values.insert(symbol, def),
        };
    }

    fn insert_type_in_scope(&mut self, symbol: Symbol, def: DefId) {
        self.insert_in_scope(symbol, def, Namespace::Type);
    }

    fn insert_value_in_scope(&mut self, symbol: Symbol, def: DefId) {
        self.insert_in_scope(symbol, def, Namespace::Value);
    }

    /// Convert a [`Ty`] AST node into a ty which represents
    /// that type and which can actually be used by the type
    /// checker to perform checking and inference.
    fn lower_ty(&mut self, ty: &Ty) -> TyId {
        match &ty.kind {
            AstTyKind::Never => self.ty(TyKind::Never),
            AstTyKind::Paren(inner) => self.lower_ty(inner),
            AstTyKind::Array(inner) => self.lower_array_ty(inner),
            AstTyKind::Tup(inner) => self.lower_tup_ty(inner),
            AstTyKind::Fn(fn_ty) => self.lower_fn_ty(fn_ty, ty.span),
            AstTyKind::Path(path) => self.lower_path_ty(path),
            AstTyKind::ImplicitSelf => self.lower_implicit_self_ty(),
            // When `_` is used as a type annotation, it means
            // the type should be inferred. Hence, introduce
            // a fresh inference variable `?a` to represent
            // this type.
            AstTyKind::Infer => self.fresh_var_at(Some(ty.span)),
            AstTyKind::Err => self.ty(TyKind::Err),
        }
    }

    fn lower_array_ty(&mut self, inner: &Ty) -> TyId {
        let elem = self.lower_ty(inner);
        self.ty(TyKind::Array(elem))
    }

    fn lower_tup_ty(&mut self, inner: &[Box<Ty>]) -> TyId {
        let args = inner.iter().map(|x| self.lower_ty(x)).collect();
        self.ty(TyKind::Tuple(args))
    }

    fn lower_fn_ty(&mut self, fn_ty: &FnTy, span: Span) -> TyId {
        let FnTy { inputs, output } = fn_ty;
        let input_args = inputs.iter().map(|x| self.lower_ty(x)).collect();
        let output_ty = self.lower_ret_ty(output, Some(span));
        self.ty(TyKind::Fn(input_args, output_ty))
    }

    fn lower_ret_ty(&mut self, output: &FnRetTy, default_span: Option<Span>) -> TyId {
        match output {
            FnRetTy::Default(_) => self.fresh_var_at(default_span),
            FnRetTy::Ty(ty) => self.lower_ty(ty),
        }
    }

    fn lower_implicit_self_ty(&mut self) -> TyId {
        unimplemented!()
    }

    fn lower_path_ty(&mut self, path: &Path) -> TyId {
        if let [segment] = path.segments.as_slice() {
            let found = PRIMITIVE_TYPES
                .iter()
                .find(|(symbol, _)| self.symbols.get(symbol) == Some(segment.ident.symbol));
            if let Some((symbol, con)) = found {
                self.positions.record_primitive(segment.ident.span, symbol);
                return self.ty(con.clone());
            }
        }

        self.resolve_path_to_type(path)
            .map(|def| {
                self.record_path_reference(path, def);
                match &self.def(def).kind {
                    DefKind::Struct(_) => {
                        let generics = self.def(def).generics().to_vec();
                        let args = self.instantiate_struct_args(&generics, path);
                        self.ty(TyKind::Struct(def, args))
                    }
                    DefKind::Enum(_) => self.ty(TyKind::Enum(def, Vec::new())),
                    _ => self.instantiate_path(def, path),
                }
            })
            .unwrap_or_else(|| {
                self.diagnostics.push(UnresolvedType::new(
                    path.span,
                    display_path(path, &self.symbols),
                ));
                self.ty(TyKind::Err)
            })
    }
}

const PRIMITIVE_TYPES: &[(&str, TyKind)] = &[
    ("bool", TyKind::Bool),
    ("int", TyKind::Int),
    ("float", TyKind::Float),
    ("String", TyKind::Str),
];

/// The AST Visitor that performs the Resolution stage of the
/// type checking. Walks the AST, creating new defs in the
/// def table for each item it finds.
struct Resolver<'a, 'ast> {
    /// Mutable reference to the underlying TypeCheckContext.
    cx: &'a mut TypeCheckContext<'ast>,
}

impl<'ast> Resolver<'_, 'ast> {
    /// Creates a new [`Scope`] in the underling
    /// [`TypeCheckContext`], and returns a handle to it.
    fn new_scope(&mut self) -> ScopeId {
        let parent = self.cx.current_scope;
        self.cx.scopes.insert(Scope {
            parent: Some(parent),
            types: HashMap::new(),
            values: HashMap::new(),
        })
    }

    /// Enters the given scope, performs some function while
    /// inside that scope, and then exits the scope once the
    /// function is complete.
    ///
    /// This ensures that even an early exit will still
    /// ensure that the current_scope is updated back to
    /// the parent scope.
    fn with_scope<T>(&mut self, scope: ScopeId, f: impl FnOnce(&mut Self) -> T) -> T {
        let parent = self.cx.current_scope;
        self.cx.current_scope = scope;
        let result = f(self);
        self.cx.current_scope = parent;
        result
    }
}

impl Visitor for Resolver<'_, '_> {
    fn visit_item(&mut self, item: &Item) {
        match &item.kind {
            ItemKind::Fn(f) => self.resolve_fn_item(item, f),
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
}

impl Resolver<'_, '_> {
    fn resolve_fn_item(&mut self, item: &Item, f: &Fn) {
        let scope = self.new_scope();
        let ty = self.cx.fresh_var_at(Some(f.ident.span));
        let fn_def = self.cx.declare(
            f.ident.symbol,
            f.ident.span,
            DefKind::Fn(FnDef {
                scope,
                param_spans: Vec::new(),
                param_symbols: Vec::new(),
                ty,
                generics: Vec::new(),
            }),
        );

        let mut generics = Vec::new();
        self.with_scope(scope, |this| {
            generics = this.cx.declare_generic_params(&f.generics.params);
            item.walk(this);
        });
        if let DefKind::Fn(fn_data) = &mut self.cx.defs[fn_def].kind {
            fn_data.generics = generics;
        }
    }

    fn resolve_ty_alias_item(&mut self, alias: &TyAlias) {
        let scope = self.new_scope();
        let ty = self.cx.fresh_var_at(Some(alias.ident.span));
        let alias_def = self.cx.declare(
            alias.ident.symbol,
            alias.ident.span,
            DefKind::TyAlias(TyAliasDef {
                scope,
                ty,
                generics: Vec::new(),
            }),
        );

        let mut generics = Vec::new();
        self.with_scope(scope, |this| {
            generics = this.cx.declare_generic_params(&alias.generics.params);
        });
        if let DefKind::TyAlias(alias_data) = &mut self.cx.defs[alias_def].kind {
            alias_data.generics = generics;
        }
    }

    fn resolve_enum_item(&mut self, ident: &Ident, generics: &Generics, def: &AstEnumDef) {
        let scope = self.new_scope();
        let generics = self.with_scope(scope, |this| {
            this.cx.declare_generic_params(&generics.params)
        });
        let variants = def
            .variants
            .iter()
            .map(|v| self.resolve_variant_data(ident.symbol, ident.span, &v.data, generics.clone()))
            .collect::<Vec<_>>();
        let variants = self.with_scope(scope, |this| {
            variants
                .into_iter()
                .map(|v| this.cx.declare(v.name, v.span, DefKind::Variant(v)))
                .collect::<Vec<_>>()
        });
        let def = EnumDef {
            variants,
            generics,
            scope,
        };
        self.cx
            .declare(ident.symbol, ident.span, DefKind::Enum(def));
    }

    fn resolve_variant_data(
        &mut self,
        name: Symbol,
        span: Span,
        data: &VariantData,
        generics: Vec<GenericId>,
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
            },
            VariantData::Struct(fields) => VariantDef {
                name,
                span,
                fields: fields
                    .iter()
                    .map(|field| FieldDef {
                        name: field.ident.clone().unwrap().symbol,
                        ty: self.cx.fresh_var_at(Some(field.span)),
                    })
                    .collect(),
                ctor_ty,
                generics,
            },
        }
    }

    fn resolve_struct_item(&mut self, ident: &Ident, generics: &Generics, data: &VariantData) {
        let scope = self.new_scope();
        let generics = self.with_scope(scope, |this| {
            this.cx.declare_generic_params(&generics.params)
        });
        let variant = self.resolve_variant_data(ident.symbol, ident.span, data, generics);
        let def = self.cx.declare(
            ident.symbol,
            ident.span,
            DefKind::Struct(StructDef { variant, scope }),
        );
        if !matches!(data, VariantData::Struct(_)) {
            let symbol = ident.symbol;
            self.cx
                .check_redeclaration(Namespace::Value, symbol, ident.span);
            self.cx.insert_value_in_scope(symbol, def);
        }
    }

    fn resolve_trait_item(&mut self, t: &Trait) {
        self.cx
            .declare(t.ident.symbol, t.ident.span, DefKind::Trait);
    }

    fn resolve_mod_unloaded_item(&mut self, ident: &Ident) {
        let scope = self.new_scope();
        self.cx
            .declare(ident.symbol, ident.span, DefKind::Mod(scope));
    }

    fn resolve_mod_loaded_item(&mut self, ident: &Ident, item: &Item) {
        let scope = self.new_scope();
        self.cx
            .declare(ident.symbol, ident.span, DefKind::Mod(scope));
        self.with_scope(scope, |this| item.walk(this));
    }

    fn resolve_use_item(&mut self) {}

    fn resolve_impl_item(&mut self) {}
}

/// Performs the signature lowering stage of the type checking.
/// Fills in the types of the defs created by the [`Resolver`]
/// where possible, for example for functions and type aliases.
struct SignatureLowerer<'a, 'ast> {
    /// A mutable reference to the underlying TypeCheckContext.
    cx: &'a mut TypeCheckContext<'ast>,
}

impl SignatureLowerer<'_, '_> {
    /// Enters the given scope, performs some function while
    /// inside that scope, and then exits the scope once the
    /// function is complete.
    ///
    /// This ensures that even an early exit will still
    /// ensure that the current_scope is updated back to
    /// the parent scope.
    fn with_scope(&mut self, scope: ScopeId, f: impl FnOnce(&mut Self)) {
        let parent = self.cx.current_scope;
        self.cx.current_scope = scope;
        f(self);
        self.cx.current_scope = parent;
    }

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
    fn lower_fn_item(&mut self, item: &Item, f: &Fn) {
        let symbol = f.ident.symbol;
        let Some((def, scope)) = self.cx.with_fn_def(symbol, |_, def, scope| (def, scope)) else {
            return;
        };

        self.with_scope(scope, |this| {
            let fn_ty = this.lower_fn_sig(f);
            let def_ty = this.cx.def(def).ty();
            // Unifies the fresh placeholder inference variable which
            // was created during the previous Resolution stage with the
            // ty created by lowering the function signature.
            let _ = this.cx.inf.unify(def_ty, fn_ty);

            // Collect information about parameter symbols and spans.
            let param_symbols: Vec<String> = f
                .sig
                .inputs
                .iter()
                .map(|p| types::pat_display_name(&p.pat, &this.cx.symbols))
                .collect();
            if let DefKind::Fn(fn_data) = &mut this.cx.defs[def].kind {
                fn_data.param_spans = f
                    .sig
                    .inputs
                    .iter()
                    .map(|p| p.ty.as_ref().map(|ty| ty.span))
                    .collect();
                fn_data.param_symbols = param_symbols;
            }
            item.walk(this);
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
        let namespace = self.cx.defs[sid].kind.namespace();
        self.cx.insert_in_scope(symbol, sid, namespace);
    }

    fn lower_use_tree_glob(&mut self, tree: &UseTree, sid: DefId, span: Span) {
        let Some(scope) = self.cx.mod_def_scope(sid) else {
            self.cx.diagnostics.push(InvalidGlobTarget::new(
                span,
                display_path(&tree.prefix, &self.cx.symbols),
                self.cx.defs[sid].kind.describe().to_string(),
            ));
            return;
        };
        self.cx.scopes[scope]
            .types
            .clone()
            .into_iter()
            .for_each(|(symbol, sid)| self.cx.insert_type_in_scope(symbol, sid));
        self.cx.scopes[scope]
            .values
            .clone()
            .into_iter()
            .for_each(|(symbol, sid)| self.cx.insert_value_in_scope(symbol, sid));
    }

    fn lower_use_tree_nested(&mut self, items: &[UseTree], sid: DefId) {
        items
            .iter()
            .for_each(|item| self.lower_use_tree(item, Some(sid)));
    }

    fn lower_ty_alias_item(&mut self, alias: &TyAlias) {
        let symbol = alias.ident.symbol;
        let resolved = self
            .cx
            .with_ty_alias_def(symbol, |_, def, scope| (def, scope));

        if let (Some((def, scope)), Some(ty)) = (resolved, alias.ty.as_ref()) {
            self.with_scope(scope, |this| {
                let aliased = this.cx.lower_ty(ty);
                let def_ty = this.cx.def(def).ty();
                // Unifies the fresh placeholder inference variable which
                // was created during the previous Resolution stage with the
                // ty created by lowering the type of the expression being
                // aliased. A type alias can never refer to itself, directly
                // or indirectly (e.g. `type Foo = (Foo, int);`), since that
                // would make it an infinitely-sized type.
                this.cx.unify_or_report_cycle(def_ty, aliased, ty.span);
            });
        }
    }

    fn lower_variant_data_field_tys(&mut self, data: &VariantData) -> (Vec<TyId>, Vec<GenericId>) {
        let fields = match data {
            VariantData::Unit => return (vec![], vec![]),
            VariantData::Tuple(fields) | VariantData::Struct(fields) => fields,
        };
        let mut taken = self.cx.generic_names.all_names();
        let mut synthesized = Vec::new();
        let tys = fields
            .iter()
            .map(|field| {
                field
                    .ty
                    .as_ref()
                    .map(|ty| self.cx.lower_ty(ty))
                    .unwrap_or_else(|| {
                        let id = self.cx.declare_synthetic_generic_param(&mut taken);
                        synthesized.push(id);
                        self.cx.ty(TyKind::Generic(id))
                    })
            })
            .collect();
        (tys, synthesized)
    }

    fn unify_variant_field_tys(&mut self, def: DefId, lowered: &[TyId]) {
        let (DefKind::Struct(StructDef { variant, .. }) | DefKind::Variant(variant)) =
            &self.cx.defs[def].kind
        else {
            return;
        };
        let field_tys: Vec<TyId> = variant.fields.iter().map(|field| field.ty).collect();
        for (field_ty, lowered_ty) in field_tys.into_iter().zip(lowered) {
            let _ = self.cx.inf.unify(field_ty, *lowered_ty);
        }
    }

    fn unify_ctor_ty(&mut self, def: DefId, self_ty: TyKind, data: &VariantData, lowered: &[TyId]) {
        let placeholder = match &self.cx.defs[def].kind {
            DefKind::Struct(StructDef { variant, .. }) | DefKind::Variant(variant) => {
                variant.ctor_ty
            }
            _ => None,
        };
        let Some(placeholder) = placeholder else {
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
        let Some((def, scope)) = self
            .cx
            .with_struct_def(ident.symbol, |_, def, scope| (def, scope))
        else {
            return;
        };
        self.with_scope(scope, |this| {
            let (lowered, synthesized) = this.lower_variant_data_field_tys(data);
            if let DefKind::Struct(StructDef { variant, .. }) = &mut this.cx.defs[def].kind {
                variant.generics.extend(synthesized);
            }
            this.unify_variant_field_tys(def, &lowered);
            let generics = this.cx.def(def).generics().to_vec();
            let placeholder_args = generics
                .iter()
                .map(|&id| this.cx.ty(TyKind::Generic(id)))
                .collect();
            this.unify_ctor_ty(def, TyKind::Struct(def, placeholder_args), data, &lowered);
        });
    }

    fn lower_enum_item(&mut self, ident: &Ident, def: &AstEnumDef) {
        let lowered: Vec<Vec<TyId>> = def
            .variants
            .iter()
            .map(|v| self.lower_variant_data_field_tys(&v.data).0)
            .collect();

        let Some(enum_def) = self.cx.with_type_def(ident.symbol, |_, def| def) else {
            return;
        };
        let variants = match &self.cx.def(enum_def).kind {
            DefKind::Enum(enum_data) => enum_data.variants.clone(),
            _ => return,
        };
        for ((variant, lowered_fields), ast_variant) in
            variants.into_iter().zip(lowered).zip(&def.variants)
        {
            self.unify_variant_field_tys(variant, &lowered_fields);
            self.unify_ctor_ty(
                variant,
                TyKind::Enum(enum_def, Vec::new()),
                &ast_variant.data,
                &lowered_fields,
            );
        }
    }

    fn lower_mod_item(&mut self, symbol: &Ident, _kind: &ModKind, item: &Item) {
        let symbol = symbol.symbol;
        let scope = self.cx.with_mod_def(symbol, |_, _, scope| scope);
        scope
            .into_iter()
            .for_each(|scope| self.with_scope(scope, |this| item.walk(this)));
    }
}

fn display_path(path: &Path, symbols: &Interner) -> String {
    path.segments
        .iter()
        .map(|segment| symbols.resolve(segment.ident.symbol))
        .collect::<Vec<_>>()
        .join("::")
}

impl Visitor for SignatureLowerer<'_, '_> {
    fn visit_item(&mut self, item: &Item) {
        match &item.kind {
            ItemKind::Fn(f) => self.lower_fn_item(item, f),
            ItemKind::TyAlias(alias) => self.lower_ty_alias_item(alias),
            ItemKind::Struct(ident, _generics, data) => self.lower_struct_item(ident, data),
            ItemKind::Enum(ident, _generics, def) => self.lower_enum_item(ident, def),
            ItemKind::Mod(ident, kind) => self.lower_mod_item(ident, kind, item),
            ItemKind::Use(tree) => self.lower_use_tree(tree, None),
            _ => {}
        }
    }
}

fn segment_namespace(i: usize, last: usize, namespace: Namespace) -> Namespace {
    if i == last {
        namespace
    } else {
        Namespace::Type
    }
}

#[cfg(test)]
mod tests;
