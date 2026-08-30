//! Contains the type checking stage of the interpreter.
//!
//! The type checking stage depends on the AST that was constructed
//! by the `parser` during the previous stage. Given some AST, the
//! type checker does several passes of the AST.
//!
//! ```text
//!     1.
//! ```

use std::collections::HashMap;

use ast::visit::Visitor;
use ast::{
    FnRetTy, FnTy, GenericParam, Item, ItemKind, Path, SELF_TYPE, Span, Ty, TyKind as AstTyKind,
};
use intern::{Interner, Symbol};

use crate::inference::{InferenceTable, TyId, VarId};
use crate::types::TyKind;

mod adt;
mod call_graph;
mod check;
mod checked_program;
mod defs;
mod errors;
mod generics;
mod inference;
mod lower_signatures;
mod polymorphism;
mod position_index;
mod recursion;
mod resolve;
mod scope;
mod types;

use check::collect_fn_mod_items;
use defs::{
    Def, DefIdOf, DefKind, Defs, EnumDef, FnDef, IntoDefKind, ModDef, StructDef, TraitDef,
    TyAliasDef, VariantDef,
};
use errors::Diagnostics;
use generics::{GenericId, GenericRegistry};
use lower_signatures::SignatureLowerer;
use position_index::PositionIndex;
use recursion::RecursionTracker;
use resolve::Resolver;
use scope::{Namespace, ScopeId, ScopeTree};

pub use checked_program::CheckedProgram;
pub use diagnostics::{Diagnostic, Level};
pub use errors::Locale;

use crate::errors::{AlreadyDefined, AnnotationsNeeded, SelfOutsideImplOrTrait, UnresolvedType};

slotmap::new_key_type! {
    /// A handle to a def stored in the def arena within
    /// the [`TypeCheckContext`]
    pub struct DefId;
}

#[derive(Debug, Clone)]
struct ImplInfo {
    scope: ScopeId,
    of_trait: Option<DefIdOf<TraitDef>>,
    generics: Vec<GenericId>,
    trait_args: Vec<TyId>,
    self_ty: TyId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ImplTarget {
    Adt(DefId),
    Int,
    Float,
    Bool,
    Str,
    Array,
    Tuple(usize),
}

fn impl_target_of(kind: &TyKind) -> Option<ImplTarget> {
    match kind {
        TyKind::Struct(def, _) => Some(ImplTarget::Adt(*def)),
        TyKind::Enum(def, _) => Some(ImplTarget::Adt(def.id())),
        TyKind::Int => Some(ImplTarget::Int),
        TyKind::Float => Some(ImplTarget::Float),
        TyKind::Bool => Some(ImplTarget::Bool),
        TyKind::Str => Some(ImplTarget::Str),
        TyKind::Array(_) => Some(ImplTarget::Array),
        TyKind::Tuple(elems) => Some(ImplTarget::Tuple(elems.len())),
        _ => None,
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

    /// The def table. See [`Defs`].
    defs: Defs,

    /// Contains all scopes within the program, linked into a
    /// tree by parent pointers. See [`ScopeTree`].
    scopes: ScopeTree,

    /// The registry of every generic type parameter in the
    /// program. See [`GenericRegistry`].
    generics: GenericRegistry,

    /// A handle to the scope that is currently being checked.
    current_scope: ScopeId,

    /// All the diagnostics that have been accumulated so far.
    diagnostics: Diagnostics,

    /// An index which allows you to query for defs and types
    /// and other things, based on spans within the source code.
    positions: PositionIndex,

    /// Tracks which function is currently being checked, the
    /// call graph, and SCC discovery. See [`RecursionTracker`].
    recursion: RecursionTracker,

    items_by_def: HashMap<DefId, &'ast Item>,

    impls_by_target: HashMap<ImplTarget, Vec<ImplInfo>>,

    blanket_impls: Vec<ImplInfo>,

    inference_vars: Vec<(VarId, Span)>,

    return_tys: Vec<TyId>,

    self_tys: Vec<TyId>,
}

impl<'ast> TypeCheckContext<'ast> {
    /// Create a new blank [`TypeCheckContext`].
    fn new(symbols: Interner) -> Self {
        let (scopes, root) = ScopeTree::new();

        Self {
            inf: InferenceTable::new(),

            symbols,
            scopes,
            generics: GenericRegistry::new(),
            inference_vars: Vec::new(),
            defs: Defs::new(),
            current_scope: root,
            diagnostics: Diagnostics::default(),
            positions: PositionIndex::default(),
            recursion: RecursionTracker::new(),
            items_by_def: HashMap::new(),
            impls_by_target: HashMap::new(),
            blanket_impls: Vec::new(),
            return_tys: Vec::new(),
            self_tys: Vec::new(),
        }
    }

    fn def(&self, def: DefId) -> &Def {
        self.defs.get(def)
    }

    /// Updates the [`PositionIndex`] to include a newly found path.
    fn record_path_reference(&mut self, path: &Path, def: DefId) {
        if let Some(segment) = path.segments.last() {
            self.positions.record_def(segment.ident.span, def);
        }
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

    fn with_return_ty<T>(&mut self, ty: TyId, f: impl FnOnce(&mut Self) -> T) -> T {
        self.return_tys.push(ty);
        let result = f(self);
        self.return_tys.pop();
        result
    }

    fn current_return_ty(&self) -> Option<TyId> {
        self.return_tys.last().copied()
    }

    fn current_self_ty(&self) -> Option<TyId> {
        self.self_tys.last().copied()
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
            ItemKind::Trait(trt) => self.check_trait(trt),
            ItemKind::Impl(imp) => self.check_impl(imp),
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

        let (start, first_def) = self
            .resolve_primitive_prefix(path, last, namespace)
            .or_else(|| {
                let (i, first) = path.segments.iter().enumerate().next()?;
                let def = self.scopes.lookup_up_chain(
                    self.current_scope,
                    first.ident.symbol,
                    segment_namespace(i, last, namespace),
                )?;
                Some((1, def))
            })?;

        path.segments[start..]
            .iter()
            .enumerate()
            .map(|(j, segment)| (start + j, segment))
            .try_fold(first_def, |def, (i, segment)| {
                let symbol = segment.ident.symbol;
                let ns = segment_namespace(i, last, namespace);
                self.resolve_path_segment(def, symbol, ns)
            })
    }

    fn resolve_path_segment(
        &mut self,
        def: DefId,
        symbol: Symbol,
        namespace: Namespace,
    ) -> Option<DefId> {
        if let Some(found) = self.resolve_in_def_scope(def, symbol, namespace) {
            return Some(found);
        }

        let target = self.impl_target_of_def(def)?;
        self.resolve_in_impls_for_target(target, symbol, namespace)
    }

    fn resolve_in_def_scope(
        &self,
        def: DefId,
        symbol: Symbol,
        namespace: Namespace,
    ) -> Option<DefId> {
        let scope = self
            .mod_def_scope(def)
            .map(|(_, scope)| scope)
            .or_else(|| self.enum_def_scope(def).map(|(_, scope)| scope))?;
        self.scopes.lookup(scope, symbol, namespace)
    }

    fn impl_target_of_def(&mut self, def: DefId) -> Option<ImplTarget> {
        if matches!(self.def(def).kind, DefKind::Struct(_) | DefKind::Enum(_)) {
            return Some(ImplTarget::Adt(def));
        }
        let kind = self.resolved_alias_kind(def)?;
        impl_target_of(&kind)
    }

    fn unwrap_to_adt_def(&mut self, def: DefId) -> DefId {
        match self.resolved_alias_kind(def) {
            Some(TyKind::Struct(inner, _)) => inner,
            Some(TyKind::Enum(inner, _)) => inner.id(),
            _ => def,
        }
    }

    fn resolved_alias_kind(&mut self, def: DefId) -> Option<TyKind> {
        let DefKind::TyAlias(alias) = &self.def(def).kind else {
            return None;
        };
        let resolved = self.inf.resolve(alias.ty);
        self.inf.ty(resolved).cloned()
    }

    fn resolve_primitive_prefix(
        &mut self,
        path: &Path,
        last: usize,
        namespace: Namespace,
    ) -> Option<(usize, DefId)> {
        let first = path.segments.first()?;
        let (_, kind) = PRIMITIVE_TYPES
            .iter()
            .find(|(name, _)| self.symbols.get(name) == Some(first.ident.symbol))?;
        let target = impl_target_of(kind)?;

        let second = path.segments.get(1)?;
        let symbol = second.ident.symbol;
        let ns = segment_namespace(1, last, namespace);

        let def = self.resolve_in_impls_for_target(target, symbol, ns)?;
        Some((2, def))
    }

    fn resolve_in_impls_for_target(
        &self,
        target: ImplTarget,
        symbol: Symbol,
        namespace: Namespace,
    ) -> Option<DefId> {
        let mut matches = self
            .impls_by_target
            .get(&target)
            .into_iter()
            .flatten()
            .chain(self.blanket_impls.iter())
            .filter_map(|imp| self.scopes.lookup(imp.scope, symbol, namespace));

        let found = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Some(found)
    }

    fn resolve_path_to_type(&mut self, path: &Path) -> Option<DefId> {
        self.resolve_path(path, Namespace::Type)
    }

    fn resolve_path_to_value(&mut self, path: &Path) -> Option<DefId> {
        self.resolve_path(path, Namespace::Value)
    }

    fn resolve_path_to_struct(&mut self, path: &Path) -> Option<(DefId, &VariantDef)> {
        let id = self.resolve_path_to_type(path)?;
        let id = self.unwrap_to_adt_def(id);
        self.def(id).kind.as_struct().map(|variant| (id, variant))
    }

    fn resolve_path_to_enum(&mut self, path: &Path) -> Option<DefIdOf<EnumDef>> {
        let id = self.resolve_path_to_type(path)?;
        let id = self.unwrap_to_adt_def(id);
        self.enum_def_scope(id).map(|(def, _)| def)
    }

    fn resolve_path_to_trait(&mut self, path: &Path) -> Option<DefIdOf<TraitDef>> {
        let id = self.resolve_path_to_type(path)?;
        self.trait_def_scope(id).map(|(def, _)| def)
    }

    fn resolve_path_to_variant(&mut self, path: &Path) -> Option<(DefId, &VariantDef)> {
        let id = self.resolve_path_to_value(path)?;
        self.def(id)
            .kind
            .as_variant()
            .filter(|variant| variant.ctor_ty.is_none())
            .map(|variant| (id, variant))
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
                let (_, scope) = self.mod_def_scope(def)?;
                let symbol = segment.ident.symbol;
                self.scopes
                    .lookup(scope, symbol, segment_namespace(i, last, namespace))
            })
    }

    fn with_def_in_scope<T>(
        &mut self,
        symbol: Symbol,
        namespace: Namespace,
        f: impl FnOnce(&mut Self, DefId) -> T,
    ) -> Option<T> {
        self.scopes
            .lookup(self.current_scope, symbol, namespace)
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

    fn fn_def_scope(&self, def: DefId) -> Option<(DefIdOf<FnDef>, ScopeId)> {
        let DefKind::Fn(fn_data) = &self.def(def).kind else {
            return None;
        };
        Some((DefIdOf::new_unchecked(def), fn_data.scope))
    }

    fn struct_def_scope(&self, def: DefId) -> Option<(DefIdOf<StructDef>, ScopeId)> {
        let DefKind::Struct(struct_data) = &self.def(def).kind else {
            return None;
        };
        Some((DefIdOf::new_unchecked(def), struct_data.scope))
    }

    fn mod_def_scope(&self, def: DefId) -> Option<(DefIdOf<ModDef>, ScopeId)> {
        let DefKind::Mod(mod_data) = &self.def(def).kind else {
            return None;
        };
        Some((DefIdOf::new_unchecked(def), mod_data.scope))
    }

    fn trait_def_scope(&self, def: DefId) -> Option<(DefIdOf<TraitDef>, ScopeId)> {
        let DefKind::Trait(trait_data) = &self.def(def).kind else {
            return None;
        };
        Some((DefIdOf::new_unchecked(def), trait_data.scope))
    }

    fn enum_def_scope(&self, def: DefId) -> Option<(DefIdOf<EnumDef>, ScopeId)> {
        let DefKind::Enum(EnumDef { scope, .. }) = self.def(def).kind else {
            return None;
        };
        Some((DefIdOf::new_unchecked(def), scope))
    }

    fn ty_alias_def_scope(&self, def: DefId) -> Option<(DefIdOf<TyAliasDef>, ScopeId)> {
        let DefKind::TyAlias(alias_data) = &self.def(def).kind else {
            return None;
        };
        Some((DefIdOf::new_unchecked(def), alias_data.scope))
    }

    fn register_impl_for(
        &mut self,
        target: ImplTarget,
        scope: ScopeId,
        of_trait: Option<DefIdOf<TraitDef>>,
        generics: Vec<GenericId>,
        trait_args: Vec<TyId>,
        self_ty: TyId,
    ) {
        self.impls_by_target
            .entry(target)
            .or_default()
            .push(ImplInfo {
                scope,
                of_trait,
                generics,
                trait_args,
                self_ty,
            });
    }

    fn register_blanket_impl(
        &mut self,
        scope: ScopeId,
        of_trait: Option<DefIdOf<TraitDef>>,
        generics: Vec<GenericId>,
        trait_args: Vec<TyId>,
        self_ty: TyId,
    ) {
        self.blanket_impls.push(ImplInfo {
            scope,
            of_trait,
            generics,
            trait_args,
            self_ty,
        });
    }

    /// Whether `target_ty` has an impl of the trait `trait_def`.
    fn target_implements(
        &mut self,
        target_ty: TyId,
        trait_def: DefIdOf<TraitDef>,
        trait_args: &[TyId],
    ) -> bool {
        let resolved = self.inf.resolve(target_ty);
        let target = self
            .inf
            .ty(resolved)
            .cloned()
            .and_then(|kind| impl_target_of(&kind));

        let bucketed = target
            .and_then(|target| self.impls_by_target.get(&target))
            .into_iter()
            .flatten();

        let candidates: Vec<ImplInfo> = bucketed
            .chain(self.blanket_impls.iter())
            .filter(|imp| imp.of_trait == Some(trait_def))
            .cloned()
            .collect();

        candidates
            .iter()
            .any(|imp| self.impl_matches(imp, target_ty, trait_args))
    }

    fn impl_matches(&mut self, imp: &ImplInfo, target_ty: TyId, trait_args: &[TyId]) -> bool {
        self.speculatively(|this| {
            let mut subst: HashMap<GenericId, TyId> = imp
                .generics
                .iter()
                .map(|&id| (id, this.fresh_var()))
                .collect();

            let instantiated_self = this.instantiate_ty(imp.self_ty, &mut subst);
            if this.inf.unify(instantiated_self, target_ty).is_err() {
                return false;
            }

            imp.trait_args.len() == trait_args.len()
                && imp
                    .trait_args
                    .iter()
                    .zip(trait_args)
                    .all(|(&impl_arg, &query_arg)| {
                        let instantiated = this.instantiate_ty(impl_arg, &mut subst);
                        this.inf.unify(instantiated, query_arg).is_ok()
                    })
        })
    }

    fn speculatively(&mut self, f: impl FnOnce(&mut Self) -> bool) -> bool {
        let snapshot = self.inf.snapshot();
        let succeeded = f(self);
        if !succeeded {
            self.inf.rollback_to(snapshot);
        }
        succeeded
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

    /// Declares a new [`Def`] exactly like [`Self::declare`], but
    /// returns a [`DefIdOf`] handle carrying the def's kind, determined
    /// statically by the type of `data` (see [`IntoDefKind`]).
    ///
    /// Prefer this over [`Self::declare`] whenever the payload is one
    /// of the kinds with its own [`Defs`] accessor (`FnDef`,
    /// `StructDef`, `EnumDef`, `TyAliasDef`, `VariantDef`), so that
    /// accessor can be called without a runtime kind check.
    fn declare_typed<T: IntoDefKind>(&mut self, symbol: Symbol, span: Span, data: T) -> DefIdOf<T> {
        let id = self.declare(symbol, span, data.into_def_kind());
        DefIdOf::new_unchecked(id)
    }

    /// If a def already exists with the given symbol in the given
    /// Namespace of the current scope, emits an [`AlreadyDefined`]
    /// diagnostic pointing back at its original declaration.
    fn check_redeclaration(&mut self, namespace: Namespace, symbol: Symbol, span: Span) {
        self.scopes
            .lookup(self.current_scope, symbol, namespace)
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
        let id = self
            .generics
            .declare_new(self.symbols.resolve(symbol).to_owned());
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

    fn declare_self_ty_alias(&mut self, self_ty: TyId, span: Span) -> DefId {
        let symbol = self.symbols.intern(SELF_TYPE);
        self.declare_typed(
            symbol,
            span,
            TyAliasDef {
                scope: self.current_scope,
                ty: self_ty,
                generics: Vec::new(),
            },
        )
        .id()
    }

    /// Inserts a def into the current scope.
    fn insert_in_scope(&mut self, symbol: Symbol, def: DefId, namespace: Namespace) {
        self.scopes
            .insert(self.current_scope, symbol, def, namespace);
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
            AstTyKind::ImplicitSelf => self.lower_implicit_self_ty(ty.span),
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

    fn lower_implicit_self_ty(&mut self, span: Span) -> TyId {
        self.current_self_ty().unwrap_or_else(|| {
            self.diagnostics.push(SelfOutsideImplOrTrait::new(span));
            self.ty(TyKind::Err)
        })
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
                        let args = self.instantiate_adt_args(&generics, path);
                        self.ty(TyKind::Struct(def, args))
                    }
                    DefKind::Enum(_) => {
                        let generics = self.def(def).generics().to_vec();
                        let args = self.instantiate_adt_args(&generics, path);
                        self.ty(TyKind::Enum(DefIdOf::new_unchecked(def), args))
                    }
                    DefKind::Trait(_) => {
                        let generics = self.def(def).generics().to_vec();
                        let args = self.instantiate_adt_args(&generics, path);
                        self.ty(TyKind::TraitObject(DefIdOf::new_unchecked(def), args))
                    }
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

pub(crate) trait CxExt<'ast> {
    fn cx(&mut self) -> &mut TypeCheckContext<'ast>;

    /// Enters the given scope, performs some function while
    /// inside that scope, and then exits the scope once the
    /// function is complete.
    ///
    /// This ensures that even an early exit will still
    /// ensure that the current_scope is updated back to
    /// the parent scope.
    fn with_scope<T>(&mut self, scope: ScopeId, f: impl FnOnce(&mut Self) -> T) -> T {
        let parent = self.cx().current_scope;
        self.cx().current_scope = scope;
        let result = f(self);
        self.cx().current_scope = parent;
        result
    }

    fn with_self_ty<T>(&mut self, self_ty: TyId, f: impl FnOnce(&mut Self) -> T) -> T {
        self.cx().self_tys.push(self_ty);
        let result = f(self);
        self.cx().self_tys.pop();
        result
    }

    fn with_fn_def<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefIdOf<FnDef>, ScopeId) -> T,
    ) -> Option<T> {
        let (def, scope) = self
            .cx()
            .with_value_def(symbol, |cx, def| cx.fn_def_scope(def))
            .flatten()?;
        Some(f(self, def, scope))
    }

    fn with_fn_scope<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefIdOf<FnDef>) -> T,
    ) -> Option<T> {
        self.with_fn_def(symbol, |this, def, scope| {
            this.with_scope(scope, |this| f(this, def))
        })
    }

    fn with_struct_def<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefIdOf<StructDef>, ScopeId) -> T,
    ) -> Option<T> {
        let (def, scope) = self
            .cx()
            .with_type_def(symbol, |cx, def| cx.struct_def_scope(def))
            .flatten()?;
        Some(f(self, def, scope))
    }

    fn with_struct_scope<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefIdOf<StructDef>) -> T,
    ) -> Option<T> {
        self.with_struct_def(symbol, |this, def, scope| {
            this.with_scope(scope, |this| f(this, def))
        })
    }

    fn with_enum_def<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefIdOf<EnumDef>, ScopeId) -> T,
    ) -> Option<T> {
        let (def, scope) = self
            .cx()
            .with_type_def(symbol, |cx, def| cx.enum_def_scope(def))
            .flatten()?;
        Some(f(self, def, scope))
    }

    fn with_enum_scope<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefIdOf<EnumDef>) -> T,
    ) -> Option<T> {
        self.with_enum_def(symbol, |this, def, scope| {
            this.with_scope(scope, |this| f(this, def))
        })
    }

    fn with_mod_def<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefIdOf<ModDef>, ScopeId) -> T,
    ) -> Option<T> {
        let (def, scope) = self
            .cx()
            .with_type_def(symbol, |cx, def| cx.mod_def_scope(def))
            .flatten()?;
        Some(f(self, def, scope))
    }

    fn with_mod_scope<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefIdOf<ModDef>) -> T,
    ) -> Option<T> {
        self.with_mod_def(symbol, |this, def, scope| {
            this.with_scope(scope, |this| f(this, def))
        })
    }

    fn with_trait_def<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefIdOf<TraitDef>, ScopeId) -> T,
    ) -> Option<T> {
        let (def, scope) = self
            .cx()
            .with_type_def(symbol, |cx, def| cx.trait_def_scope(def))
            .flatten()?;
        Some(f(self, def, scope))
    }

    fn with_trait_scope<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefIdOf<TraitDef>) -> T,
    ) -> Option<T> {
        self.with_trait_def(symbol, |this, def, scope| {
            this.with_scope(scope, |this| f(this, def))
        })
    }

    fn with_ty_alias_def<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefIdOf<TyAliasDef>, ScopeId) -> T,
    ) -> Option<T> {
        let (def, scope) = self
            .cx()
            .with_type_def(symbol, |cx, def| cx.ty_alias_def_scope(def))
            .flatten()?;
        Some(f(self, def, scope))
    }

    fn with_ty_alias_scope<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefIdOf<TyAliasDef>) -> T,
    ) -> Option<T> {
        self.with_ty_alias_def(symbol, |this, def, scope| {
            this.with_scope(scope, |this| f(this, def))
        })
    }
}

impl<'ast> CxExt<'ast> for TypeCheckContext<'ast> {
    fn cx(&mut self) -> &mut TypeCheckContext<'ast> {
        self
    }
}

const PRIMITIVE_TYPES: &[(&str, TyKind)] = &[
    ("bool", TyKind::Bool),
    ("int", TyKind::Int),
    ("float", TyKind::Float),
    ("String", TyKind::Str),
];

fn display_path(path: &Path, symbols: &Interner) -> String {
    path.segments
        .iter()
        .map(|segment| symbols.resolve(segment.ident.symbol))
        .collect::<Vec<_>>()
        .join("::")
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
